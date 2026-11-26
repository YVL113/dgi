use std::{
    fs,
    io::{self, Write},
    path::Path,
};

pub fn ensure_directory(root: &Path) -> Result<(), String> {
    fs::create_dir_all(root)
        .map_err(|err| format!("Could not create the editing workspace: {err}"))?;
    crate::cache::plain_metadata(root, true)
        .map_err(|err| format!("Could not open the editing workspace: {err}"))?;
    Ok(())
}

pub fn add(root: &Path, label: &str, extension: &str, bytes: &[u8]) -> Result<String, String> {
    ensure_directory(root)?;
    let stem: String = label
        .chars()
        .take(60)
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim_end();
    let stem = if stem.is_empty() { "asset" } else { stem };
    let mut file = tempfile::NamedTempFile::new_in(root)
        .map_err(|err| format!("Could not stage the workspace copy: {err}"))?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|err| format!("Could not write the workspace copy: {err}"))?;

    // A filename prefix also avoids Windows reserved device names.
    for index in 1..=1_000 {
        let name = if index == 1 {
            format!("Edit - {stem}.{extension}")
        } else {
            format!("Edit - {stem} ({index}).{extension}")
        };
        match file.persist_noclobber(root.join(&name)) {
            Ok(_) => return Ok(name),
            Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => file = err.file,
            Err(err) => return Err(format!("Could not save the workspace copy: {}", err.error)),
        }
    }
    Err("Too many workspace copies have the same name.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_name_import_retains_existing_workspace_asset() {
        let root = tempfile::tempdir().unwrap();
        let first = add(root.path(), "../inside", "png", b"first").unwrap();
        let second = add(root.path(), "../inside", "png", b"second").unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(root.path().join(first)).unwrap(), b"first");
        assert_eq!(fs::read(root.path().join(second)).unwrap(), b"second");
    }
}
