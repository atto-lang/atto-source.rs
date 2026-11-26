//! Same-directory publication for linked guest Wasm. Trusted output parent;
//! not a hostile-directory sandbox or a multi-output transaction. Adapted from
//! the existing native CLI publication helper (legacy PR #65).
use std::{fs, io::{Read, Write}, path::Path};

fn canonical_destination(path: &str) -> Result<std::path::PathBuf, String> {
    let destination = Path::new(path);
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let directory = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    let basename = destination
        .file_name()
        .ok_or("output requires a filename")?;
    let resolved = directory.join(basename);
    // Never follow the final output component, even for an existing path.
    // A symlink (including a dangling one) must not redirect compiler writes.
    match fs::symlink_metadata(&resolved) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("refusing symlinked compiler output".into())
        }
        Ok(metadata) if !metadata.is_file() => Err("compiler output must be a regular file".into()),
        Ok(_) => {
            // Only replace a previous version-1 Wasm artifact, never source,
            // a lockfile, a diagnostic receipt or another arbitrary file.
            // This is a format marker, not a full Wasm validity proof.
            let mut header = [0_u8; 8];
            let mut prior = fs::File::open(&resolved).map_err(|e| e.to_string())?;
            if prior.read_exact(&mut header).is_err() || header != *b"\0asm\x01\0\0\0" {
                return Err("refusing to replace non-Wasm output".into());
            }
            Ok(resolved)
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(resolved),
        Err(error) => Err(error.to_string()),
    }
}

/// Publish a complete artifact with create-new staging and same-directory
/// rename. In particular, replacing a hardlinked destination must never
/// truncate its other link (which could be the source file).
pub(super) fn publish_artifact(path: &str, bytes: &[u8]) -> Result<(), String> {
    if !bytes.starts_with(b"\0asm\x01\0\0\0") {
        return Err("invalid generated Wasm header".into());
    }
    let destination = canonical_destination(path)?;
    let parent = destination.parent().ok_or("output requires a parent")?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    for attempt in 0..64_u8 {
        let temporary = parent.join(format!(
            ".attoc-{}-{nonce}-{attempt}.tmp",
            std::process::id()
        ));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        };
        let result = (|| {
            file.write_all(bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            // Recheck for a clear diagnostic. rename() replaces the final
            // directory entry without following its symlink in any case.
            if canonical_destination(path)? != destination {
                return Err("output parent changed during publication".into());
            }
            fs::rename(&temporary, &destination).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    Err("unable to allocate unique compiler artifact staging file".into())
}
