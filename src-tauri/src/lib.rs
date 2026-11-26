mod asset_names;
mod audio;
mod cache;
mod cloud_key;
mod delivery;
mod mesh;
mod model;
mod rbxh;
mod sqlite_cache;
mod texture;
mod workspace;

use cache::{Asset, Cache, Change, ClearReport, MAX_ASSET_SIZE, ScanProgress};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{
    AppHandle, Emitter, Manager, WebviewWindowBuilder, async_runtime::spawn_blocking, ipc::Response,
};
use tauri_plugin_dialog::DialogExt;
#[cfg(windows)]
use tauri_plugin_window_controls::WindowControlsBuilderExt;

pub use delivery::{decode_animation_file, decode_model_file};

#[cfg(windows)]
// Explicit browser arguments replace Wry's defaults, including its autoplay policy.
const WINDOWS_BROWSER_ARGUMENTS: &str = concat!(
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection ",
    "--autoplay-policy=no-user-gesture-required ",
    "--use-angle=vulkan",
);

struct Library {
    cache: Cache,
    folder: Option<PathBuf>,
    detected: bool,
    workspace: PathBuf,
    source: Option<CacheSelection>,
}

#[derive(Clone)]
struct CacheSelection {
    folder: PathBuf,
    detected: bool,
}

struct AppState {
    library: Mutex<Library>,
    key: Mutex<Option<cloud_key::CloudKey>>,
    data_dir: PathBuf,
    http: reqwest::Client,
}

#[derive(Serialize)]
struct Snapshot {
    folder: Option<String>,
    detected: bool,
    workspace: bool,
    #[serde(rename = "cacheFolder")]
    cache_folder: Option<String>,
    #[serde(rename = "cacheDetected")]
    cache_detected: bool,
    #[serde(rename = "canClearCache")]
    can_clear_cache: bool,
    assets: Vec<Asset>,
    changes: Vec<Change>,
}

#[derive(Serialize)]
struct ClearedCache {
    snapshot: Snapshot,
    report: ClearReport,
    #[serde(rename = "rescanError")]
    rescan_error: Option<String>,
}

#[derive(Serialize)]
struct WorkspaceAsset {
    snapshot: Snapshot,
    id: String,
}

impl Library {
    fn scan(&mut self, folder: PathBuf, detected: bool) -> Result<Snapshot, String> {
        let assets = self.cache.scan(folder.clone())?;
        self.select(folder, detected, assets)
    }

    fn scan_with_progress(
        &mut self,
        folder: PathBuf,
        detected: bool,
        progress: impl FnMut(ScanProgress),
    ) -> Result<Snapshot, String> {
        let assets = self.cache.scan_with_progress(folder.clone(), progress)?;
        self.select(folder, detected, assets)
    }

    fn select(
        &mut self,
        folder: PathBuf,
        detected: bool,
        assets: Vec<Asset>,
    ) -> Result<Snapshot, String> {
        if folder != self.workspace {
            self.source = Some(CacheSelection {
                folder: folder.clone(),
                detected,
            });
        }
        self.folder = Some(folder);
        self.detected = detected;
        self.snapshot(assets)
    }

    fn snapshot(&self, assets: Vec<Asset>) -> Result<Snapshot, String> {
        Ok(Snapshot {
            folder: self
                .folder
                .as_ref()
                .map(|folder| folder.to_string_lossy().into_owned()),
            detected: self.detected,
            workspace: self.folder.as_ref() == Some(&self.workspace),
            cache_folder: self
                .source
                .as_ref()
                .map(|source| source.folder.to_string_lossy().into_owned()),
            cache_detected: self.source.as_ref().is_some_and(|source| source.detected),
            can_clear_cache: self.folder.as_deref().is_some_and(is_roblox_cache),
            assets,
            changes: self.cache.changes()?,
        })
    }

    fn refresh(&mut self) -> Result<Snapshot, String> {
        self.refresh_with_progress(|_| {})
    }

