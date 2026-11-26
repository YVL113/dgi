use crate::{
    asset_names::{self, BinaryNames, Identity},
    audio, rbxh, sqlite_cache,
    texture::{KTX2_MIME, is_supported_ktx2},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

pub const MAX_ASSET_SIZE: usize = 32 * 1024 * 1024;
const MAX_SEARCH_ENTRIES: usize = 100_000;
const MAX_SEARCH_DEPTH: usize = 32;
const SCAN_UPDATE_INTERVAL: Duration = Duration::from_millis(120);
const MAX_STATE_SIZE: usize = 32 * 1024 * 1024;
const MAX_NAME_LENGTH: usize = 120;
const HEADER_SAMPLE_SIZE: usize = 4096;
const BINARY_MAGIC: &[u8] = b"<roblox!\x89\xff\r\n\x1a\n";
const HTTP_CACHE_MAGIC: &[u8] = b"RBXH\x02\0\0\0";
const HTTP_CACHE_HEADER_SIZE: usize = 37;
const MAX_BINARY_CLASSES: usize = 256;
const MAX_BINARY_INSTANCES: usize = 65_536;
const MAX_BINARY_CHUNKS: usize = 2_048;
const MAX_BINARY_CLASS_CHUNK: usize = 512 * 1024;
const MAX_BINARY_CHUNK: usize = 8 * 1024 * 1024;
const MAX_BINARY_EXPANDED: usize = 64 * 1024 * 1024;

pub const BINARY_ANIMATION_MIME: &str = "application/x-roblox-binary";

type CacheResult<T> = Result<T, String>;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub default_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_asset_id: Option<String>,
    pub path: String,
    pub kind: &'static str,
    pub category: AssetCategory,
    pub mime: &'static str,
    pub size: u64,
    pub modified: u64,
    pub checksum: String,
    pub changed: bool,
    pub preview_only: bool,
    pub database_entry: bool,
    pub cache_entry: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetCategory {
    Image,
    Texture,
    Animation,
    Avatar,
    Model,
    Mesh,
    Audio,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: String,
    pub name: String,
    pub path: String,
    pub original_checksum: String,
    pub current_checksum: String,
    pub changed_at: u64,
}

#[derive(Clone, Copy, Serialize)]
pub struct ScanProgress {
    pub checked: usize,
    pub total: Option<usize>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearReport {
    pub removed: usize,
    pub kept: usize,
    pub failed: usize,
    pub database_removed: usize,
    pub database_error: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Index {
    records: BTreeMap<String, Record>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    relative: PathBuf,
    name: Option<String>,
    history: Option<History>,
}

#[derive(Clone, Serialize, Deserialize)]
struct History {
    original: String,
    current: String,
    changed_at: u64,
    pending: Option<Pending>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Pending {
    before: Option<String>,
    after: String,
    changed_at: u64,
}

#[derive(Clone, Copy)]
struct Format {
    kind: &'static str,
    category: AssetCategory,
    mime: &'static str,
}

#[derive(Clone)]
struct SeenAsset {
    relative: PathBuf,
    asset: Asset,
    stamp: Option<(u64, SystemTime)>,
    inline_record: Option<Arc<[u8]>>,
}

struct Session {
    root: PathBuf,
    storage: PathBuf,
    index: Index,
    seen: BTreeMap<String, SeenAsset>,
    unsupported: HashMap<PathBuf, (u64, SystemTime)>,
    _lock: File,
}

pub struct Cache {
    data_dir: PathBuf,
    session: Option<Session>,
}

impl Cache {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            session: None,
        }
    }

    pub fn scan(&mut self, root: PathBuf) -> CacheResult<Vec<Asset>> {
        self.scan_with_progress(root, |_| {})
    }

    pub fn scan_with_progress(
        &mut self,
        root: PathBuf,
        progress: impl FnMut(ScanProgress),
    ) -> CacheResult<Vec<Asset>> {
        self.scan_root(root, false, progress)
    }

    pub fn scan_incremental(&mut self, root: PathBuf) -> CacheResult<Vec<Asset>> {
        self.scan_root(root, true, |_| {})
    }

    fn scan_root(
        &mut self,
        root: PathBuf,
        reuse_unchanged: bool,
        progress: impl FnMut(ScanProgress),
    ) -> CacheResult<Vec<Asset>> {
        plain_metadata(&root, true).map_err(|error| format!("Cannot scan that folder: {error}"))?;
        let root = fs::canonicalize(&root).map_err(io_message("Cannot open the cache folder"))?;
        fs::create_dir_all(&self.data_dir)
            .map_err(io_message("Cannot create the backup folder"))?;
        let data_dir = fs::canonicalize(&self.data_dir)
            .map_err(io_message("Cannot open the backup folder"))?;
        if root.starts_with(&data_dir) {
            return Err("Choose a cache folder outside the app's backup folder.".into());
        }
        if let Some(session) = self.session.as_mut().filter(|session| session.root == root) {
            return session.scan(&data_dir, reuse_unchanged, progress);
        }
        let mut session = Session::open(root, &data_dir)?;
        let assets = session.scan(&data_dir, false, progress)?;
        self.session = Some(session);
        Ok(assets)
    }

    pub fn clear_scanned(&mut self, root: &Path) -> CacheResult<ClearReport> {
        let root = fs::canonicalize(root).map_err(io_message("Cannot open the cache folder"))?;
        let session = self.session_mut()?;
        if session.root != root {
            return Err("The cache folder changed. Scan it and try again.".into());
        }
        ensure_player_closed(&session.root)?;
        let mut report = ClearReport::default();
        let mut gone = Vec::new();
        for (id, seen) in &session.seen {
            if session.index.records.get(id).is_some_and(record_changed) {
                report.kept += 1;
                continue;
            }
            if seen.inline_record.is_some() {
                continue;
            }
            let Ok(path) = session.target(&seen.relative) else {
                report.failed += 1;
                continue;
            };
            match target_metadata(&path) {
                Ok(None) => gone.push(id.clone()),
                Ok(Some(metadata))
                    if seen.stamp.is_some() && file_stamp(&metadata) == seen.stamp =>
                {
                    match fs::remove_file(&path) {
                        Ok(()) => {
                            report.removed += 1;
                            gone.push(id.clone());
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            gone.push(id.clone())
                        }
                        Err(_) => report.failed += 1,
                    }
                }
                Ok(Some(_)) => report.kept += 1,
                Err(_) => report.failed += 1,
            }
        }
        session.clear_unlisted_http_files(&mut report);
        for id in gone {
            session.seen.remove(&id);
        }
        let protected: Vec<String> = session
            .index
            .records
            .values()
            .filter(|record| record_changed(record))
            .filter_map(|record| database_key(&record.relative).map(str::to_owned))
            .collect();
        match sqlite_cache::clear_http_responses(&root, &protected) {
            Ok(removed) => {
                report.database_removed = removed;
                session.seen.retain(|id, asset| {
                    asset.inline_record.is_none()
                        || session.index.records.get(id).is_some_and(record_changed)
                });
            }
            Err(error) => report.database_error = Some(error),
        }
        Ok(report)
    }

    pub fn scanned_assets(&self) -> Vec<Asset> {
        self.session.as_ref().map_or_else(Vec::new, Session::assets)
    }

    pub fn preview(&self, id: &str) -> CacheResult<Vec<u8>> {
        if self.asset(id)?.kind == "reference" {
            return Err("Open the model preview for this asset.".into());
        }
        self.preview_bytes(id)
    }

    pub fn model_preview(&self, id: &str) -> CacheResult<Vec<u8>> {
        if !matches!(self.asset(id)?.kind, "reference" | "animation") {
            return Err("Select a Roblox model to preview.".into());
        }
        self.preview_bytes(id)
    }

    fn preview_bytes(&self, id: &str) -> CacheResult<Vec<u8>> {
        let session = self
            .session
            .as_ref()
            .ok_or("Select and scan a cache folder first.")?;
        let seen = session.seen_asset(id)?;
        if seen.inline_record.is_some() {
            let bytes = session
                .read_current(&seen.relative)?
                .ok_or("This cache entry was evicted. Scan again.")?;
            if checksum(&bytes) != seen.asset.checksum {
                return Err("This asset changed in the database. Scan the cache again.".into());
            }
            return Ok(http_cache_payload(&bytes, bytes.len() as u64)
                .ok_or("The cached response is no longer valid.")?
                .to_vec());
        }
        let path = session.target(&seen.relative)?;
        let bytes = read_asset(&path)?;
        if checksum(&bytes) != seen.asset.checksum {
            return Err("This asset changed on disk. Scan the folder again.".into());
        }
        if bytes.starts_with(HTTP_CACHE_MAGIC) {
            let payload = http_cache_payload(&bytes, bytes.len() as u64)
                .ok_or("The cached response is no longer valid.")?;
            return Ok(payload.to_vec());
        }
        Ok(bytes)
    }

    pub fn asset(&self, id: &str) -> CacheResult<&Asset> {
        let session = self
            .session
            .as_ref()
            .ok_or("Select a cache folder first.")?;
        Ok(&session.seen_asset(id)?.asset)
    }

    pub fn original_preview(&self, id: &str) -> CacheResult<Vec<u8>> {
        let session = self
            .session
            .as_ref()
            .ok_or("Select a cache folder first.")?;
        let Some(history) = session.history(id) else {
            return self.preview(id);
        };
        let record = session.verify_backup(id, &history.original)?;
        if record.starts_with(HTTP_CACHE_MAGIC) {
            return Ok(http_cache_payload(&record, record.len() as u64)
                .ok_or("The original cache record is invalid.")?
                .to_vec());
        }
        Ok(record)
    }

    pub fn rename(&mut self, id: &str, name: String) -> CacheResult<()> {
        let name = name.trim();
        if name.chars().count() > MAX_NAME_LENGTH || name.chars().any(char::is_control) {
            return Err(format!(
                "Use a name of up to {MAX_NAME_LENGTH} characters with no control characters."
            ));
        }
        let session = self.session_mut()?;
        let relative = session.seen_asset(id)?.relative.clone();
        let name = (!name.is_empty()).then(|| name.to_owned());
        session.update_index(|index| {
            index.record(id, &relative).name = name;
        })?;
        if let Some(seen) = session.seen.get_mut(id) {
            let asset = &mut seen.asset;
            asset.custom_name = session
                .index
                .records
                .get(id)
                .and_then(|record| record.name.clone());
            asset.name = asset
                .custom_name
                .as_ref()
                .unwrap_or(&asset.default_name)
                .clone();
        }
        Ok(())
    }

    pub fn replace(&mut self, id: &str, bytes: &[u8]) -> CacheResult<()> {
        if bytes.is_empty() || bytes.len() > MAX_ASSET_SIZE {
            return Err(format!(
                "The replacement must be between 1 byte and {} MiB.",
                MAX_ASSET_SIZE / (1024 * 1024)
            ));
        }
        let format = classify(bytes).ok_or("The replacement is not a supported asset file.")?;
        let session = self.session_mut()?;
        ensure_player_closed(&session.root)?;
        session.reconcile(id)?;
        let seen = session.seen_asset(id)?;
        if seen.asset.preview_only {
            return Err("This Roblox cache entry is available for preview only.".into());
        }
        if format.kind != seen.asset.kind {
            return Err(format!("Choose a replacement {} file.", seen.asset.kind));
        }
        let relative = seen.relative.clone();
        if let Some(expected) = extension_mime(&relative)
            && format.mime != expected
        {
            return Err(format!("Choose a replacement in the {expected} format."));
        }
        let before = session
            .read_current(&relative)?
            .ok_or("The cached asset is missing. Scan again.")?;
        if before.starts_with(HTTP_CACHE_MAGIC) {
            sqlite_cache::check_external_target(&session.root, &relative)?;
        }
        let before_hash = checksum(&before);
        if before_hash != seen.asset.checksum {
            return Err("This asset changed on disk. Scan the folder again.".into());
        }
        if let Some(history) = session.history(id)
            && before_hash != history.current
        {
            return Err(
                "The file differs from the last verified version. The backup is intact.".into(),
            );
        }
        let record;
        let bytes = if before.starts_with(HTTP_CACHE_MAGIC) {
            if http_cache_payload(&before, before.len() as u64) == Some(bytes) {
                return Ok(());
            }
            record = rbxh::replace_body(&before, bytes)?;
            record.as_slice()
        } else {
            bytes
        };
        if bytes.len() > MAX_ASSET_SIZE {
            return Err("The replacement cache record exceeds 32 MiB.".into());
        }
        let after_hash = checksum(bytes);
        if before_hash == after_hash {
            return Ok(());
        }
        let original = session.ensure_backup(id, &before, &before_hash)?;
        session.write_change(
            id,
            &relative,
            &original,
            Some(before_hash),
            &after_hash,
            bytes,
        )
    }

    pub fn changes(&self) -> CacheResult<Vec<Change>> {
        let Some(session) = &self.session else {
            return Ok(Vec::new());
        };
        let mut changes: Vec<_> = session
            .index
            .records
            .iter()
            .filter_map(|(id, record)| {
                let history = record.history.as_ref()?;
                if history.original == history.current && history.pending.is_none() {
                    return None;
                }
                Some(Change {
                    id: id.clone(),
                    name: session
                        .seen
                        .get(id)
                        .map(|seen| seen.asset.name.clone())
                        .or_else(|| record.name.clone())
                        .unwrap_or_else(|| file_name(&record.relative).into_owned()),
                    path: display_path(&record.relative),
                    original_checksum: history.original.clone(),
                    current_checksum: history.current.clone(),
                    changed_at: history.changed_at,
                })
            })
            .collect();
        changes.sort_by(|a, b| {
            b.changed_at
                .cmp(&a.changed_at)
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok(changes)
    }

    pub fn restore(&mut self, id: &str) -> CacheResult<()> {
        self.session_mut()?.restore(id)
    }

    pub fn restore_all(&mut self) -> CacheResult<()> {
        let ids: Vec<_> = self
            .changes()?
            .into_iter()
            .map(|change| change.id)
            .collect();
        let mut failures = Vec::new();
        for id in ids {
            let session = self.session_mut()?;
            if let Err(error) = session.restore(&id) {
                let path = session
                    .index
                    .records
                    .get(&id)
                    .map(|record| display_path(&record.relative));
                failures.push(format!(
                    "{}: {error}",
                    path.as_deref().unwrap_or("Unknown asset")
                ));
            }
        }
        if failures.is_empty() {
            return Ok(());
        }
        let failed_count = failures.len();
        failures.truncate(3);
        Err(format!(
            "Could not restore {failed_count} asset(s). {}",
            failures.join(" · ")
        ))
    }

    fn session_mut(&mut self) -> CacheResult<&mut Session> {
        self.session
            .as_mut()
            .ok_or_else(|| "Select and scan a cache folder first.".into())
    }
}

impl Index {
    fn record(&mut self, id: &str, relative: &Path) -> &mut Record {
        self.records.entry(id.to_owned()).or_insert_with(|| Record {
            relative: relative.to_path_buf(),
            name: None,
            history: None,
        })
    }
}

impl Session {
    fn open(root: PathBuf, data_dir: &Path) -> CacheResult<Self> {
        let storage = data_dir.join(path_hash(&root));
        fs::create_dir_all(&storage).map_err(io_message("Cannot create the backup folder"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(storage.join("workspace.lock"))
            .map_err(io_message("Cannot open the backup lock"))?;
        lock.try_lock().map_err(|error| {
            format!("Cannot lock this cache's backup state. Close other app instances. {error}")
        })?;
        let index_path = storage.join("index.json");
        let index: Index = match File::open(index_path) {
            Ok(file) => {
                let bytes = read_limited(file, MAX_STATE_SIZE)
                    .map_err(io_message("Cannot read the backup index"))?;
                serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Cannot parse the backup index: {error}"))?
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Index::default(),
            Err(error) => return Err(format!("Cannot open the backup index: {error}")),
        };
        for (id, record) in &index.records {
            validate_relative(&record.relative)?;
            if *id != asset_id(&root, &record.relative) {
                return Err("The backup index contains an invalid asset identifier.".into());
            }
        }
        Ok(Self {
            root,
            storage,
            index,
            seen: BTreeMap::new(),
            unsupported: HashMap::new(),
            _lock: lock,
        })
    }

    fn scan(
        &mut self,
        data_dir: &Path,
        reuse_unchanged: bool,
        mut progress: impl FnMut(ScanProgress),
    ) -> CacheResult<Vec<Asset>> {
        let mut directories = vec![(self.root.clone(), 0)];
        let mut entry_count = 0;
        let mut files = Vec::new();
        progress(ScanProgress {
            checked: 0,
            total: None,
        });
        let mut last_update = Instant::now();
        while let Some((directory, depth)) = directories.pop() {
            if let Err(error) = plain_metadata(&directory, true) {
                if depth == 0 {
                    return Err(format!("Cannot read the cache folder: {error}"));
                }
                continue;
            }
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) if depth == 0 => {
                    return Err(format!("Cannot read the cache folder: {error}"));
                }
                Err(_) => continue,
            };
            for entry in entries {
                entry_count += 1;
                if entry_count > MAX_SEARCH_ENTRIES {
                    return Err(format!(
                        "The folder has more than {MAX_SEARCH_ENTRIES} entries. Select a smaller cache folder."
                    ));
                }
                let Ok(entry) = entry else { continue };
                let path = entry.path();
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if is_link(&metadata) {
                    continue;
                }
                if metadata.is_dir() {
                    if depth < MAX_SEARCH_DEPTH && path != data_dir {
                        directories.push((path, depth + 1));
                    }
                    continue;
                }
                if metadata.is_file() {
                    files.push((path, metadata));
                    if last_update.elapsed() >= SCAN_UPDATE_INTERVAL {
                        progress(ScanProgress {
                            checked: files.len(),
                            total: None,
                        });
                        last_update = Instant::now();
                    }
                }
            }
        }
        let total = Some(files.len());
        progress(ScanProgress { checked: 0, total });
        last_update = Instant::now();
        let mut seen = BTreeMap::new();
        let mut unsupported = HashMap::new();
        for (index, (path, metadata)) in files.into_iter().enumerate() {
            if let Ok(relative) = path.strip_prefix(&self.root)
                && validate_relative(relative).is_ok()
            {
                let stamp = file_stamp(&metadata);
                if let Some(stamp) = stamp
                    && reuse_unchanged
                    && self.unsupported.get(relative) == Some(&stamp)
                {
                    unsupported.insert(relative.to_path_buf(), stamp);
                } else {
                    let id = asset_id(&self.root, relative);
                    let cached = self
                        .seen
                        .get(&id)
                        .filter(|entry| reuse_unchanged && stamp.is_some() && entry.stamp == stamp);
                    let asset = match cached {
                        Some(entry) => Ok(Some(entry.asset.clone())),
                        None => self.snapshot(&id, relative, &path, metadata),
                    };
                    match asset {
                        Ok(Some(asset)) => {
                            seen.insert(
                                id,
                                SeenAsset {
                                    relative: relative.to_path_buf(),
                                    asset,
                                    stamp,
                                    inline_record: None,
                                },
                            );
                        }
                        Ok(None) => {
                            if let Some(stamp) = stamp {
                                unsupported.insert(relative.to_path_buf(), stamp);
                            }
                        }
                        Err(_) => {} // Retry transient read errors at the next scan.
                    }
                }
            }
            if last_update.elapsed() >= SCAN_UPDATE_INTERVAL {
                progress(ScanProgress {
                    checked: index + 1,
                    total,
                });
                last_update = Instant::now();
            }
        }
        self.add_database_assets(&mut seen, reuse_unchanged);
        self.seen = seen;
        self.unsupported = unsupported;
        let pending_ids: Vec<_> = self
            .index
            .records
            .keys()
            .filter(|id| {
                self.history(id)
                    .is_some_and(|history| history.pending.is_some())
            })
            .cloned()
            .collect();
        for id in pending_ids {
            // A conflicting file stays listed in Changes while the rest of the cache remains usable.
            let _ = self.reconcile(&id);
            if let Some(seen) = self.seen.get_mut(&id) {
                seen.asset.changed = self.index.records.get(&id).is_some_and(record_changed);
            }
        }
        progress(ScanProgress {
            checked: total.unwrap_or_default(),
            total,
        });
        Ok(self.assets())
    }

    fn assets(&self) -> Vec<Asset> {
        let mut assets: Vec<_> = self.seen.values().map(|seen| seen.asset.clone()).collect();
        assets.sort_by(|a, b| a.path.cmp(&b.path));
        assets
    }

    fn clear_unlisted_http_files(&self, report: &mut ClearReport) {
        if self.root.file_name().and_then(|name| name.to_str())
            != Some(sqlite_cache::STORAGE_FOLDER)
        {
            return;
        }
        for index in 0..=u8::MAX {
            let shard = format!("{index:02x}");
            let folder = self.root.join(&shard);
            match plain_metadata(&folder, true) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => {
                    report.failed += 1;
                    continue;
                }
            }
            let Ok(entries) = fs::read_dir(&folder) else {
                report.failed += 1;
                continue;
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    report.failed += 1;
                    continue;
                };
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if name.len() != 32
                    || !name.starts_with(&shard)
                    || !name.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    continue;
                }
                let relative = Path::new(&shard).join(name);
                let id = asset_id(&self.root, &relative);
                if self.seen.contains_key(&id)
                    || self.index.records.get(&id).is_some_and(record_changed)
                {
                    continue;
                }
                let Ok(path) = self.target(&relative) else {
                    report.failed += 1;
                    continue;
                };
                match remove_unlisted_http_file(&path) {
                    Ok(true) => report.removed += 1,
                    Ok(false) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => report.failed += 1,
                }
            }
        }
    }

    fn add_database_assets(&self, seen: &mut BTreeMap<String, SeenAsset>, reuse_unchanged: bool) {
        let responses = match sqlite_cache::read_asset_responses(&self.root) {
            Ok(responses) => responses,
            Err(_) => {
                // Roblox may have the database locked briefly. Keep the last successful snapshot.
                seen.extend(
                    self.seen
                        .iter()
                        .filter(|(_, entry)| entry.inline_record.is_some())
                        .map(|(id, entry)| (id.clone(), entry.clone())),
                );
                return;
            }
        };
        for (key, response) in responses {
            let relative = Path::new(sqlite_cache::DATABASE_NAME).join(key);
            let id = asset_id(&self.root, &relative);
            if reuse_unchanged
                && let Some(previous) = self.seen.get(&id)
                && previous.inline_record.as_deref() == Some(response.as_slice())
            {
                seen.insert(id, previous.clone());
                continue;
            }
            let Some(body) = http_cache_payload(&response, response.len() as u64) else {
                continue;
            };
            let mut identity = Identity::default();
            let Some(format) = classify_with_names(body, Some(&mut identity)) else {
                continue;
            };
            if !matches!(
                format.category,
                AssetCategory::Animation
                    | AssetCategory::Mesh
                    | AssetCategory::Image
                    | AssetCategory::Texture
                    | AssetCategory::Audio
            ) {
                continue;
            }
            let default_name = default_name(&relative, format.category, &identity);
            let custom_name = self
                .index
                .records
                .get(&id)
                .and_then(|record| record.name.clone());
            let asset = Asset {
                id: id.clone(),
                name: custom_name.as_ref().unwrap_or(&default_name).clone(),
                default_name,
                custom_name,
                source_asset_id: identity.source_id,
                path: display_path(&relative),
                kind: format.kind,
                category: format.category,
                mime: format.mime,
                size: body.len() as u64,
                modified: 0,
                checksum: checksum(&response),
                changed: self.index.records.get(&id).is_some_and(record_changed),
                preview_only: !matches!(format.kind, "animation" | "image" | "audio")
                    || format.mime == KTX2_MIME,
                database_entry: true,
                cache_entry: true,
            };
            seen.insert(
                id,
                SeenAsset {
                    relative,
                    asset,
                    stamp: None,
                    inline_record: Some(response.into()),
                },
            );
        }
    }

    fn snapshot(
        &self,
        id: &str,
        relative: &Path,
        path: &Path,
        metadata: Metadata,
    ) -> CacheResult<Option<Asset>> {
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ASSET_SIZE as u64 {
            return Ok(None);
        }
        let mut file = open_asset(path).map_err(io_message("Cannot open the asset"))?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(HEADER_SAMPLE_SIZE as u64)
            .read_to_end(&mut bytes)
            .map_err(io_message("Cannot inspect the asset format"))?;
        let wrapped = bytes.starts_with(HTTP_CACHE_MAGIC);
        let payload = if wrapped {
            let Some(payload) = http_cache_payload(&bytes, metadata.len()) else {
                return Ok(None);
            };
            payload
        } else {
            &bytes
        };
        let document = looks_like_roblox_document(payload);
        let format = if document { None } else { classify(payload) };
        let audio_tag_may_continue =
            audio::id3_header_size(payload).is_some_and(|offset| offset < metadata.len() as usize);
        if format.is_none() && !document && !audio_tag_may_continue {
            return Ok(None);
        }
        let mut identity = Identity::default();
        let remaining = MAX_ASSET_SIZE + 1 - bytes.len();
        file.take(remaining as u64)
            .read_to_end(&mut bytes)
            .map_err(io_message("Cannot read the asset"))?;
        if bytes.len() > MAX_ASSET_SIZE {
            return Ok(None);
        }
        let payload = if wrapped {
            let Some(payload) = http_cache_payload(&bytes, bytes.len() as u64) else {
                return Ok(None);
            };
            payload
        } else {
            &bytes
        };
        let Some(format) = format.or_else(|| classify_with_names(payload, Some(&mut identity)))
        else {
            return Ok(None);
        };
        if !document {
            identity.name = asset_names::media_name(payload, format.mime);
        }
        let record = self.index.records.get(id);
        let custom_name = record.and_then(|record| record.name.clone());
        let default_name = default_name(relative, format.category, &identity);
        Ok(Some(Asset {
            id: id.to_owned(),
            name: custom_name.as_ref().unwrap_or(&default_name).clone(),
            default_name,
            custom_name,
            source_asset_id: identity.source_id,
            path: display_path(relative),
            kind: format.kind,
            category: format.category,
            mime: format.mime,
            size: payload.len() as u64,
            modified: epoch_ms(metadata.modified().unwrap_or(UNIX_EPOCH)),
            checksum: checksum(&bytes),
            changed: record.is_some_and(record_changed),
            preview_only: format.mime == KTX2_MIME || matches!(format.kind, "reference" | "mesh"),
            database_entry: false,
            cache_entry: wrapped,
        }))
    }

    fn seen_asset(&self, id: &str) -> CacheResult<&SeenAsset> {
        self.seen.get(id).ok_or_else(|| {
            "This asset is no longer in the selected cache. Scan the folder again.".into()
        })
    }

    fn refresh_seen(&mut self, id: &str, relative: PathBuf) -> CacheResult<()> {
        if database_key(&relative).is_some() {
            let mut next = BTreeMap::new();
            self.add_database_assets(&mut next, false);
            self.seen.remove(id);
            self.seen.extend(next);
            return Ok(());
        }
        let path = self.target(&relative)?;
        let Some(metadata) = target_metadata(&path)? else {
            self.seen.remove(id);
            return Ok(());
        };
        let stamp = file_stamp(&metadata);
        if let Some(asset) = self.snapshot(id, &relative, &path, metadata)? {
            self.seen.insert(
                id.to_owned(),
                SeenAsset {
                    relative,
                    asset,
                    stamp,
                    inline_record: None,
                },
            );
        } else {
            self.seen.remove(id);
        }
        Ok(())
    }

    fn read_current(&self, relative: &Path) -> CacheResult<Option<Vec<u8>>> {
        validate_relative(relative)?;
        if let Some(key) = database_key(relative) {
            return sqlite_cache::read_response(&self.root, key);
        }
        let path = self.target(relative)?;
        target_metadata(&path)?
            .map(|_| read_asset(&path))
            .transpose()
    }

    fn current_hash(&self, relative: &Path) -> CacheResult<Option<String>> {
        Ok(self.read_current(relative)?.map(|bytes| checksum(&bytes)))
    }

    fn target(&self, relative: &Path) -> CacheResult<PathBuf> {
        validate_relative(relative)?;
        for ancestor in self.root.ancestors() {
            plain_metadata(ancestor, true)
                .map_err(io_message("The cache folder is no longer accessible"))?;
        }
        let mut path = self.root.clone();
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            path.push(component);
            if components.peek().is_some() {
                plain_metadata(&path, true)
                    .map_err(io_message("The asset's parent folder is unavailable"))?;
            }
        }
        target_metadata(&path)?;
        Ok(path)
    }

    fn update_index(&mut self, update: impl FnOnce(&mut Index)) -> CacheResult<()> {
        let mut next = self.index.clone();
        update(&mut next);
        let bytes = serde_json::to_vec(&next)
            .map_err(|error| format!("Cannot encode the backup index: {error}"))?;
        if bytes.len() > MAX_STATE_SIZE {
            return Err("The backup index exceeds the supported size.".into());
        }
        let temporary = staged_file(&self.storage, &bytes)?;
        persist(temporary, &self.storage.join("index.json"))?;
        self.index = next;
        Ok(())
    }

    fn history(&self, id: &str) -> Option<&History> {
        self.index.records.get(id)?.history.as_ref()
    }

    fn backup_path(&self, id: &str) -> PathBuf {
        self.storage.join(format!("{id}.original"))
    }

    fn ensure_backup(&self, id: &str, before: &[u8], before_hash: &str) -> CacheResult<String> {
        let backup_path = self.backup_path(id);
        if let Some(history) = self.history(id) {
            self.verify_backup(id, &history.original)?;
            return Ok(history.original.clone());
        }
        if target_metadata(&backup_path)?.is_none() {
            let temporary = staged_file(&self.storage, before)?;
            temporary.persist_noclobber(&backup_path).map_err(|error| {
                format!("Cannot save the first original backup: {}", error.error)
            })?;
        }
        self.verify_backup(id, before_hash)?;
        Ok(before_hash.to_owned())
    }

    fn verify_backup(&self, id: &str, hash: &str) -> CacheResult<Vec<u8>> {
        let bytes = read_asset(&self.backup_path(id))
            .map_err(|error| format!("Cannot read the first original backup. {error}"))?;
        if checksum(&bytes) != hash {
            return Err("The first original backup failed checksum verification.".into());
        }
        Ok(bytes)
    }

    fn write_change(
        &mut self,
        id: &str,
        relative: &Path,
        original: &str,
        before: Option<String>,
        after: &str,
        bytes: &[u8],
    ) -> CacheResult<()> {
        let temporary = if database_key(relative).is_some() {
            None
        } else {
            let target = self.target(relative)?;
            let parent = target.parent().ok_or("The asset has no parent folder.")?;
            Some(staged_file(parent, bytes)?)
        };
        let changed_at = epoch_ms(SystemTime::now());
        self.update_index(|index| {
            let record = index.record(id, relative);
            let history = record.history.get_or_insert_with(|| History {
                original: original.to_owned(),
                current: original.to_owned(),
                changed_at,
                pending: None,
            });
            history.pending = Some(Pending {
                before: before.clone(),
                after: after.to_owned(),
                changed_at,
            });
        })?;
        if let Some(key) = database_key(relative) {
            sqlite_cache::write_response(&self.root, key, before.as_deref(), bytes)?;
        } else {
            let target = self.target(relative)?;
            if current_hash(&target)? != before {
                return Err(
                    "The asset changed before it could be written. The backup is intact.".into(),
                );
            }
            let temporary = temporary.expect("file writes have a staged file");
            if before.is_none() {
                temporary.persist_noclobber(&target).map_err(|error| {
                    format!("Cannot restore the missing asset: {}", error.error)
                })?;
            } else {
                persist(temporary, &target)?;
            }
            sqlite_cache::sync_external_size(&self.root, relative, bytes.len())?;
        }
        if self.current_hash(relative)?.as_deref() != Some(after) {
            return Err("The saved cache entry failed verification. The backup is intact.".into());
        }
        self.reconcile(id)?;
        self.refresh_seen(id, relative.to_path_buf())
    }

    fn reconcile(&mut self, id: &str) -> CacheResult<()> {
        let Some(record) = self.index.records.get(id) else {
            return Ok(());
        };
        let Some(pending) = record
            .history
            .as_ref()
            .and_then(|history| history.pending.clone())
        else {
            return Ok(());
        };
        let actual = self.current_hash(&record.relative)?;
        if actual.as_deref() == Some(pending.after.as_str())
            && database_key(&record.relative).is_none()
        {
            if let Some(bytes) = self.read_current(&record.relative)? {
                sqlite_cache::sync_external_size(&self.root, &record.relative, bytes.len())?;
            }
        }
        let committed = actual.as_deref() == Some(pending.after.as_str());
        if !committed && actual != pending.before {
            return Err(
                "The file changed outside the app during a write. The backup is intact.".into(),
            );
        }
        self.update_index(|index| {
            if let Some(history) = index
                .records
                .get_mut(id)
                .and_then(|record| record.history.as_mut())
            {
                if committed {
                    history.current = pending.after;
                    history.changed_at = pending.changed_at;
                }
                history.pending = None;
            }
        })
    }

    fn restore(&mut self, id: &str) -> CacheResult<()> {
        ensure_player_closed(&self.root)?;
        let record = self
            .index
            .records
            .get(id)
            .ok_or("There is no original backup for this asset.")?;
        let history = record
            .history
            .as_ref()
            .ok_or("There is no original backup for this asset.")?;
        let relative = record.relative.clone();
        let original = history.original.clone();
        let current = self.read_current(&relative)?;
        if database_key(&relative).is_some()
            || current
                .as_ref()
                .is_some_and(|bytes| bytes.starts_with(HTTP_CACHE_MAGIC))
        {
            ensure_player_closed(&self.root)?;
        }
        let before = current.as_ref().map(|bytes| checksum(bytes));
        if before.as_deref() == Some(original.as_str()) {
            self.update_index(|index| {
                if let Some(history) = index
                    .records
                    .get_mut(id)
                    .and_then(|record| record.history.as_mut())
                {
                    history.current = original;
                    history.pending = None;
                }
            })?;
            return self.refresh_seen(id, relative);
        }
        if let Some(before) = &before {
            self.reconcile(id)?;
            if self
                .history(id)
                .is_none_or(|history| before != &history.current)
            {
                return Err("The file was modified outside the app. The backup is intact.".into());
            }
        }
        let bytes = self.verify_backup(id, &original)?;
        self.write_change(id, &relative, &original, before, &original, &bytes)
    }
}

fn record_changed(record: &Record) -> bool {
    record
        .history
        .as_ref()
        .is_some_and(|history| history.original != history.current || history.pending.is_some())
}

fn http_cache_payload(bytes: &[u8], file_len: u64) -> Option<&[u8]> {
    rbxh::parse(bytes, file_len).map(|record| record.body)
}

fn database_key(relative: &Path) -> Option<&str> {
    let mut parts = relative.components();
    if parts.next()?.as_os_str() != sqlite_cache::DATABASE_NAME {
        return None;
    }
    let key = parts.next()?.as_os_str().to_str()?;
    if parts.next().is_some() || sqlite_cache::key_bytes(key).is_err() {
        return None;
    }
    Some(key)
}

fn ensure_player_closed(root: &Path) -> CacheResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
            return Ok(());
        };
        let candidates = [
            local.join("Roblox/rbx-storage"),
            local.join("Temp/Roblox/http"),
            local.join("Temp/Roblox/http-wob"),
        ];
        if !candidates
            .iter()
            .any(|path| fs::canonicalize(path).is_ok_and(|path| path == root))
        {
            return Ok(());
        }
        let output = std::process::Command::new("tasklist.exe")
            .args([
                "/FI",
                "IMAGENAME eq RobloxPlayerBeta.exe",
                "/FO",
                "CSV",
                "/NH",
            ])
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|error| format!("Cannot check whether Roblox Player is closed: {error}"))?;
        if !output.status.success() {
            return Err("Cannot check whether Roblox Player is closed.".into());
        }
        if String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains("robloxplayerbeta.exe")
        {
            return Err("Close Roblox Player before applying or restoring cache replacements, then join again.".into());
        }
    }
    #[cfg(not(windows))]
    let _ = root;
    Ok(())
}

