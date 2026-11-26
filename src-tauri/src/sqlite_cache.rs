use crate::cache::{MAX_ASSET_SIZE, plain_metadata};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{fmt::Write, io, path::Path, time::Duration};

pub const DATABASE_NAME: &str = "rbx-storage.db";
pub const STORAGE_FOLDER: &str = "rbx-storage";
const MAX_CANDIDATE_BYTES: usize = 64 * 1024 * 1024;

pub fn read_asset_responses(root: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let Some(path) = database_path(root)? else {
        return Ok(Vec::new());
    };
    read_database(&path)
}

fn database_path(root: &Path) -> Result<Option<std::path::PathBuf>, String> {
    if root.file_name().and_then(|name| name.to_str()) != Some(STORAGE_FOLDER) {
        return Ok(None);
    }
    let Some(folder) = root.parent() else {
        return Ok(None);
    };
    let path = folder.join(DATABASE_NAME);
    match plain_metadata(&path, false) {
        Ok(_) => Ok(Some(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn clear_http_responses(root: &Path, protected: &[String]) -> Result<usize, String> {
    let Some(path) = database_path(root)? else {
        return Ok(0);
    };
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_millis(500))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA trusted_schema = OFF;")
        .map_err(|error| error.to_string())?;
    // Categories 10 and 11 contain HTTP bodies and redirects. Other rows are Roblox bookkeeping.
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let keys = {
        let mut statement = transaction
            .prepare(
                "SELECT id FROM files
         WHERE category IN (10, 11) AND typeof(id) = 'blob' AND length(id) = 16
           AND typeof(content) = 'blob' AND length(content) >= 37
           AND substr(content, 1, 8) = X'5242584802000000'",
            )
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    let mut removed = 0;
    for key in keys {
        let hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
        if !protected.contains(&hex) {
            removed += transaction
                .execute("DELETE FROM files WHERE id = ?1", [key])
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(removed)
}

pub fn key_bytes(key: &str) -> Result<Vec<u8>, String> {
    if key.len() != 32 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid cache database key.".into());
    }
    (0..key.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&key[index..index + 2], 16)
                .map_err(|_| "Invalid cache database key.".into())
        })
        .collect()
}

fn open(root: &Path, writable: bool) -> Result<Connection, String> {
    let path =
        database_path(root)?.ok_or("The Roblox cache database is missing. Scan it again.")?;
    for ancestor in path.parent().into_iter().flat_map(Path::ancestors) {
        plain_metadata(ancestor, true).map_err(|error| error.to_string())?;
    }
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let connection = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NOFOLLOW)
        .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA trusted_schema = OFF;")
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

pub fn read_response(root: &Path, key: &str) -> Result<Option<Vec<u8>>, String> {
    let connection = open(root, false)?;
    connection.query_row(
        "SELECT content FROM files WHERE id = ?1 AND typeof(content) = 'blob' AND length(content) <= ?2",
        params![key_bytes(key)?, MAX_ASSET_SIZE as i64], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())
}

pub fn write_response(
    root: &Path,
    key: &str,
    before: Option<&str>,
    record: &[u8],
) -> Result<(), String> {
    let mut connection = open(root, true)?;
    let key = key_bytes(key)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let current: Option<Vec<u8>> = transaction.query_row(
        "SELECT content FROM files WHERE id = ?1 AND typeof(content) = 'blob' AND length(content) <= ?2",
        params![&key, MAX_ASSET_SIZE as i64], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    let current = current
        .ok_or("The cache entry was evicted or moved. Scan the cache again before applying.")?;
    if Some(format!("{:x}", Sha256::digest(&current))).as_deref() != before {
        return Err(
            "The cache entry changed before it could be written. The backup is intact.".into(),
        );
    }
    let changed = transaction
        .execute(
            "UPDATE files SET content = ?1, size = ?2 WHERE id = ?3 AND content = ?4",
            params![record, record.len() as i64, &key, &current],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Expected exactly one cache database update.".into());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    if read_response(
        root,
        &key.iter().map(|b| format!("{b:02x}")).collect::<String>(),
    )?
    .as_deref()
        != Some(record)
    {
        return Err(
            "The committed cache entry did not match the replacement. The backup is intact.".into(),
        );
    }
    Ok(())
}

fn external_key(relative: &Path) -> Option<&str> {
    let mut components = relative.components();
    let shard = components.next()?.as_os_str().to_str()?;
    let key = components.next()?.as_os_str().to_str()?;
    if components.next().is_some()
        || shard.len() != 2
        || !key.starts_with(shard)
        || key_bytes(key).is_err()
    {
        return None;
    }
    Some(key)
}

pub fn check_external_target(root: &Path, relative: &Path) -> Result<(), String> {
    let Some(key) = external_key(relative) else {
        return Ok(());
    };
    if database_path(root)?.is_none() {
        return Ok(());
    }
    let connection = open(root, false)?;
    let external: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1 AND content IS NULL)",
            [key_bytes(key)?],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !external {
        return Err(
            "This file is no longer the active database payload. Scan the cache again.".into(),
        );
    }
    Ok(())
}

/// Keep the database's length synchronized when its payload lives in a shard file.
pub fn sync_external_size(root: &Path, relative: &Path, size: usize) -> Result<(), String> {
    let Some(key) = external_key(relative) else {
        return Ok(());
    };
    if database_path(root)?.is_none() {
        return Ok(());
    }
    let connection = open(root, true)?;
    let changed = connection
        .execute(
            "UPDATE files SET size = ?1 WHERE id = ?2 AND content IS NULL",
            params![size as i64, key_bytes(key)?],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(
            "The external cache entry moved during the write. The backup is intact.".into(),
        );
    }
    Ok(())
}

fn read_database(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_millis(200))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA query_only = ON; PRAGMA trusted_schema = OFF;")
        .map_err(|error| error.to_string())?;
    // Parse the variable-length HTTP record in Rust; URLs and headers shift its offsets.
    let mut query = connection
        .prepare(
            "SELECT id, content FROM files
         WHERE typeof(id) = 'blob' AND length(id) = 16 AND typeof(content) = 'blob'
           AND length(content) BETWEEN 38 AND ?1
           AND substr(content, 1, 8) = X'5242584802000000'",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = query
        .query([MAX_ASSET_SIZE as i64])
        .map_err(|error| error.to_string())?;
    let mut responses = Vec::new();
    let mut size = 0;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let id: Vec<u8> = row.get(0).map_err(|error| error.to_string())?;
        let content: Vec<u8> = row.get(1).map_err(|error| error.to_string())?;
        size += content.len();
        if size > MAX_CANDIDATE_BYTES {
            return Err("The Roblox cache database exceeds the model response limit.".into());
        }
        let mut key = String::with_capacity(id.len() * 2);
        for byte in id {
            let _ = write!(key, "{byte:02x}");
        }
        responses.push((key, content));
    }
    Ok(responses)
}