    fn refresh_with_progress(
        &mut self,
        progress: impl FnMut(ScanProgress),
    ) -> Result<Snapshot, String> {
        match self.folder.clone() {
            Some(folder) => self.scan_with_progress(folder, self.detected, progress),
            None => self.detect(progress),
        }
    }

    fn sync(&mut self) -> Result<Snapshot, String> {
        match self.folder.clone() {
            Some(folder) => {
                let assets = self.cache.scan_incremental(folder)?;
                self.snapshot(assets)
            }
            None => self.detect(|_| {}),
        }
    }

    fn detect(&mut self, mut progress: impl FnMut(ScanProgress)) -> Result<Snapshot, String> {
        let mut first_readable = None;
        let mut last_error = None;
        for path in cache_locations().into_iter().filter(|path| path.is_dir()) {
            match self.scan_with_progress(path.clone(), true, &mut progress) {
                Ok(snapshot) if !snapshot.assets.is_empty() || !snapshot.changes.is_empty() => {
                    return Ok(snapshot);
                }
                Ok(_) => {
                    first_readable.get_or_insert(path);
                }
                Err(error) => last_error = Some(error),
            }
        }
        if let Some(path) = first_readable {
            return self.scan_with_progress(path, true, progress);
        }
        if let Some(error) = last_error {
            return Err(error);
        }
        Ok(Snapshot {
            folder: None,
            detected: false,
            workspace: false,
            cache_folder: None,
            cache_detected: false,
            can_clear_cache: false,
            assets: Vec::new(),
            changes: Vec::new(),
        })
    }

    fn open_workspace(&mut self) -> Result<Snapshot, String> {
        workspace::ensure_directory(&self.workspace)?;
        self.scan(self.workspace.clone(), false)
    }

    fn open_library(&mut self, progress: impl FnMut(ScanProgress)) -> Result<Snapshot, String> {
        if let Some(source) = self.source.clone().filter(|source| source.folder.is_dir()) {
            return self.scan_with_progress(source.folder, source.detected, progress);
        }
        let snapshot = self.detect(progress)?;
        if snapshot.folder.is_none() {
            self.folder = None;
            self.detected = false;
            self.source = None;
        }
        Ok(snapshot)
    }

