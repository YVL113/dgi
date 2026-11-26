use crate::texture::{decode_ktx2, is_supported_ktx2};
use crate::{
    audio,
    cache::{BINARY_ANIMATION_MIME, MAX_ASSET_SIZE, validate_binary_animation},
};
use flate2::read::MultiGzDecoder;
use reqwest::{
    Client, StatusCode, Url,
    header::{CONTENT_TYPE, HeaderValue, LOCATION},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Cursor, Read},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

const ROBLOX_API: &str = "apis.roblox.com";
const MAX_REDIRECTS: usize = 5;
const MAX_LOCATION_RESPONSE: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetLocation {
    location: Option<String>,
    #[serde(default)]
    is_archived: bool,
}

#[derive(Clone, Serialize)]
struct Progress<'a> {
    job: u32,
    message: &'a str,
}

pub fn progress(app: &AppHandle, job: u32, message: &str) {
    let _ = app.emit("asset-progress", Progress { job, message });
}

pub fn client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
}

pub fn key_header(key: &str) -> Result<HeaderValue, String> {
    if key.len() > 1024 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("Enter a valid Roblox Open Cloud API key without spaces.".into());
    }
    let mut header =
        HeaderValue::from_str(key).map_err(|_| "The API key contains invalid characters.")?;
    header.set_sensitive(true);
    Ok(header)
}

pub async fn download(
    client: &Client,
    key: &str,
    asset_id: &str,
    app: &AppHandle,
    job: u32,
) -> Result<Vec<u8>, String> {
    let asset_id = asset_id
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or("Enter a positive numeric Roblox asset ID.")?;
    let url = Url::parse(&format!(
        "https://{ROBLOX_API}/asset-delivery-api/v1/assetId/{asset_id}"
    ))
    .map_err(|_| "Could not create the Asset Delivery request.")?;
    let key = key_header(key)?;
    progress(app, job, "Requesting asset from Roblox…");
    let mut response = follow_redirects(client, url, &key).await?;
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN =>
            return Err("Roblox denied access. Check the API key's Asset Delivery permissions and access to this asset.".into()),
        StatusCode::NOT_FOUND => return Err("Roblox could not find this asset ID.".into()),
        StatusCode::TOO_MANY_REQUESTS => return Err("Roblox is rate limiting downloads. Try again shortly.".into()),
        status if !status.is_success() => return Err(format!("Roblox Asset Delivery returned HTTP {}.", status.as_u16())),
        _ => {}
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ASSET_SIZE as u64)
    {
        return Err("The Roblox download is larger than 32 MiB.".into());
    }
    progress(app, job, "Downloading asset…");
    let total = response.content_length();
    let mut last_update = Instant::now();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The download was interrupted. Try again.")?
    {
        if chunk.len() > MAX_ASSET_SIZE - bytes.len() {
            return Err("The Roblox download is larger than 32 MiB.".into());
        }
        bytes.extend_from_slice(&chunk);
        if last_update.elapsed() >= Duration::from_millis(200) {
            let received = bytes.len() as f64 / (1024.0 * 1024.0);
            let message = match total {
                Some(total) => format!(
                    "Downloading asset… {received:.1} / {:.1} MiB",
                    total as f64 / (1024.0 * 1024.0)
                ),
                None => format!("Downloading asset… {received:.1} MiB received"),
            };
            progress(app, job, &message);
            last_update = Instant::now();
        }
    }
    if bytes.is_empty() {
        return Err("Roblox returned an empty download.".into());
    }
    Ok(bytes)
}