fn remove_unlisted_http_file(path: &Path) -> io::Result<bool> {
    let mut file = open_asset(path)?;
    let mut header = [0; HTTP_CACHE_HEADER_SIZE];
    if let Err(error) = file.read_exact(&mut header) {
        return if error.kind() == io::ErrorKind::UnexpectedEof {
            Ok(false)
        } else {
            Err(error)
        };
    }
    let metadata = file.metadata()?;
    let len = u32::from_le_bytes(header[25..29].try_into().unwrap());
    if !header.starts_with(HTTP_CACHE_MAGIC)
        || u64::from(len) + HTTP_CACHE_HEADER_SIZE as u64 != metadata.len()
    {
        return Ok(false);
    }
    let stamp = file_stamp(&metadata);
    drop(file);
    if stamp.is_none() || file_stamp(&plain_metadata(path, false)?) != stamp {
        return Ok(false);
    }
    fs::remove_file(path)?;
    Ok(true)
}

fn file_stamp(metadata: &Metadata) -> Option<(u64, SystemTime)> {
    Some((metadata.len(), metadata.modified().ok()?))
}

fn markup_start(bytes: &[u8]) -> &[u8] {
    bytes
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(bytes)
        .trim_ascii_start()
}

fn looks_like_roblox_document(bytes: &[u8]) -> bool {
    let markup = markup_start(bytes);
    markup.starts_with(b"<roblox") || markup.starts_with(b"<?xml")
}