    fn add_to_workspace(
        &mut self,
        name: &str,
        mut bytes: Vec<u8>,
    ) -> Result<WorkspaceAsset, String> {
        if texture::is_supported_ktx2(&bytes) {
            bytes = delivery::image_preview(bytes, false)?;
        }
        if bytes.len() > MAX_ASSET_SIZE {
            return Err("The editable copy would exceed 32 MiB.".into());
        }
        let extension = cache::asset_extension(&bytes)
            .ok_or("Choose a supported image, audio file, or Roblox animation.")?;
        delivery::validate_replacement(&bytes)?;
        let name = Path::new(name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("asset");
        let path = workspace::add(&self.workspace, name, extension, &bytes)?;
        let snapshot = self.open_workspace()?;
        let id = snapshot
            .assets
            .iter()
            .find(|asset| asset.path == path)
            .ok_or("The copy was saved, but could not be displayed. Scan the workspace again.")?
            .id
            .clone();
        Ok(WorkspaceAsset { snapshot, id })
    }

    fn apply_workspace_asset(&mut self, id: &str) -> Result<WorkspaceAsset, String> {
        if self.folder.as_ref() != Some(&self.workspace) {
            return Err("Open the workspace edit before applying it to Roblox.".into());
        }
        let original = self.cache.original_preview(id)?;
        let replacement = self.cache.preview(id)?;
        let kind = self.cache.asset(id)?.kind;
        delivery::validate_replacement(&replacement)?;
        let result = (|| {
            let library = self.open_library(|_| {})?;
            let mut matches = Vec::new();
            for asset in library
                .assets
                .iter()
                .filter(|asset| asset.cache_entry && !asset.preview_only && asset.kind == kind)
            {
                // A workspace copy has its first original. A cache replacement has one too.
                // Comparing both lets an older copy be applied again without guessing by name.
                let current = self.cache.preview(&asset.id)?;
                let first = self.cache.original_preview(&asset.id)?;
                if current == original || first == original {
                    matches.push(asset.id.clone());
                }
            }
            let target = match matches.as_slice() {
                [target] => target.clone(),
                [] => return Err("The original animation or image is not in the selected Roblox cache. Load it in Roblox, close Player, then scan again. You can also select the target directly in Library.".into()),
                _ => return Err("More than one cache entry matches this workspace original. Select the exact target in Library and choose the workspace file as its replacement.".into()),
            };
            self.cache.replace(&target, &replacement)?;
            Ok(WorkspaceAsset {
                snapshot: self.refresh()?,
                id: target,
            })
        })();
        if result.is_err() {
            // Keep the interface and backend on the same workspace after an unsuccessful apply.
            self.open_workspace()?;
        }
        result
    }
}

fn cache_locations() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        paths.push(local.join("Roblox").join("rbx-storage"));
        paths.push(local.join("Temp").join("Roblox").join("http"));
        paths.push(local.join("Temp").join("Roblox").join("http-wob"));
    }
    let temp = std::env::temp_dir().join("Roblox");
    for folder in ["http", "http-wob"] {
        let path = temp.join(folder);
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

fn is_roblox_cache(folder: &Path) -> bool {
    if cache::plain_metadata(folder, true).is_err() {
        return false;
    }
    let Ok(canonical) = fs::canonicalize(folder) else {
        return false;
    };
    cache_locations().iter().any(|path| {
        cache::plain_metadata(path, true).is_ok()
            && fs::canonicalize(path).is_ok_and(|candidate| candidate == canonical)
    })
}

#[derive(Clone, Serialize)]
struct ScanUpdate {
    job: u32,
    #[serde(flatten)]
    progress: ScanProgress,
}

fn emit_scan_progress(app: AppHandle, job: Option<u32>) -> impl FnMut(ScanProgress) {
    move |progress| {
        if let Some(job) = job {
            let _ = app.emit("cache-scan-progress", ScanUpdate { job, progress });
        }
    }
}

async fn with_library<T: Send + 'static>(
    app: AppHandle,
    action: impl FnOnce(&mut Library) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut library = state
            .library
            .lock()
            .map_err(|_| "Restart DGI and try again.")?;
        action(&mut library)
    })
    .await
    .map_err(|_| "The background operation stopped unexpectedly.")?
}

#[tauri::command]
async fn scan(app: AppHandle, job: Option<u32>) -> Result<Snapshot, String> {
    let progress = emit_scan_progress(app.clone(), job);
    with_library(app, move |library| library.refresh_with_progress(progress)).await
}

#[tauri::command]
async fn refresh_cache(app: AppHandle) -> Result<Snapshot, String> {
    with_library(app, Library::sync).await
}

#[tauri::command]
async fn clear_cache(
    app: AppHandle,
    folder: PathBuf,
    job: Option<u32>,
) -> Result<ClearedCache, String> {
    let progress = emit_scan_progress(app.clone(), job);
    with_library(app, move |library| {
        if library.folder.as_ref() != Some(&folder) || !is_roblox_cache(&folder) {
            return Err("Select and scan the Roblox cache before clearing it.".into());
        }
        let report = library.cache.clear_scanned(&folder)?;
        let (snapshot, rescan_error) = match library.refresh_with_progress(progress) {
            Ok(snapshot) => (snapshot, None),
            Err(error) => (
                library.snapshot(library.cache.scanned_assets())?,
                Some(error),
            ),
        };
        Ok(ClearedCache {
            snapshot,
            report,
            rescan_error,
        })
    })
    .await
}

#[tauri::command]
async fn open_workspace(app: AppHandle) -> Result<Snapshot, String> {
    with_library(app, Library::open_workspace).await
}

#[tauri::command]
async fn open_library(app: AppHandle, job: Option<u32>) -> Result<Snapshot, String> {
    let progress = emit_scan_progress(app.clone(), job);
    with_library(app, move |library| library.open_library(progress)).await
}

