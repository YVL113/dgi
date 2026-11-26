use serde::Serialize;
use std::ffi::OsString;

use crate::delivery::key_header;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Manual,
    Process,
    #[cfg(windows)]
    WindowsUser,
    #[cfg(windows)]
    WindowsSystem,
}

pub struct CloudKey {
    pub value: String,
    pub source: Source,
}

impl CloudKey {
    pub fn manual(value: &str) -> Result<Option<Self>, String> {
        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }
        key_header(value)?;
        Ok(Some(Self {
            value: value.into(),
            source: Source::Manual,
        }))
    }

    pub fn detect() -> Option<Self> {
        pick(
            std::env::vars_os().filter_map(|(name, value)| Some((name.into_string().ok()?, value))),
            Source::Process,
        )
        .or_else(platform_key)
    }
}

fn rank(name: &str) -> Option<u8> {
    let normalized: String = name
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_uppercase() as char)
        .collect();
    match normalized.as_str() {
        "DGIROBLOXAPIKEY" => Some(0),
        "ROBLOXOPENCLOUDAPIKEY" | "RBXOPENCLOUDAPIKEY" => Some(1),
        "ROBLOXCLOUDAPIKEY" | "RBXCLOUDAPIKEY" | "ROBLOXASSETDELIVERYAPIKEY" => Some(2),
        "ROBLOXAPIKEY" | "RBXAPIKEY" | "OPENCLOUDROBLOXAPIKEY" => Some(3),
        "ROBLOXOPENCLOUDKEY" | "RBXOPENCLOUDKEY" | "ROBLOXCLOUDKEY" | "RBXCLOUDKEY"
        | "OPENCLOUDAPIKEY" => Some(4),
        _ => {
            let name = name.to_ascii_uppercase();
            let parts: Vec<_> = name.split(|ch: char| !ch.is_ascii_alphanumeric()).collect();
            let roblox = parts.contains(&"ROBLOX") || parts.contains(&"RBX");
            let api_key =
                parts.windows(2).any(|pair| pair == ["API", "KEY"]) || parts.contains(&"APIKEY");
            let cloud_key = parts.contains(&"KEY")
                && (parts.contains(&"CLOUD") || parts.contains(&"OPENCLOUD"));
            (roblox
                && (api_key || cloud_key)
                && !parts.iter().any(|part| {
                    matches!(
                        *part,
                        "NAME" | "URL" | "PATH" | "FILE" | "ID" | "IDS" | "COOKIE"
                    )
                }))
            .then_some(5)
        }
    }
}

fn pick(values: impl IntoIterator<Item = (String, OsString)>, source: Source) -> Option<CloudKey> {
    values
        .into_iter()
        .filter_map(|(name, value)| {
            let priority = rank(&name)?;
            let value = value.into_string().ok()?;
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| {
                    value
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                })
                .unwrap_or(value);
            if value.is_empty()
                || value.starts_with('$')
                || (value.starts_with('%') && value.ends_with('%'))
            {
                return None;
            }
            key_header(value).ok()?;
            Some((priority, name, value.to_owned()))
        })
        .min_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)))
        .map(|(_, _, value)| CloudKey { value, source })
}

#[cfg(windows)]
fn platform_key() -> Option<CloudKey> {
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ},
    };

    [
        (HKEY_CURRENT_USER, "Environment", Source::WindowsUser),
        (
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            Source::WindowsSystem,
        ),
    ]
    .into_iter()
    .find_map(|(hive, path, source)| {
        let environment = RegKey::predef(hive)
            .open_subkey_with_flags(path, KEY_READ)
            .ok()?;
        pick(
            environment.enum_values().filter_map(|entry| {
                let (name, _) = entry.ok()?;
                rank(&name)?;
                let value = environment.get_value(&name).ok()?;
                Some((name, value))
            }),
            source,
        )
    })
}

#[cfg(not(windows))]
fn platform_key() -> Option<CloudKey> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_variants_and_ignores_metadata_or_other_providers() {
        let inputs = [
            ("OPENAI_API_KEY", "elsewhere"),
            ("ROBLOX_API_KEY_FILE", "file-path"),
            ("ROBLOX_API_KEY_ID", "key-id"),
            ("Roblox-Cloud-Api-Key", "bad value"),
            ("dev_roblox_open_cloud_key", "fake-dev-key"),
            ("rbx_api_key", "\"fake-preferred-key\""),
        ];
        let result = pick(
            inputs.map(|(name, value)| (name.into(), value.into())),
            Source::Process,
        )
        .unwrap();
        assert_eq!(result.value, "fake-preferred-key");
        assert!(matches!(result.source, Source::Process));
        assert!(rank("RobloxOpenCloudApiKey").is_some());
        assert!(rank("ROBLOX_API_KEY_FILE").is_none());
        assert!(rank("ROBLOX_COOKIE").is_none());
        assert_eq!(
            serde_json::to_string(&result.source).unwrap(),
            "\"process\""
        );
    }
}