fn classify(bytes: &[u8]) -> Option<Format> {
    classify_with_names(bytes, None)
}

fn classify_with_names(bytes: &[u8], identity: Option<&mut Identity>) -> Option<Format> {
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        "image/bmp"
    } else if is_supported_ktx2(bytes) {
        return Some(Format {
            kind: "image",
            category: AssetCategory::Texture,
            mime: KTX2_MIME,
        });
    } else if bytes.starts_with(BINARY_MAGIC) {
        let mut names = identity.is_some().then(BinaryNames::default);
        let flavor = binary_model_flavor_with_names(bytes, names.as_mut()).ok()?;
        if let Some((identity, names)) = identity.zip(names) {
            *identity = names.identity();
        }
        return Some(flavor.format(true));
    } else if looks_like_roblox_document(bytes) {
        let xml = markup_start(bytes);
        let flavor = xml_model_flavor(xml)?;
        if let Some(identity) = identity
            && let Some(details) = asset_names::xml_identity(xml)
        {
            *identity = details;
        }
        return Some(flavor.format(false));
    } else if is_roblox_mesh(bytes) {
        return Some(Format {
            kind: "mesh",
            category: AssetCategory::Mesh,
            mime: "application/x-roblox-mesh",
        });
    } else if let Some(mime) = audio::mime(bytes) {
        return Some(Format {
            kind: "audio",
            category: AssetCategory::Audio,
            mime,
        });
    } else {
        return None;
    };
    Some(Format {
        kind: "image",
        category: AssetCategory::Image,
        mime,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ModelFlavor {
    Model,
    Decal,
    Avatar,
    Curve,
    Keyframes,
}

impl ModelFlavor {
    fn from_class(name: &[u8]) -> Self {
        match name {
            b"KeyframeSequence" => Self::Keyframes,
            b"CurveAnimation" => Self::Curve,
            b"Humanoid"
            | b"HumanoidDescription"
            | b"Accessory"
            | b"Hat"
            | b"Shirt"
            | b"Pants"
            | b"ShirtGraphic"
            | b"CharacterMesh"
            | b"BodyPartDescription"
            | b"AccessoryDescription" => Self::Avatar,
            b"Decal" | b"Texture" => Self::Decal,
            _ => Self::Model,
        }
    }

    fn format(self, binary: bool) -> Format {
        let (kind, category) = match self {
            Self::Model => ("reference", AssetCategory::Model),
            Self::Decal => ("reference", AssetCategory::Image),
            Self::Avatar => ("reference", AssetCategory::Avatar),
            Self::Curve => ("reference", AssetCategory::Animation),
            Self::Keyframes => ("animation", AssetCategory::Animation),
        };
        let mime = match (self, binary) {
            (Self::Keyframes, true) => BINARY_ANIMATION_MIME,
            (Self::Keyframes, false) => "application/xml",
            (_, true) => "application/x-roblox-model",
            (_, false) => "application/x-roblox-model+xml",
        };
        Format {
            kind,
            category,
            mime,
        }
    }
}

fn xml_model_flavor(bytes: &[u8]) -> Option<ModelFlavor> {
    use quick_xml::{Reader, events::Event};
    let mut reader = Reader::from_reader(bytes);
    let mut root = false;
    let mut flavor = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(tag) if !root => {
                if tag.local_name().as_ref() != "roblox" {
                    return None;
                }
                root = true;
            }
            Event::Start(tag) | Event::Empty(tag)
                if root && tag.local_name().as_ref() == "Item" =>
            {
                let class = tag.try_get_attribute("class").ok()??;
                let current = ModelFlavor::from_class(class.value.as_bytes());
                flavor =
                    Some(flavor.map_or(current, |previous: ModelFlavor| previous.max(current)));
                if current == ModelFlavor::Keyframes {
                    return flavor;
                }
            }
            Event::DocType(_) => return None,
            Event::End(tag) if root && tag.local_name().as_ref() == "roblox" => return flavor,
            Event::Eof => return None,
            _ => {}
        }
    }
}