#[tauri::command]
async fn copy_asset(app: AppHandle, id: String) -> Result<WorkspaceAsset, String> {
    with_library(app, move |library| {
        let name = library.cache.asset(&id)?.name.clone();
        let bytes = library.cache.preview(&id)?;
        library.add_to_workspace(&name, bytes)
    })
    .await
}

#[tauri::command]
async fn apply_workspace_asset(app: AppHandle, id: String) -> Result<WorkspaceAsset, String> {
    with_library(app, move |library| library.apply_workspace_asset(&id)).await
}

#[tauri::command]
async fn import_asset(app: AppHandle) -> Result<Option<WorkspaceAsset>, String> {
    let Some(path) = pick_asset_file(&app, "Choose an asset to import").await? else {
        return Ok(None);
    };
    with_library(app, move |library| {
        let bytes = read_replacement(&path)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("asset");
        library.add_to_workspace(name, bytes).map(Some)
    })
    .await
}

#[tauri::command]
async fn detect_cache(app: AppHandle, job: Option<u32>) -> Result<Snapshot, String> {
    let preference = app.state::<AppState>().data_dir.join("last-folder.json");
    let progress = emit_scan_progress(app.clone(), job);
    with_library(app, move |library| {
        let snapshot = library.detect(progress)?;
        if snapshot.detected {
            let _ = persist_folder(&preference, None);
        }
        Ok(snapshot)
    })
    .await
}

#[tauri::command]
async fn choose_cache(app: AppHandle, job: Option<u32>) -> Result<Option<Snapshot>, String> {
    let dialog_app = app.clone();
    let folder = spawn_blocking(move || {
        let mut picker = dialog_app
            .dialog()
            .file()
            .set_title("Choose a cache folder");
        if let Some(default) = cache_locations().into_iter().find(|path| path.is_dir()) {
            picker = picker.set_directory(default);
        }
        picker.blocking_pick_folder().map(|path| path.into_path())
    })
    .await
    .map_err(|_| "Could not open the folder picker.")?
    .transpose()
    .map_err(|_| "Choose a local folder.")?;
    let Some(folder) = folder else {
        return Ok(None);
    };
    let state = app.state::<AppState>();
    let preference = state.data_dir.join("last-folder.json");
    let progress = emit_scan_progress(app.clone(), job);
    with_library(app, move |library| {
        if fs::canonicalize(&folder)
            .ok()
            .zip(fs::canonicalize(&library.workspace).ok())
            .is_some_and(|(chosen, workspace)| chosen == workspace)
        {
            return library.open_workspace().map(Some);
        }
        let snapshot = library.scan_with_progress(folder.clone(), false, progress)?;
        // The cache's journal is independent of this convenience preference.
        let _ = persist_folder(&preference, Some(&folder));
        Ok(Some(snapshot))
    })
    .await
}