async fn follow_redirects(
    client: &Client,
    mut url: Url,
    key: &HeaderValue,
) -> Result<reqwest::Response, String> {
    for _ in 0..=MAX_REDIRECTS {
        let mut request = client.get(url.clone());
        if url.host_str() == Some(ROBLOX_API) && url.path().starts_with("/asset-delivery-api/") {
            request = request.header("x-api-key", key.clone());
        }
        let mut response = request.send().await.map_err(|err| {
            if err.is_timeout() {
                "Roblox took too long to respond. Try again."
            } else {
                "Could not reach Roblox Asset Delivery. Check your connection and try again."
            }
        })?;
        let is_location = response.status().is_success()
            && url.host_str() == Some(ROBLOX_API)
            && url.path().starts_with("/asset-delivery-api/")
            && response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    value
                        .split(';')
                        .next()
                        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
                });
        if is_location {
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "Roblox’s download location response was interrupted.")?
            {
                if chunk.len() > MAX_LOCATION_RESPONSE - bytes.len() {
                    return Err("Roblox returned an oversized download location response.".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            url = asset_location(&url, &bytes)?;
            continue;
        }
        if !response.status().is_redirection() {
            return Ok(response);
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or("Roblox returned an invalid download redirect.")?;
        url = redirect_target(&url, location)?;
    }
    Err("Roblox returned too many download redirects.".into())
}

fn asset_location(from: &Url, bytes: &[u8]) -> Result<Url, String> {
    let result: AssetLocation = serde_json::from_slice(bytes)
        .map_err(|_| "Roblox returned an invalid download location response.")?;
    if result.is_archived {
        return Err("Roblox marked this asset as archived.".into());
    }
    let location = result
        .location
        .filter(|value| !value.trim().is_empty())
        .ok_or("Roblox did not return a download location for this asset.")?;
    redirect_target(from, &location)
}

fn redirect_target(from: &Url, location: &str) -> Result<Url, String> {
    let url = from
        .join(location)
        .map_err(|_| "Roblox returned an invalid download URL.")?;
    let trusted = url.host_str().is_some_and(|host| {
        matches!(host, ROBLOX_API | "assetdelivery.roblox.com" | "rbxcdn.com")
            || host.ends_with(".rbxcdn.com")
    });
    if url.scheme() != "https"
        || !trusted
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err("Roblox returned a download URL outside its approved hosts.".into());
    }
    Ok(url)
}

pub fn animation_xml(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    convert_animation(bytes, false)
}

fn convert_animation(bytes: Vec<u8>, binary: bool) -> Result<Vec<u8>, String> {
    let mode = if binary {
        "--encode-animation"
    } else {
        "--decode-animation"
    };
    convert_in_worker(&bytes, mode, "animation")
}

pub fn model_preview(bytes: &[u8]) -> Result<Vec<u8>, String> {
    convert_in_worker(bytes, "--preview-model", "model")
}

fn convert_in_worker(bytes: &[u8], mode: &str, asset: &str) -> Result<Vec<u8>, String> {
    let prepare = || format!("Could not prepare the {asset} preview.");
    let directory = tempfile::tempdir().map_err(|_| prepare())?;
    let input = directory.path().join("input.asset");
    let output = directory.path().join("preview.asset");
    fs::write(&input, bytes).map_err(|_| prepare())?;
    let program =
        std::env::current_exe().map_err(|_| format!("Could not find the {asset} decoder."))?;
    let mut command = Command::new(program);
    command
        .arg(mode)
        .arg(&input)
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // The model decoder runs outside the app so a malformed file cannot unwind into the UI.
    let mut child = command
        .spawn()
        .map_err(|_| format!("Could not start the {asset} decoder."))?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match child.try_wait() {
            Ok(Some(exit)) if exit.success() => return read_bounded(&output, asset),
            Ok(Some(_)) if asset == "animation" => {
                return Err(
                    "The file is not a supported Roblox KeyframeSequence animation.".into(),
                );
            }
            Ok(Some(_)) => return Err(format!("The Roblox {asset} could not be decoded.")),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "The {asset} decoder did not finish. Try a smaller {asset}."
                ));
            }
        }
    }
}

pub fn decode_animation_file(input: &Path, output: &Path, binary: bool) -> Result<(), String> {
    let bytes = read_bounded(input, "animation")?;
    let animation = decode_animation(&bytes, binary)?;
    fs::write(output, animation).map_err(|_| "Could not save the converted animation.".into())
}

pub fn decode_model_file(input: &Path, output: &Path) -> Result<(), String> {
    let bytes = read_bounded(input, "model")?;
    let preview = crate::model::preview(&bytes)?;
    fs::write(output, preview).map_err(|_| "Could not save the model preview.".into())
}

fn read_bounded(path: &Path, asset: &str) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|_| format!("Could not open the {asset} file."))?;
    let mut bytes = Vec::new();
    file.take(MAX_ASSET_SIZE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("Could not read the {asset} file."))?;
    if bytes.len() > MAX_ASSET_SIZE {
        return Err(format!("The {asset} is larger than 32 MiB."));
    }
    Ok(bytes)
}