fn is_roblox_mesh(bytes: &[u8]) -> bool {
    let Some(newline) = bytes
        .get(..bytes.len().min(24))
        .and_then(|header| header.iter().position(|byte| *byte == b'\n'))
    else {
        return false;
    };
    let header = bytes[..newline].trim_ascii_end();
    let body = &bytes[newline + 1..];
    match header {
        b"version 1.00" | b"version 1.01" => body.first().is_some_and(u8::is_ascii_digit),
        b"version 2.00" | b"version 3.00" | b"version 3.01" | b"version 4.00" | b"version 4.01"
        | b"version 5.00" | b"version 6.00" => body.len() >= 12,
        b"version 7.00" => body.starts_with(b"CORE"),
        _ => false,
    }
}

/// Check the model container without handing arbitrary cache files to a full model decoder.
pub fn validate_binary_animation(bytes: &[u8]) -> CacheResult<()> {
    if matches!(binary_model_flavor(bytes), Ok(ModelFlavor::Keyframes)) {
        Ok(())
    } else {
        Err("Unsupported or malformed Roblox binary animation.".into())
    }
}

pub(crate) fn validate_binary_model(bytes: &[u8]) -> CacheResult<()> {
    binary_model_flavor(bytes).map(|_| ())
}

fn binary_model_flavor(bytes: &[u8]) -> CacheResult<ModelFlavor> {
    binary_model_flavor_with_names(bytes, None)
}