fn persist_folder(path: &Path, folder: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing settings folder")?)?;
    serde_json::to_writer(&mut file, &folder)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[tauri::command]
async fn image_preview(app: AppHandle, id: String) -> Result<Response, String> {
    let bytes = with_library(app, move |library| library.cache.preview(&id)).await?;
    spawn_blocking(move || delivery::image_preview(bytes, false))
        .await
        .map_err(|_| "The image could not be decoded.")?
        .map(Response::new)
}

#[tauri::command]
async fn thumbnail(app: AppHandle, id: String) -> Result<Response, String> {
    let bytes = with_library(app, move |library| library.cache.preview(&id)).await?;
    spawn_blocking(move || delivery::image_preview(bytes, true))
        .await
        .map_err(|_| "The thumbnail could not be created.")?
        .map(Response::new)
}

#[tauri::command]
async fn animation_preview(app: AppHandle, id: String) -> Result<Response, String> {
    let bytes = with_library(app, move |library| library.cache.preview(&id)).await?;
    spawn_blocking(move || delivery::animation_xml(bytes))
        .await
        .map_err(|_| "The animation could not be decoded.")?
        .map(Response::new)
}

#[tauri::command]
async fn audio_preview(app: AppHandle, id: String) -> Result<Response, String> {
    with_library(app, move |library| {
        if library.cache.asset(&id)?.kind != "audio" {
            return Err("The selected asset is not an audio file.".into());
        }
        library.cache.preview(&id).map(Response::new)
    })
    .await
}

#[tauri::command]
async fn mesh_preview(app: AppHandle, id: String) -> Result<Response, String> {
    let bytes = with_library(app, move |library| {
        if library.cache.asset(&id)?.kind != "mesh" {
            return Err("Select a Roblox mesh to preview.".into());
        }
        library.cache.preview(&id)
    })
    .await?;
    spawn_blocking(move || mesh::preview(&bytes))
        .await
        .map_err(|_| "The mesh could not be decoded.")?
        .map(Response::new)
}

#[tauri::command]
async fn model_preview(app: AppHandle, id: String) -> Result<Response, String> {
    let bytes = with_library(app, move |library| library.cache.model_preview(&id)).await?;
    spawn_blocking(move || delivery::model_preview(&bytes))
        .await
        .map_err(|_| "The model could not be decoded.")?
        .map(Response::new)
}

#[tauri::command]
async fn rig_meshes(app: AppHandle) -> Result<Response, String> {
    let path = app
        .state::<AppState>()
        .data_dir
        .join("studio-rig-meshes.json");
    spawn_blocking(move || read_rig_meshes(&path))
        .await
        .map_err(|_| "Could not load the Studio rig meshes.")?
        .map(Response::new)
}

fn read_rig_meshes(path: &Path) -> Result<Vec<u8>, String> {
    const MAX_MESH_PACK_BYTES: u64 = 2 * 1024 * 1024;
    let metadata = match cache::plain_metadata(path, false) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not inspect the Studio rig meshes: {error}")),
    };
    if metadata.len() > MAX_MESH_PACK_BYTES {
        return Err("The Studio rig mesh pack exceeds 2 MiB.".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_MESH_PACK_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("Could not read the Studio rig meshes: {error}"))?;
    if bytes.len() as u64 > MAX_MESH_PACK_BYTES {
        return Err("The Studio rig mesh pack exceeds 2 MiB.".into());
    }
    Ok(bytes)
}

#[tauri::command]
async fn rename(app: AppHandle, id: String, name: String) -> Result<Snapshot, String> {
    with_library(app, move |library| {
        library.cache.rename(&id, name)?;
        library.refresh()
    })
    .await
}

#[tauri::command]
async fn replace_local(app: AppHandle, id: String) -> Result<Option<Snapshot>, String> {
    let Some(picked) = pick_asset_file(&app, "Choose a replacement asset").await? else {
        return Ok(None);
    };
    let bytes = spawn_blocking(move || {
        let bytes = read_replacement(&picked)?;
        delivery::validate_replacement(&bytes)?;
        Ok::<_, String>(bytes)
    })
    .await
    .map_err(|_| "The replacement could not be read.")??;
    with_library(app, move |library| {
        library.cache.replace(&id, &bytes)?;
        library.refresh().map(Some)
    })
    .await
}

async fn pick_asset_file(app: &AppHandle, title: &'static str) -> Result<Option<PathBuf>, String> {
    let app = app.clone();
    spawn_blocking(move || app.dialog().file().set_title(title).blocking_pick_file())
        .await
        .map_err(|_| "Could not open the file picker.")?
        .map(|path| path.into_path().map_err(|_| "Choose a local file.".into()))
        .transpose()
}

fn read_replacement(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|err| format!("Could not open the replacement: {err}"))?;
    let meta = file
        .metadata()
        .map_err(|err| format!("Could not inspect the replacement: {err}"))?;
    if !meta.is_file() || meta.len() > MAX_ASSET_SIZE as u64 {
        return Err("Choose a regular file no larger than 32 MiB.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_ASSET_SIZE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("Could not read the replacement: {err}"))?;
    if bytes.len() > MAX_ASSET_SIZE {
        return Err("Choose a file no larger than 32 MiB.".into());
    }
    Ok(bytes)
}

#[tauri::command]
fn set_api_key(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<Option<cloud_key::Source>, String> {
    let key = cloud_key::CloudKey::manual(&key)?;
    let source = key.as_ref().map(|key| key.source);
    *state.key.lock().map_err(|_| "Restart DGI and try again.")? = key;
    Ok(source)
}

#[tauri::command]
fn api_key_status(state: tauri::State<'_, AppState>) -> Result<Option<cloud_key::Source>, String> {
    Ok(state
        .key
        .lock()
        .map_err(|_| "Restart DGI and try again.")?
        .as_ref()
        .map(|key| key.source))
}

#[tauri::command]
fn detect_api_key(state: tauri::State<'_, AppState>) -> Result<Option<cloud_key::Source>, String> {
    let detected = cloud_key::CloudKey::detect();
    let source = detected.as_ref().map(|key| key.source);
    if detected.is_some() {
        *state.key.lock().map_err(|_| "Restart DGI and try again.")? = detected;
    }
    Ok(source)
}

#[tauri::command]
async fn replace_remote(
    app: AppHandle,
    id: String,
    asset_id: String,
    job: u32,
) -> Result<Snapshot, String> {
    let state = app.state::<AppState>();
    let client = state.http.clone();
    let key = state
        .key
        .lock()
        .map_err(|_| "Restart DGI and try again.")?
        .as_ref()
        .map(|key| key.value.clone())
        .ok_or("Add your Roblox Open Cloud API key in Settings first.")?;
    let target_id = id.clone();
    let (kind, mime) = with_library(app.clone(), move |library| {
        let asset = library.cache.asset(&target_id)?;
        if asset.preview_only {
            return Err("Make an editable copy of this asset before replacing it.".into());
        }
        Ok((asset.kind, asset.mime))
    })
    .await?;
    let bytes = delivery::download(&client, &key, &asset_id, &app, job).await?;
    delivery::progress(&app, job, "Verifying and replacing asset…");
    let bytes = spawn_blocking(move || delivery::prepare_remote_replacement(bytes, kind, mime))
        .await
        .map_err(|_| "The replacement could not be decoded.")??;
    with_library(app, move |library| {
        library.cache.replace(&id, &bytes)?;
        library.refresh()
    })
    .await
}

#[tauri::command]
async fn restore(app: AppHandle, id: String) -> Result<Snapshot, String> {
    with_library(app, move |library| {
        library.cache.restore(&id)?;
        library.refresh()
    })
    .await
}

#[tauri::command]
async fn restore_all(app: AppHandle) -> Result<Snapshot, String> {
    with_library(app, move |library| {
        library.cache.restore_all()?;
        library.refresh()
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_window_controls::init());
    builder
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let workspace = data_dir.join("workspace");
            let canonical_workspace = fs::canonicalize(&workspace).ok();
            let folder = fs::read(data_dir.join("last-folder.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PathBuf>(&bytes).ok())
                .filter(|path| path.is_dir() && fs::canonicalize(path).ok() != canonical_workspace);
            app.manage(AppState {
                library: Mutex::new(Library {
                    cache: Cache::new(data_dir.join("library")),
                    folder,
                    detected: false,
                    workspace,
                    source: None,
                }),
                key: Mutex::new(cloud_key::CloudKey::detect()),
                data_dir,
                http: delivery::client()?,
            });
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .ok_or("The main window is missing from the app configuration.")?;
            let window = WebviewWindowBuilder::from_config(app, config)?;
            #[cfg(windows)]
            let window = window
                .transparent(true)
                .window_controls_height(44)
                .additional_browser_args(WINDOWS_BROWSER_ARGUMENTS);
            let window = window.build()?;
            #[cfg(windows)]
            if let Ok(hwnd) = window.hwnd() {
                use windows_sys::Win32::Graphics::Dwm::{
                    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
                };

                // Windows can ignore the preference on older versions or in snapped layouts.
                unsafe {
                    DwmSetWindowAttribute(
                        hwnd.0,
                        DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                        std::ptr::from_ref(&DWMWCP_ROUND).cast(),
                        std::mem::size_of_val(&DWMWCP_ROUND) as u32,
                    );
                }
            }
            window.show()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan,
            refresh_cache,
            clear_cache,
            open_library,
            open_workspace,
            copy_asset,
            apply_workspace_asset,
            import_asset,
            detect_cache,
            choose_cache,
            image_preview,
            thumbnail,
            animation_preview,
            audio_preview,
            mesh_preview,
            model_preview,
            rig_meshes,
            rename,
            replace_local,
            set_api_key,
            api_key_status,
            detect_api_key,
            replace_remote,
            restore,
            restore_all
        ])
        .run(tauri::generate_context!())
        .expect("error while running DGI");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png(color: [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn editable_copy_can_be_replaced_and_restored_without_changing_its_source() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_root = tmp.path().join("cache");
        fs::create_dir(&cache_root).unwrap();
        let original = png([255, 0, 0, 255]);
        let replacement = png([0, 255, 0, 255]);
        let mut wrapped = vec![0; 37];
        wrapped[..8].copy_from_slice(b"RBXH\x02\0\0\0");
        wrapped[13..17].copy_from_slice(&200_u32.to_le_bytes());
        wrapped[25..29].copy_from_slice(&(original.len() as u32).to_le_bytes());
        wrapped.extend_from_slice(&original);
        let source = cache_root.join("texture");
        fs::write(&source, &wrapped).unwrap();
        fs::write(cache_root.join("thumbnail.png"), &original).unwrap();
        let mut library = Library {
            cache: Cache::new(tmp.path().join("library")),
            folder: None,
            detected: false,
            workspace: tmp.path().join("workspace"),
            source: None,
        };
        let source_snapshot = library.scan(cache_root.clone(), true).unwrap();
        assert_eq!(source_snapshot.assets.len(), 2);
        let asset = &source_snapshot.assets[0];
        assert!(!asset.preview_only);
        let bytes = library.cache.preview(&asset.id).unwrap();
        let copy = library.add_to_workspace(&asset.name, bytes).unwrap();
        assert!(copy.snapshot.workspace);
        assert_eq!(copy.snapshot.assets.len(), 1);
        assert_eq!(copy.snapshot.cache_folder, source_snapshot.folder);
        assert!(!copy.snapshot.assets[0].preview_only);
        library.cache.replace(&copy.id, &replacement).unwrap();
        assert_eq!(library.refresh().unwrap().changes.len(), 1);
        assert_eq!(library.cache.preview(&copy.id).unwrap(), replacement);
        library.cache.restore(&copy.id).unwrap();
        assert_eq!(library.cache.preview(&copy.id).unwrap(), original);
        assert_eq!(fs::read(source).unwrap(), wrapped);

        let reopened = library.open_library(|_| {}).unwrap();
        assert!(!reopened.workspace);
        assert!(reopened.detected);
        assert_eq!(reopened.folder, source_snapshot.folder);
        assert_eq!(reopened.assets.len(), 2);
        assert_eq!(library.cache.preview(&asset.id).unwrap(), original);
        assert_eq!(library.open_workspace().unwrap().assets[0].id, copy.id);
    }

    fn workspace_apply_fixture(
        copies: usize,
    ) -> (tempfile::TempDir, Library, rusqlite::Connection, Vec<u8>) {
        let tmp = tempfile::tempdir().unwrap();
        let roblox = tmp.path().join("Roblox");
        let folder = roblox.join("rbx-storage");
        fs::create_dir_all(&folder).unwrap();
        let original = png([255, 0, 0, 255]);
        let mut record = vec![0; 37];
        record[..8].copy_from_slice(b"RBXH\x02\0\0\0");
        record[13..17].copy_from_slice(&200_u32.to_le_bytes());
        record[25..29].copy_from_slice(&(original.len() as u32).to_le_bytes());
        record[29..33].copy_from_slice(&rbxh::xxh32(&original).to_le_bytes());
        record.extend_from_slice(&original);
        let db = rusqlite::Connection::open(roblox.join("rbx-storage.db")).unwrap();
        db.execute_batch("CREATE TABLE files (id BLOB PRIMARY KEY, content BLOB, size INTEGER NOT NULL, category INTEGER DEFAULT 10);").unwrap();
        for n in 1..=copies {
            db.execute(
                "INSERT INTO files(id,content,size) VALUES (?1,?2,?3)",
                rusqlite::params![&[n as u8; 16][..], &record, record.len() as i64],
            )
            .unwrap();
        }
        let mut library = Library {
            cache: Cache::new(tmp.path().join("library")),
            folder: None,
            detected: false,
            workspace: tmp.path().join("workspace"),
            source: None,
        };
        library.scan(folder, true).unwrap();
        (tmp, library, db, record)
    }

    #[test]
    fn workspace_apply_updates_the_real_database_and_retains_the_first_original() {
        let (_tmp, mut library, db, original_record) = workspace_apply_fixture(1);
        let original_payload = rbxh::parse(&original_record, original_record.len() as u64)
            .unwrap()
            .body
            .to_vec();
        let copy = library.add_to_workspace("Image", original_payload).unwrap();
        let green = png([0, 255, 0, 255]);
        let blue = png([0, 0, 255, 255]);
        library.cache.replace(&copy.id, &green).unwrap();
        let read_record = || {
            db.query_row("SELECT content FROM files", [], |row| {
                row.get::<_, Vec<u8>>(0)
            })
            .unwrap()
        };
        assert_eq!(read_record(), original_record);
        let applied = library.apply_workspace_asset(&copy.id).unwrap();
        assert!(!applied.snapshot.workspace);
        assert_eq!(applied.snapshot.changes.len(), 1);
        assert_eq!(library.cache.preview(&applied.id).unwrap(), green);
        let saved = read_record();
        assert_eq!(rbxh::parse(&saved, saved.len() as u64).unwrap().body, green);
        library.open_workspace().unwrap();
        library.cache.replace(&copy.id, &blue).unwrap();
        let second = library.apply_workspace_asset(&copy.id).unwrap();
        assert_eq!(second.id, applied.id);
        assert_eq!(library.cache.preview(&second.id).unwrap(), blue);
        library.cache.restore(&second.id).unwrap();
        assert_eq!(read_record(), original_record);
        // Applying never discards the user's editable workspace file.
        library.open_workspace().unwrap();
        assert_eq!(library.cache.preview(&copy.id).unwrap(), blue);
    }

    #[test]
    fn workspace_apply_refuses_missing_or_ambiguous_sources_and_keeps_workspace_open() {
        for copies in [0, 2] {
            let (_tmp, mut library, db, original) = workspace_apply_fixture(copies);
            let body = rbxh::parse(&original, original.len() as u64)
                .unwrap()
                .body
                .to_vec();
            let copy = library.add_to_workspace("Image", body).unwrap();
            let replacement = png([0, 255, 0, 255]);
            library.cache.replace(&copy.id, &replacement).unwrap();
            let error = match library.apply_workspace_asset(&copy.id) {
                Ok(_) => panic!("Should reject a missing or ambiguous source"),
                Err(error) => error,
            };
            assert!(error.contains(if copies == 0 {
                "not in the selected"
            } else {
                "More than one"
            }));
            assert!(
                library
                    .snapshot(library.cache.scanned_assets())
                    .unwrap()
                    .workspace
            );
            assert_eq!(library.cache.preview(&copy.id).unwrap(), replacement);
            let rows = db
                .prepare("SELECT content FROM files")
                .unwrap()
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(rows.iter().all(|row| *row == original));
        }
    }
}