fn decode_animation(bytes: &[u8], binary: bool) -> Result<Vec<u8>, String> {
    let dom = if bytes.starts_with(b"<roblox!") {
        validate_binary_animation(bytes)?;
        rbx_binary::from_reader(bytes)
            .map_err(|_| "This binary Roblox animation could not be decoded.")?
    } else {
        rbx_xml::from_reader_default(bytes).map_err(|_| "The Roblox animation XML is invalid.")?
    };
    let roots: Vec<_> = dom
        .root()
        .children()
        .iter()
        .copied()
        .filter(|id| {
            dom.get_by_ref(*id)
                .is_some_and(|item| item.class.as_str() == "KeyframeSequence")
        })
        .collect();
    if roots.len() != 1 {
        return Err("The file must contain exactly one top-level KeyframeSequence.".into());
    }
    let mut output = Vec::new();
    if binary {
        rbx_binary::to_writer(&mut output, &dom, &roots)
            .map_err(|_| "Could not convert the animation to Roblox binary format.")?;
    } else {
        rbx_xml::to_writer_default(&mut output, &dom, &roots)
            .map_err(|_| "This animation could not be prepared for playback.")?;
    }
    if output.len() > MAX_ASSET_SIZE {
        return Err("The converted Roblox animation exceeds 32 MiB.".into());
    }
    Ok(output)
}

fn unpack_download(bytes: Vec<u8>, limit: usize) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(b"\x1f\x8b") {
        return Ok(bytes);
    }
    let mut unpacked = Vec::new();
    MultiGzDecoder::new(bytes.as_slice())
        .take(limit as u64 + 1)
        .read_to_end(&mut unpacked)
        .map_err(|_| "Roblox returned a damaged gzip download.")?;
    if unpacked.len() > limit {
        return Err("The Roblox download expands to more than 32 MiB.".into());
    }
    Ok(unpacked)
}

pub fn prepare_remote_replacement(
    bytes: Vec<u8>,
    target_kind: &str,
    target_mime: &str,
) -> Result<Vec<u8>, String> {
    let bytes = unpack_download(bytes, MAX_ASSET_SIZE)?;
    if target_kind == "animation" {
        let downloaded_binary = bytes.starts_with(b"<roblox!");
        let target_binary = target_mime == BINARY_ANIMATION_MIME;
        if downloaded_binary != target_binary {
            return convert_animation(bytes, target_binary);
        }
    }
    validate_replacement(&bytes)?;
    Ok(bytes)
}

fn decode_image(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "The image type could not be identified.")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| "The image is invalid or too large to decode.".into())
}

pub fn image_preview(bytes: Vec<u8>, thumbnail: bool) -> Result<Vec<u8>, String> {
    let is_texture = is_supported_ktx2(&bytes);
    let decoded = if is_texture {
        decode_ktx2(&bytes, thumbnail)?
    } else {
        decode_image(&bytes)?
    };
    if !thumbnail && !is_texture {
        return Ok(bytes);
    }
    let mut output = Cursor::new(Vec::new());
    let image = if thumbnail {
        decoded.thumbnail(256, 192)
    } else {
        decoded
    };
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| "The image could not be encoded as a thumbnail.")?;
    Ok(output.into_inner())
}

pub fn validate_replacement(bytes: &[u8]) -> Result<(), String> {
    if audio::mime(bytes).is_some() {
        return Ok(());
    }
    if image::guess_format(bytes).is_ok() {
        return decode_image(bytes).map(|_| ());
    }
    if !bytes
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(bytes)
        .trim_ascii_start()
        .starts_with(b"<")
    {
        return Err(
            "Choose a supported image, audio file, or Roblox KeyframeSequence animation.".into(),
        );
    }
    animation_xml(bytes.to_vec()).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;

    #[test]
    fn accepts_roblox_location_envelopes_and_rejects_other_hosts() {
        let endpoint =
            Url::parse("https://apis.roblox.com/asset-delivery-api/v1/assetId/123").unwrap();
        let url = asset_location(
            &endpoint,
            br#"{"location":"https://c0.rbxcdn.com/asset","assetTypeId":24}"#,
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("c0.rbxcdn.com"));
        assert!(
            asset_location(
                &endpoint,
                br#"{"location":"https://rbxcdn.com.example.test/asset"}"#
            )
            .is_err()
        );
        assert!(
            asset_location(
                &endpoint,
                br#"{"location":"https://rbxcdn.com/asset","isArchived":true}"#
            )
            .is_err()
        );
        assert!(asset_location(&endpoint, br#"{"errors":[{}]}"#).is_err());
    }

    #[test]
    fn bounds_gzip_expansion_and_converts_animation_encodings() {
        let xml = br#"<roblox version="4"><Item class="KeyframeSequence"><Properties>
            <string name="Name">Source clip</string>
            </Properties></Item></roblox>"#;
        let binary = decode_animation(xml, true).unwrap();
        assert!(binary.starts_with(b"<roblox!"));
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&binary).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(unpack_download(compressed.clone(), binary.len() - 1).is_err());
        assert_eq!(unpack_download(compressed, binary.len()).unwrap(), binary);
        let reopened = decode_animation(&binary, false).unwrap();
        assert!(String::from_utf8(reopened).unwrap().contains("Source clip"));
    }
}