fn binary_chunk(bytes: &[u8], size: usize, compressed: bool) -> Option<std::borrow::Cow<'_, [u8]>> {
    use std::borrow::Cow;
    if !compressed {
        return Some(Cow::Borrowed(bytes));
    }
    let chunk = if bytes.starts_with(b"\x28\xb5\x2f\xfd") {
        zstd::bulk::decompress(bytes, size).ok()?
    } else {
        lz4_flex::block::decompress(bytes, size).ok()?
    };
    (chunk.len() == size).then_some(Cow::Owned(chunk))
}

fn binary_model_flavor_with_names(
    bytes: &[u8],
    mut names: Option<&mut BinaryNames>,
) -> CacheResult<ModelFlavor> {
    const INVALID: &str = "Unsupported or malformed Roblox binary model.";
    if bytes.len() < 32 || bytes.len() > MAX_ASSET_SIZE || !bytes.starts_with(BINARY_MAGIC) {
        return Err(INVALID.into());
    }
    let class_count = read_u32(bytes, 16).ok_or(INVALID)?;
    let instance_count = read_u32(bytes, 20).ok_or(INVALID)?;
    if bytes[14..16] != [0, 0]
        || class_count == 0
        || class_count > MAX_BINARY_CLASSES
        || instance_count == 0
        || instance_count > MAX_BINARY_INSTANCES
    {
        return Err(INVALID.into());
    }
    let mut data = &bytes[32..];
    let mut classes = 0;
    let mut instances = 0;
    let mut expanded = 0;
    let mut flavor = ModelFlavor::Model;
    for _ in 0..MAX_BINARY_CHUNKS {
        let header = data.get(..16).ok_or(INVALID)?;
        let length = read_u32(header, 4).ok_or(INVALID)?;
        let unpacked = read_u32(header, 8).ok_or(INVALID)?;
        if unpacked > MAX_BINARY_CHUNK || length > MAX_BINARY_CHUNK {
            return Err(INVALID.into());
        }
        expanded += unpacked;
        if expanded > MAX_BINARY_EXPANDED {
            return Err(INVALID.into());
        }
        let payload_end = 16 + if length == 0 { unpacked } else { length };
        let payload = data.get(16..payload_end).ok_or(INVALID)?;
        data = &data[payload_end..];
        match &header[..4] {
            b"INST" => {
                if unpacked > MAX_BINARY_CLASS_CHUNK {
                    return Err(INVALID.into());
                }
                let payload = binary_chunk(payload, unpacked, length != 0).ok_or(INVALID)?;
                let name_length = read_u32(&payload, 4).ok_or(INVALID)?;
                if name_length > 128 {
                    return Err(INVALID.into());
                }
                let name_end = 8 + name_length;
                let name = payload.get(8..name_end).ok_or(INVALID)?;
                let count = read_u32(&payload, name_end + 1).ok_or(INVALID)?;
                instances += count;
                classes += 1;
                if classes > class_count
                    || instances > instance_count
                    || (name_end + 5) + count * 4 > payload.len()
                {
                    return Err(INVALID.into());
                }
                if count > 0 {
                    flavor = flavor.max(ModelFlavor::from_class(name));
                }
                if let Some(names) = names.as_deref_mut() {
                    let _ = names.read_class(&payload);
                }
            }
            b"PROP" if unpacked <= MAX_BINARY_CLASS_CHUNK => {
                if let Some(names) = names.as_deref_mut()
                    && let Some(payload) = binary_chunk(payload, unpacked, length != 0)
                {
                    let _ = names.read_property(&payload);
                }
            }
            b"PRNT" => {
                if let Some(names) = names.as_deref_mut()
                    && unpacked == 5 + instance_count * 8
                    && let Some(payload) = binary_chunk(payload, unpacked, length != 0)
                {
                    let _ = names.read_parents(&payload, instance_count);
                }
            }
            b"END\0" => {
                if classes == class_count && instances == instance_count {
                    return Ok(flavor);
                }
                return Err(INVALID.into());
            }
            _ => {}
        }
    }
    Err(INVALID.into())
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<usize> {
    let value = bytes.get(offset..offset + 4)?.try_into().ok()?;
    usize::try_from(u32::from_le_bytes(value)).ok()
}

fn extension_mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpeg" | "jpg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "ogg" | "oga" | "opus" => Some("audio/ogg"),
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        "flac" => Some("audio/flac"),
        "rbxmx" => Some("application/xml"),
        "rbxm" => Some(BINARY_ANIMATION_MIME),
        _ => None,
    }
}

pub fn asset_extension(bytes: &[u8]) -> Option<&'static str> {
    match classify(bytes)?.mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" => Some("bmp"),
        "audio/ogg" => Some("ogg"),
        "audio/mpeg" => Some("mp3"),
        "audio/wav" => Some("wav"),
        "audio/flac" => Some("flac"),
        "application/xml" => Some("rbxmx"),
        BINARY_ANIMATION_MIME => Some("rbxm"),
        _ => None,
    }
}

fn display_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn file_name(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name().unwrap_or_default().to_string_lossy()
}

fn default_name(path: &Path, category: AssetCategory, identity: &Identity) -> String {
    if let Some(name) = &identity.name {
        return name.clone();
    }
    let kind = match category {
        AssetCategory::Image => "Image",
        AssetCategory::Texture => "Texture",
        AssetCategory::Animation => "Animation",
        AssetCategory::Avatar => "Avatar asset",
        AssetCategory::Model => "Model",
        AssetCategory::Mesh => "Mesh",
        AssetCategory::Audio => "Audio",
    };
    if let Some(source_id) = &identity.source_id {
        return format!("{kind} {source_id}");
    }
    if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        && matches!(stem.len(), 32 | 40 | 64)
        && stem.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return format!("{kind} · {}", &stem[..8]);
    }
    file_name(path).into_owned()
}

fn path_hash(path: &Path) -> String {
    checksum(path.as_os_str().as_encoded_bytes())
}

fn asset_id(root: &Path, relative: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(root.as_os_str().as_encoded_bytes());
    digest.update([0]);
    digest.update(relative.as_os_str().as_encoded_bytes());
    format!("{:x}", digest.finalize())
}

fn checksum(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn epoch_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |duration| {
        duration.as_millis().min(u64::MAX as u128) as u64
    })
}

fn validate_relative(path: &Path) -> CacheResult<()> {
    if path.as_os_str().is_empty()
        || !path.components().all(|part| match part {
            Component::Normal(name) => !name.as_encoded_bytes().contains(&b':'),
            _ => false,
        })
    {
        return Err("The asset path is outside the selected cache.".into());
    }
    Ok(())
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(crate) fn plain_metadata(path: &Path, directory: bool) -> io::Result<Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if is_link(&metadata)
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path is a link or has an unexpected type",
        ));
    }
    Ok(metadata)
}

fn target_metadata(path: &Path) -> CacheResult<Option<Metadata>> {
    match plain_metadata(path, false) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot access the asset file: {error}")),
    }
}

fn read_asset(path: &Path) -> CacheResult<Vec<u8>> {
    plain_metadata(path, false).map_err(io_message("Cannot open the asset"))?;
    let file = open_asset(path).map_err(io_message("Cannot open the asset"))?;
    read_limited(file, MAX_ASSET_SIZE).map_err(io_message("Cannot read the asset"))
}

fn open_asset(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if is_link(&metadata) || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path is not a regular file",
        ));
    }
    Ok(file)
}

fn read_limited(file: File, limit: usize) -> io::Result<Vec<u8>> {
    if file.metadata()?.len() > limit as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds the size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds the size limit",
        ));
    }
    Ok(bytes)
}

fn current_hash(path: &Path) -> CacheResult<Option<String>> {
    target_metadata(path)?
        .map(|_| read_asset(path).map(|bytes| checksum(&bytes)))
        .transpose()
}

fn staged_file(directory: &Path, bytes: &[u8]) -> CacheResult<NamedTempFile> {
    let mut temporary =
        NamedTempFile::new_in(directory).map_err(io_message("Cannot stage the file"))?;
    temporary
        .write_all(bytes)
        .map_err(io_message("Cannot write the staged file"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(io_message("Cannot flush the staged file"))?;
    Ok(temporary)
}

fn persist(temporary: NamedTempFile, target: &Path) -> CacheResult<()> {
    temporary
        .persist(target)
        .map_err(|error| format!("Cannot move the staged file into place: {}", error.error))?;
    Ok(())
}

fn io_message(action: &str) -> impl FnOnce(io::Error) -> String + '_ {
    move |error| format!("{action}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &[u8] = b"\x89PNG\r\n\x1a\nfirst";
    const REPLACEMENT: &[u8] = b"\x89PNG\r\n\x1a\nsecond";
    const LAST_REPLACEMENT: &[u8] = b"\x89PNG\r\n\x1a\nthird";
    const ANIMATION: &[u8] = b"<roblox version=\"4\"><Item class=\"KeyframeSequence\" /></roblox>";

    #[test]
    fn finds_live_database_assets_and_clears_cached_http_responses() {
        let roblox = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let folder = roblox.path().join("rbx-storage");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("loose.rbxmx"), ANIMATION).unwrap();
        let writer =
            rusqlite::Connection::open(roblox.path().join(sqlite_cache::DATABASE_NAME)).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL;
             CREATE TABLE files (id BLOB PRIMARY KEY, content BLOB, category INTEGER NOT NULL);",
            )
            .unwrap();
        let response = |body: &[u8], status: u32| {
            let mut bytes = vec![0; HTTP_CACHE_HEADER_SIZE];
            bytes[..HTTP_CACHE_MAGIC.len()].copy_from_slice(HTTP_CACHE_MAGIC);
            bytes[13..17].copy_from_slice(&status.to_le_bytes());
            bytes[25..29].copy_from_slice(&(body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(body);
            bytes
        };
        let insert = |key: u8, body: &[u8], status: u32| {
            writer
                .execute(
                    "INSERT INTO files VALUES (?1, ?2, ?3)",
                    rusqlite::params![
                        &[key; 16][..],
                        response(body, status),
                        if status == 200 { 10 } else { 11 }
                    ],
                )
                .unwrap();
        };
        let named = br#"<roblox version="4"><Item class="KeyframeSequence"><Properties>
            <string name="Name">Inline Walk</string></Properties></Item></roblox>"#;
        insert(1, named, 200);
        insert(2, b"<roblox><Item class=\"Accessory\" /></roblox>", 200);
        insert(3, ANIMATION, 301);
        let mut cache = Cache::new(app_data.path().into());
        let list = cache.scan(folder.clone()).unwrap();
        assert_eq!(list.len(), 2);
        let inline = list.iter().find(|asset| asset.database_entry).unwrap();
        assert_eq!(inline.name, "Inline Walk");
        assert_eq!(inline.category, AssetCategory::Animation);
        assert!(!inline.preview_only);
        assert_eq!(cache.preview(&inline.id).unwrap(), named);

        cache.rename(&inline.id, "Renamed clip".into()).unwrap();
        insert(
            4,
            b"<roblox><Item class=\"CurveAnimation\" /></roblox>",
            200,
        );
        insert(5, b"version 1.01\n1\n", 200);
        let refreshed = cache.scan_incremental(folder.clone()).unwrap();
        assert_eq!(refreshed.len(), 4);
        let mesh = refreshed.iter().find(|asset| asset.kind == "mesh").unwrap();
        assert!(mesh.database_entry && mesh.preview_only);
        assert_eq!(cache.preview(&mesh.id).unwrap(), b"version 1.01\n1\n");
        assert_eq!(cache.asset(&inline.id).unwrap().name, "Renamed clip");
        let updated = br#"<roblox version="4"><Item class="KeyframeSequence"><Properties>
            <string name="Name">Inline Jump</string></Properties></Item></roblox>"#;
        assert_eq!(named.len(), updated.len());
        writer
            .execute(
                "UPDATE files SET content = ?1 WHERE id = ?2",
                rusqlite::params![response(updated, 200), &[1u8; 16][..]],
            )
            .unwrap();
        cache.scan_incremental(folder.clone()).unwrap();
        let changed = cache.asset(&inline.id).unwrap();
        assert_eq!(changed.default_name, "Inline Jump");
        assert_eq!(changed.name, "Renamed clip");
        assert_eq!(cache.preview(&inline.id).unwrap(), updated);
        writer
            .execute(
                "INSERT INTO files VALUES (?1, ?2, 22)",
                rusqlite::params![&[6u8; 16][..], b"internal bookkeeping".as_slice()],
            )
            .unwrap();
        writer
            .execute("INSERT INTO files VALUES (?1, NULL, 10)", [&[7u8; 16][..]])
            .unwrap();
        let shard = folder.join("aa");
        fs::create_dir(&shard).unwrap();
        let hidden_file = shard.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let unrelated_file = shard.join("aabbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        fs::write(&hidden_file, response(b"unlisted response", 302)).unwrap();
        fs::write(&unrelated_file, b"non-cache file").unwrap();
        let report = cache.clear_scanned(&folder).unwrap();
        let remaining = cache.scan(folder.clone()).unwrap();
        assert_eq!(report.removed, 2);
        assert_eq!(report.database_removed, 5);
        assert!(report.database_error.is_none());
        assert!(remaining.is_empty());
        assert!(!hidden_file.exists());
        assert!(unrelated_file.exists());
        assert_eq!(
            writer
                .query_row("SELECT count(*) FROM files", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn reports_real_scan_totals_and_clears_only_unchanged_scanned_assets() {
        let folder = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        for file in ["ordinary.png", "edited.png", "refreshed.png"] {
            fs::write(folder.path().join(file), ORIGINAL).unwrap();
        }
        let index = folder.path().join("index.sqlite3");
        fs::write(&index, b"cache bookkeeping").unwrap();
        let mut cache = Cache::new(storage.path().into());
        let mut progress = Vec::new();
        let assets = cache
            .scan_with_progress(folder.path().into(), |update| progress.push(update))
            .unwrap();
        assert_eq!(assets.len(), 3);
        assert_eq!(progress.first().unwrap().total, None);
        assert_eq!(progress.last().unwrap().total, Some(4));
        assert_eq!(progress.last().unwrap().checked, 4);

        let edited = &assets
            .iter()
            .find(|asset| asset.path == "edited.png")
            .unwrap()
            .id;
        cache.replace(edited, REPLACEMENT).unwrap();
        let refreshed = folder.path().join("refreshed.png");
        fs::write(&refreshed, REPLACEMENT).unwrap();
        let new_asset = folder.path().join("new.png");
        fs::write(&new_asset, ORIGINAL).unwrap();
        assert!(cache.clear_scanned(other.path()).is_err());

        let report = cache.clear_scanned(folder.path()).unwrap();
        let remaining = cache.scan(folder.path().into()).unwrap();
        assert_eq!((report.removed, report.kept, report.failed), (1, 2, 0));
        assert_eq!(remaining.len(), 3);
        assert!(remaining.iter().any(|asset| asset.path == "new.png"));
        assert!(!folder.path().join("ordinary.png").exists());
        assert!(index.is_file());
        assert!(new_asset.is_file());
        assert_eq!(fs::read(refreshed).unwrap(), REPLACEMENT);
        assert_eq!(cache.preview(edited).unwrap(), REPLACEMENT);
        cache.restore(edited).unwrap();
        assert_eq!(cache.preview(edited).unwrap(), ORIGINAL);
    }

    #[test]
    fn detected_names_and_source_ids_survive_custom_name_reset() {
        let folder = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let file = folder.path().join("abcd0123abcd0123abcd0123abcd0123");
        fs::write(
            &file,
            br#"<roblox version="4"><Item class="KeyframeSequence"><Properties>
            <string name="Name">Walk &amp; Run</string>
            <int64 name="SourceAssetId">9007199254740993</int64>
            </Properties><Item class="Keyframe"><Properties><string name="Name">Frame</string>
            </Properties></Item></Item></roblox>"#,
        )
        .unwrap();
        let mut cache = Cache::new(app_data.path().into());
        let asset = cache.scan(folder.path().into()).unwrap().pop().unwrap();
        assert_eq!(asset.name, "Walk & Run");
        assert_eq!(asset.source_asset_id.as_deref(), Some("9007199254740993"));
        assert_eq!(asset.path, "abcd0123abcd0123abcd0123abcd0123");
        cache.rename(&asset.id, "My clip".into()).unwrap();
        assert_eq!(cache.asset(&asset.id).unwrap().name, "My clip");
        cache.rename(&asset.id, String::new()).unwrap();
        assert_eq!(cache.asset(&asset.id).unwrap().name, "Walk & Run");

        fs::write(file, ANIMATION).unwrap();
        let unnamed = cache.scan(folder.path().into()).unwrap().pop().unwrap();
        assert_eq!(unnamed.name, "Animation · abcd0123");
        assert_eq!(unnamed.source_asset_id, None);
    }

    #[test]
    fn replaces_a_wrapped_asset_and_restores_the_exact_original() {
        let folder = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let mut wrapped = vec![0; HTTP_CACHE_HEADER_SIZE];
        wrapped[..HTTP_CACHE_MAGIC.len()].copy_from_slice(HTTP_CACHE_MAGIC);
        wrapped[13..17].copy_from_slice(&200_u32.to_le_bytes());
        wrapped[25..29].copy_from_slice(&(ORIGINAL.len() as u32).to_le_bytes());
        wrapped.extend_from_slice(ORIGINAL);
        let path = folder.path().join("cached-image");
        fs::write(&path, &wrapped).unwrap();
        fs::write(
            folder.path().join("truncated"),
            &wrapped[..wrapped.len() - 1],
        )
        .unwrap();

        let mut cache = Cache::new(storage.path().into());
        let assets = cache.scan(folder.path().into()).unwrap();
        assert_eq!(assets.len(), 1);
        let asset = &assets[0];
        assert!(!asset.preview_only);
        assert!(asset.cache_entry);
        assert_eq!(asset.size, ORIGINAL.len() as u64);
        assert_eq!(cache.preview(&asset.id).unwrap(), ORIGINAL);
        cache.replace(&asset.id, REPLACEMENT).unwrap();
        let updated = fs::read(&path).unwrap();
        assert_eq!(
            http_cache_payload(&updated, updated.len() as u64).unwrap(),
            REPLACEMENT
        );
        assert_eq!(cache.preview(&asset.id).unwrap(), REPLACEMENT);
        assert_eq!(cache.changes().unwrap().len(), 1);
        cache.restore(&asset.id).unwrap();
        assert_eq!(fs::read(path).unwrap(), wrapped);
        assert!(cache.changes().unwrap().is_empty());
    }

    #[test]
    fn repeated_replacement_keeps_first_original_across_restarts() {
        let cache_folder = tempfile::tempdir().unwrap();
        let data_folder = tempfile::tempdir().unwrap();
        let path = cache_folder.path().join("cached");
        fs::write(&path, ORIGINAL).unwrap();
        let mut cache = Cache::new(data_folder.path().into());
        let id = cache
            .scan(cache_folder.path().into())
            .unwrap()
            .pop()
            .unwrap()
            .id;
        cache.rename(&id, "First asset".into()).unwrap();
        cache.replace(&id, REPLACEMENT).unwrap();
        cache.replace(&id, LAST_REPLACEMENT).unwrap();
        assert_eq!(cache.preview(&id).unwrap(), LAST_REPLACEMENT);
        let changes = cache.changes().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].original_checksum, checksum(ORIGINAL));
        assert_eq!(changes[0].current_checksum, checksum(LAST_REPLACEMENT));
        drop(cache);

        let mut reopened = Cache::new(data_folder.path().into());
        let asset = reopened
            .scan(cache_folder.path().into())
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(asset.id, id);
        assert_eq!(asset.name, "First asset");
        assert!(asset.changed);
        fs::remove_file(&path).unwrap();
        assert_eq!(reopened.changes().unwrap().len(), 1);
        reopened.restore_all().unwrap();
        assert_eq!(fs::read(path).unwrap(), ORIGINAL);
        assert_eq!(reopened.preview(&id).unwrap(), ORIGINAL);
        assert!(reopened.changes().unwrap().is_empty());
    }

    #[test]
    fn refuses_replacements_of_a_different_kind_and_conflicting_restores() {
        let cache_folder = tempfile::tempdir().unwrap();
        let data_folder = tempfile::tempdir().unwrap();
        let path = cache_folder.path().join("cached");
        fs::write(&path, ORIGINAL).unwrap();
        let mut cache = Cache::new(data_folder.path().into());
        let id = cache
            .scan(cache_folder.path().into())
            .unwrap()
            .pop()
            .unwrap()
            .id;
        assert!(cache.replace(&id, ANIMATION).is_err());
        assert!(cache.replace("../cached", REPLACEMENT).is_err());
        cache.replace(&id, REPLACEMENT).unwrap();
        fs::write(&path, LAST_REPLACEMENT).unwrap();
        assert!(cache.restore(&id).is_err());
        assert_eq!(fs::read(&path).unwrap(), LAST_REPLACEMENT);
        assert_eq!(cache.changes().unwrap().len(), 1);

        let second_folder = tempfile::tempdir().unwrap();
        fs::write(second_folder.path().join("cached"), ORIGINAL).unwrap();
        let other = cache
            .scan(second_folder.path().into())
            .unwrap()
            .pop()
            .unwrap();
        assert_ne!(other.id, id);
        assert!(cache.replace(&id, REPLACEMENT).is_err());
    }

    #[test]
    fn reconciles_a_committed_write_after_reopening() {
        let cache_folder = tempfile::tempdir().unwrap();
        let data_folder = tempfile::tempdir().unwrap();
        let path = cache_folder.path().join("cached");
        fs::write(&path, ORIGINAL).unwrap();
        let mut cache = Cache::new(data_folder.path().into());
        let id = cache
            .scan(cache_folder.path().into())
            .unwrap()
            .pop()
            .unwrap()
            .id;
        cache.replace(&id, REPLACEMENT).unwrap();
        let after = checksum(LAST_REPLACEMENT);
        cache
            .session_mut()
            .unwrap()
            .update_index(|index| {
                let history = index
                    .records
                    .get_mut(&id)
                    .unwrap()
                    .history
                    .as_mut()
                    .unwrap();
                history.pending = Some(Pending {
                    before: Some(history.current.clone()),
                    after: after.clone(),
                    changed_at: 42,
                });
            })
            .unwrap();
        fs::write(&path, LAST_REPLACEMENT).unwrap();
        drop(cache);

        let mut reopened = Cache::new(data_folder.path().into());
        reopened.scan(cache_folder.path().into()).unwrap();
        let change = reopened.changes().unwrap().pop().unwrap();
        assert_eq!(change.current_checksum, after);
        assert_eq!(change.changed_at, 42);
        assert!(
            reopened
                .session_mut()
                .unwrap()
                .history(&id)
                .unwrap()
                .pending
                .is_none()
        );
        reopened.restore(&id).unwrap();
        assert_eq!(fs::read(path).unwrap(), ORIGINAL);
    }

    #[test]
    fn identifies_a_bounded_binary_animation_class() {
        fn model_with_class(name: &str, count: u32) -> Vec<u8> {
            let mut binary = Vec::from(BINARY_MAGIC);
            binary.extend_from_slice(&0u16.to_le_bytes());
            binary.extend_from_slice(&1u32.to_le_bytes());
            binary.extend_from_slice(&count.to_le_bytes());
            binary.extend_from_slice(&[0; 8]);
            let mut instance = Vec::from(0u32.to_le_bytes());
            instance.extend_from_slice(&(name.len() as u32).to_le_bytes());
            instance.extend_from_slice(name.as_bytes());
            instance.push(0);
            instance.extend_from_slice(&count.to_le_bytes());
            instance.resize(instance.len() + count as usize * 4, 0);
            let compressed = lz4_flex::block::compress(&instance);
            binary.extend_from_slice(b"INST");
            binary.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
            binary.extend_from_slice(&(instance.len() as u32).to_le_bytes());
            binary.extend_from_slice(&[0; 4]);
            binary.extend_from_slice(&compressed);
            binary.extend_from_slice(b"END\0");
            binary.extend_from_slice(&[0; 12]);
            binary
        }

        let mut animation = model_with_class("KeyframeSequence", 28_378);
        assert_eq!(classify(&animation).unwrap().mime, BINARY_ANIMATION_MIME);
        assert!(classify(&model_with_class("KeyframeSequence", 65_537)).is_none());
        assert_eq!(
            classify(&model_with_class("Part", 1)).unwrap().category,
            AssetCategory::Model
        );
        assert_eq!(
            classify(&model_with_class("Accessory", 1))
                .unwrap()
                .category,
            AssetCategory::Avatar
        );
        assert_eq!(
            classify(&model_with_class("CurveAnimation", 1))
                .unwrap()
                .category,
            AssetCategory::Animation
        );
        animation.pop();
        assert!(classify(&animation).is_none());
    }

    #[test]
    fn incremental_scan_finds_new_clips_and_drops_evictions() {
        let folder = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let root = folder.path();
        let evicted = root.join("old");
        let unrecognized = root.join("unrecognized");
        fs::write(&evicted, ORIGINAL).unwrap();
        fs::write(&unrecognized, b"unsupported").unwrap();
        let mut cache = Cache::new(storage.path().into());
        assert_eq!(cache.scan(root.into()).unwrap().len(), 1);
        assert_eq!(cache.scan_incremental(root.into()).unwrap().len(), 1);

        let xml = format!(
            "<roblox><!--{}--><Item class=\"KeyframeSequence\" /></roblox>",
            " ".repeat(HEADER_SAMPLE_SIZE)
        );
        fs::write(root.join("new-animation"), xml).unwrap();
        fs::remove_file(evicted).unwrap();

        let assets = cache.scan_incremental(root.into()).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].kind, "animation");
        assert_eq!(assets[0].name, "new-animation");

        fs::write(&unrecognized, ANIMATION).unwrap();
        let assets = cache.scan_incremental(root.into()).unwrap();
        assert_eq!(assets.len(), 2);
        assert!(assets.iter().any(|asset| asset.name == "unrecognized"));
    }

    #[test]
    fn filters_by_observed_asset_types_without_enabling_reference_writes() {
        let folder = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let mut audio = vec![0; 28];
        audio[..4].copy_from_slice(b"OggS");
        audio[26] = 1;
        audio[27] = 19;
        audio.extend_from_slice(b"OpusHead\x01\x01\0\0\x80\xbb\0\0\0\0\0");
        for (name, payload) in [
            ("image", ORIGINAL),
            ("mesh", b"version 7.00\nCORE001".as_slice()),
            ("audio", audio.as_slice()),
            ("decal", b"<roblox><Item class = 'Decal'/></roblox>".as_slice()),
            ("avatar", b"<roblox><Item class='Model'><Item class='Decal'/><Item class='Humanoid'/></Item></roblox>".as_slice()),
        ] {
            fs::write(folder.path().join(name), payload).unwrap();
        }
        let mut cache = Cache::new(storage.path().into());
        let assets = cache.scan(folder.path().into()).unwrap();
        assert_eq!(assets.len(), 5);
        for (name, category, kind, preview_only) in [
            ("image", AssetCategory::Image, "image", false),
            ("mesh", AssetCategory::Mesh, "mesh", true),
            ("audio", AssetCategory::Audio, "audio", false),
            ("decal", AssetCategory::Image, "reference", true),
            ("avatar", AssetCategory::Avatar, "reference", true),
        ] {
            let asset = assets.iter().find(|asset| asset.name == name).unwrap();
            assert_eq!(asset.category, category);
            assert_eq!(asset.kind, kind);
            assert_eq!(asset.preview_only, preview_only);
            assert_eq!(cache.preview(&asset.id).is_err(), kind == "reference");
            if preview_only {
                assert!(cache.replace(&asset.id, REPLACEMENT).is_err());
            }
        }
    }

    #[test]
    fn previews_and_replaces_tagged_cached_audio_with_its_wrapper() {
        let folder = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let mut mp3 = b"ID3\x04\0\0\0\0\x28\0".to_vec();
        // The ID3 tag extends beyond the 4 KiB scan sample.
        mp3.resize(10 + 5120, 0);
        for _ in 0..2 {
            let start = mp3.len();
            mp3.extend_from_slice(b"\xff\xfb\x90\0");
            mp3.resize(start + 417, 0);
        }
        let mut wrapped = vec![0; HTTP_CACHE_HEADER_SIZE];
        wrapped[..HTTP_CACHE_MAGIC.len()].copy_from_slice(HTTP_CACHE_MAGIC);
        wrapped[13..17].copy_from_slice(&200_u32.to_le_bytes());
        wrapped[25..29].copy_from_slice(&(mp3.len() as u32).to_le_bytes());
        wrapped.extend_from_slice(&mp3);
        let path = folder.path().join("cached-audio");
        fs::write(&path, &wrapped).unwrap();
        let copy_path = folder.path().join("audio.mp3");
        fs::write(&copy_path, &mp3).unwrap();
        let wrong_format = b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0";

        let mut cache = Cache::new(storage.path().into());
        let assets = cache.scan(folder.path().into()).unwrap();
        assert_eq!(assets.len(), 2);
        let cached = assets.iter().find(|asset| asset.cache_entry).unwrap();
        let copy = assets.iter().find(|asset| !asset.cache_entry).unwrap();
        for asset in [cached, copy] {
            assert_eq!(asset.kind, "audio");
            assert_eq!(asset.mime, "audio/mpeg");
            assert_eq!(asset.size, mp3.len() as u64);
            assert_eq!(cache.preview(&asset.id).unwrap(), mp3);
        }
        assert!(!cached.preview_only);
        cache.replace(&cached.id, &mp3).unwrap();
        assert!(cache.replace(&copy.id, wrong_format).is_err());
        assert_eq!(fs::read(&path).unwrap(), wrapped);
        let mut updated = mp3.clone();
        *updated.last_mut().unwrap() = 1;
        cache.replace(&copy.id, &updated).unwrap();
        assert_eq!(cache.preview(&copy.id).unwrap(), updated);
        cache.restore(&copy.id).unwrap();
        assert_eq!(fs::read(&copy_path).unwrap(), mp3);
    }

    fn wrapped_record(body: &[u8]) -> Vec<u8> {
        let url = b"https://example.invalid/cached-animation";
        let headers = b"roblox-assettypeid: 24\r\n";
        let mut record = HTTP_CACHE_MAGIC.to_vec();
        record.extend_from_slice(&(url.len() as u32).to_le_bytes());
        record.extend_from_slice(url);
        record.push(0);
        for value in [
            200,
            headers.len() as u32,
            rbxh::xxh32(headers),
            body.len() as u32,
            rbxh::xxh32(body),
            0,
        ] {
            record.extend_from_slice(&value.to_le_bytes());
        }
        record.extend_from_slice(headers);
        record.extend_from_slice(body);
        record
    }

    fn fixture_database(root: &Path) -> rusqlite::Connection {
        fs::create_dir(root.join(sqlite_cache::STORAGE_FOLDER)).unwrap();
        let connection =
            rusqlite::Connection::open(root.join(sqlite_cache::DATABASE_NAME)).unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE files (id BLOB PRIMARY KEY NOT NULL, content BLOB, size INTEGER DEFAULT 0 NOT NULL,
            hits INTEGER DEFAULT 0 NOT NULL, atime INTEGER DEFAULT 0 NOT NULL, category INTEGER DEFAULT 10 NOT NULL,
            score INTEGER DEFAULT 0 NOT NULL, ttl INTEGER DEFAULT 0 NOT NULL);").unwrap();
        connection
    }

    #[test]
    fn live_inline_replacement_persists_and_restores_first_original_after_reopen() {
        let roblox = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let connection = fixture_database(roblox.path());
        let key = [0x31u8; 16];
        let original = wrapped_record(ANIMATION);
        let unrelated = wrapped_record(ORIGINAL);
        connection
            .execute(
                "INSERT INTO files (id,content,size) VALUES (?1,?2,?3)",
                rusqlite::params![&key[..], &original, original.len() as i64],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO files (id,content,size) VALUES (?1,?2,?3)",
                rusqlite::params![&[0x32u8; 16][..], &unrelated, unrelated.len() as i64],
            )
            .unwrap();
        let folder = roblox.path().join(sqlite_cache::STORAGE_FOLDER);
        let first = br#"<roblox version="4"><Item class="KeyframeSequence"><Properties><string name="Name">First replacement</string></Properties></Item></roblox>"#;
        let last = br#"<roblox version="4"><Item class="KeyframeSequence"><Properties><string name="Name">Last replacement</string></Properties></Item></roblox>"#;
        let mut cache = Cache::new(app_data.path().into());
        let assets = cache.scan(folder.clone()).unwrap();
        let animation = assets
            .iter()
            .find(|asset| asset.kind == "animation")
            .unwrap();
        assert!(!animation.preview_only && animation.database_entry && animation.cache_entry);
        let id = animation.id.clone();
        cache.replace(&id, first).unwrap();
        let read_record = || {
            connection
                .query_row(
                    "SELECT content,size FROM files WHERE id=?1",
                    [&key[..]],
                    |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap()
        };
        let (saved, size) = read_record();
        assert_eq!(
            http_cache_payload(&saved, saved.len() as u64).unwrap(),
            first
        );
        assert_eq!(size as usize, saved.len());
        let offset = 13 + u32::from_le_bytes(saved[8..12].try_into().unwrap()) as usize;
        assert_eq!(
            u32::from_le_bytes(saved[offset + 16..offset + 20].try_into().unwrap()),
            rbxh::xxh32(first)
        );
        assert_eq!(cache.preview(&id).unwrap(), first);
        assert!(cache.asset(&id).unwrap().changed);
        drop(cache);
        let mut cache = Cache::new(app_data.path().into());
        cache.scan(folder.clone()).unwrap();
        assert!(cache.asset(&id).unwrap().changed);
        cache.replace(&id, last).unwrap();
        assert_eq!(cache.preview(&id).unwrap(), last);
        // Clear cache must retain the active inline replacement and its history.
        let report = cache.clear_scanned(&folder).unwrap();
        assert_eq!(report.kept, 1);
        assert_eq!(report.database_removed, 1);
        cache.scan(folder).unwrap();
        assert_eq!(cache.preview(&id).unwrap(), last);
        cache.restore(&id).unwrap();
        assert_eq!(read_record(), (original.clone(), original.len() as i64));
        assert!(cache.changes().unwrap().is_empty());
        assert_eq!(
            connection
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn live_inline_conflicts_do_not_overwrite_a_newer_record() {
        let roblox = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let connection = fixture_database(roblox.path());
        let key = [0x41u8; 16];
        let original = wrapped_record(ANIMATION);
        connection
            .execute(
                "INSERT INTO files(id,content,size) VALUES (?1,?2,?3)",
                rusqlite::params![&key[..], &original, original.len() as i64],
            )
            .unwrap();
        let mut cache = Cache::new(app_data.path().into());
        let asset = cache
            .scan(roblox.path().join(sqlite_cache::STORAGE_FOLDER))
            .unwrap()
            .remove(0);
        let outside = wrapped_record(br#"<roblox><Item class="KeyframeSequence"><Properties><string name="Name">Outside change</string></Properties></Item></roblox>"#);
        connection
            .execute(
                "UPDATE files SET content=?1,size=?2 WHERE id=?3",
                rusqlite::params![&outside, outside.len() as i64, &key[..]],
            )
            .unwrap();
        assert!(cache.preview(&asset.id).is_err());
        assert!(cache.replace(&asset.id, ANIMATION).is_err());
        // A competing write between read and commit is rejected by the database writer too.
        assert!(
            sqlite_cache::write_response(
                &roblox.path().join(sqlite_cache::STORAGE_FOLDER),
                &key.iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
                Some(&checksum(&original)),
                &original
            )
            .is_err()
        );
        assert_eq!(
            connection
                .query_row("SELECT content FROM files WHERE id=?1", [&key[..]], |row| {
                    row.get::<_, Vec<u8>>(0)
                })
                .unwrap(),
            outside
        );
        assert!(cache.changes().unwrap().is_empty());
    }

    #[test]
    fn external_cache_replacement_updates_shard_record_and_database_size() {
        let roblox = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let connection = fixture_database(roblox.path());
        let key = [0xaau8; 16];
        let hex = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let folder = roblox.path().join(sqlite_cache::STORAGE_FOLDER);
        fs::create_dir(folder.join("aa")).unwrap();
        let path = folder.join("aa").join(hex);
        let original = wrapped_record(ORIGINAL);
        fs::write(&path, &original).unwrap();
        connection
            .execute(
                "INSERT INTO files(id,content,size) VALUES (?1,NULL,?2)",
                rusqlite::params![&key[..], original.len() as i64],
            )
            .unwrap();
        let mut cache = Cache::new(app_data.path().into());
        let asset = cache.scan(folder.clone()).unwrap().remove(0);
        assert!(asset.cache_entry && !asset.preview_only);
        cache.replace(&asset.id, REPLACEMENT).unwrap();
        let saved = fs::read(&path).unwrap();
        assert_eq!(
            http_cache_payload(&saved, saved.len() as u64).unwrap(),
            REPLACEMENT
        );
        assert_eq!(
            connection
                .query_row("SELECT size FROM files WHERE id=?1", [&key[..]], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap() as usize,
            saved.len()
        );
        drop(cache);
        let mut cache = Cache::new(app_data.path().into());
        cache.scan(folder).unwrap();
        cache.restore(&asset.id).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            connection
                .query_row("SELECT size FROM files WHERE id=?1", [&key[..]], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap() as usize,
            original.len()
        );
    }
}
