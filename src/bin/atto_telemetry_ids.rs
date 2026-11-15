//! Rust-owned source identity developer tool. This is a compiler-backed
//! check/lookup, not a telemetry exporter and not a log/span parser.
//!
//! Usage:
//!   atto_telemetry_ids check <source.atto> <registry.json>
//!   atto_telemetry_ids locate <source.atto> <registry.json> <rid_...>
//!
//! The logical source unit is derived from the actual package-root file, never
//! accepted as an unverifiable user-supplied alias.
use oreslang_source_rs::{check_source_ids, locate_source_routine, sync_source_ids};
use std::{
    env,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const MAX_BYTES: u64 = 1024 * 1024;

/// Best-effort metadata stability before opening and after consuming the
/// source. This is not a security proof against a privileged concurrent
/// writer, but prevents ordinary in-place mutation/replacement from being
/// accepted as a single source-ID snapshot.
fn same_snapshot_metadata(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    if !first.is_file() || !second.is_file() || first.len() != second.len() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        first.dev() == second.dev() && first.ino() == second.ino()
            && first.mtime() == second.mtime()
            && first.mtime_nsec() == second.mtime_nsec()
            && first.ctime() == second.ctime()
            && first.ctime_nsec() == second.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        match (first.modified(), second.modified()) {
            (Ok(before), Ok(after)) => before == after,
            _ => false,
        }
    }
}

fn read_regular_bounded(path: &Path) -> Result<String, String> {
    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("telemetry ID inputs must be regular non-symlink files".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !same_snapshot_metadata(&before, &opened) {
        return Err("telemetry ID input changed during opening".into());
    }
    if opened.len() > MAX_BYTES {
        return Err("telemetry ID input exceeds 1 MiB budget".into());
    }
    let mut text = String::new();
    (&mut file).take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > MAX_BYTES {
        return Err("telemetry ID input exceeds 1 MiB budget".into());
    }
    let consumed = file.metadata().map_err(|e| e.to_string())?;
    if !same_snapshot_metadata(&opened, &consumed) ||
        consumed.len() != text.len() as u64 {
        return Err("telemetry ID source or registry mutated while reading".into());
    }
    Ok(text)
}

fn rooted_source(source: &Path, registry: &Path) -> Result<(PathBuf, String), String> {
    let root = fs::canonicalize(".").map_err(|e| e.to_string())?;
    let canonical_source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let canonical_registry = fs::canonicalize(registry).map_err(|e| e.to_string())?;
    if !canonical_source.starts_with(&root) || !canonical_registry.starts_with(&root) {
        return Err("source and registry must be inside current package root".into());
    }
    if source.is_absolute() || registry.is_absolute()
        || root.join(source) != canonical_source
        || root.join(registry) != canonical_registry
    {
        return Err("input paths must be canonical package-relative paths without symlink aliases".into());
    }
    if canonical_source == canonical_registry {
        return Err("source and registry must be distinct files".into());
    }
    // Filesystem canonicalization does not collapse hardlinks. Do not let
    // a single inode carry both independently reviewed source and manifest.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let src = fs::metadata(&canonical_source).map_err(|e| e.to_string())?;
        let ids = fs::metadata(&canonical_registry).map_err(|e| e.to_string())?;
        if src.dev() == ids.dev() && src.ino() == ids.ino() {
            return Err("source and registry must not be hardlink aliases".into());
        }
    }
    // Reject alternative path spellings/aliases including symlink parents.
    let logical = canonical_source.strip_prefix(&root).map_err(|e| e.to_string())?;
    if logical.as_os_str().is_empty() {
        return Err("expected source below package root".into());
    }
    let name = logical.to_str().ok_or("non-UTF-8 source units are unsupported")?;
    let unit = name.replace(std::path::MAIN_SEPARATOR, "/");
    if !(unit.ends_with(".atto") || unit.ends_with(".ores")) {
        return Err("expected .atto source or explicitly supported legacy .ores".into());
    }
    Ok((canonical_source, unit))
}

/// Generate one nonzero 128-bit static routine ID using kernel CSPRNG
/// bytes. Never derive stable IDs from a hash of source text: moves and
/// duplicate signatures need explicit checked-in identity assignment.
fn new_os_routine_id() -> Result<String, String> {
    #[cfg(unix)]
    {
        let mut bytes = [0u8; 16];
        let mut source = fs::File::open("/dev/urandom")
            .map_err(|_| "OS CSPRNG unavailable: refusing to synthesize an ID")?;
        source.read_exact(&mut bytes)
            .map_err(|_| "OS CSPRNG exhausted: refusing to synthesize an ID")?;
        if bytes.iter().all(|value| *value == 0) {
            return Err("OS CSPRNG returned an invalid all-zero ID".into());
        }
        let mut result = String::with_capacity(36);
        result.push_str("rid_");
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in bytes {
            result.push(HEX[(byte >> 4) as usize] as char);
            result.push(HEX[(byte & 15) as usize] as char);
        }
        return Ok(result);
    }
    #[cfg(not(unix))]
    Err("this CLI requires a native OS CSPRNG adapter; no unsafe fallback".into())
}

/// Write the reviewed proposal to an unused regular filename. The checked-in
/// registry is intentionally never modified in-place or through a hardlink.
fn write_proposal(path: &Path, source: &Path, registry: &Path,
                  json: &serde_json::Value) -> Result<(), String> {
    if path.is_absolute() || path.components().any(|part|
        !matches!(part, std::path::Component::Normal(_))) {
        return Err("proposal destination must be a canonical package-relative path".into());
    }
    let root = fs::canonicalize(".").map_err(|e| e.to_string())?;
    // Path::parent("proposal.json") returns Some("") rather than None.
    // Canonicalizing "" fails on Unix, incorrectly rejecting a root-level
    // proposal even when it is a safe package-relative new filename.
    let parent = path.parent().filter(|value| !value.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let real_parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if !real_parent.starts_with(&root) || root.join(parent) != real_parent {
        return Err("proposal parent aliases or escapes the package root".into());
    }
    let resolved = real_parent.join(path.file_name().ok_or("proposal filename required")?);
    if resolved == fs::canonicalize(source).map_err(|e| e.to_string())? ||
       resolved == fs::canonicalize(registry).map_err(|e| e.to_string())? {
        return Err("proposal must not alias source or checked-in registry".into());
    }
    let bytes = serde_json::to_vec_pretty(json).map_err(|e| e.to_string())?;
    // The on-disk proposal contains a final newline in addition to the
    // serialized JSON. Enforce the input reader's full 1 MiB file budget,
    // not a budget on the JSON bytes before appending the newline.
    if bytes.len() >= MAX_BYTES as usize {
        return Err("synchronized registry plus newline exceeds 1 MiB".into());
    }
    let mut output = fs::OpenOptions::new().write(true).create_new(true)
        .open(path).map_err(|e| format!("proposal must be a new path: {e}"))?;
    output.write_all(&bytes).map_err(|e| e.to_string())?;
    output.write_all(b"\n").map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

fn run(args: &[String]) -> Result<String, String> {
    let (operation, source_path, registry_path, query, moved_from) = match args {
        [command, source, registry] if command == "check" =>
            (command.as_str(), source.as_str(), registry.as_str(), None, None),
        [command, source, registry, id] if command == "locate" =>
            (command.as_str(), source.as_str(), registry.as_str(), Some(id.as_str()), None),
        [command, source, registry, destination] if command == "sync" =>
            (command.as_str(), source.as_str(), registry.as_str(), Some(destination.as_str()), None),
        [command, source, registry, destination, moved] if command == "sync"
                && moved.starts_with("--moved-from=") =>
            (command.as_str(), source.as_str(), registry.as_str(),
             Some(destination.as_str()), Some(&moved["--moved-from=".len()..])),
        _ => return Err(
            "usage: atto_telemetry_ids check <source.atto> <registry.json> | \
             locate <source.atto> <registry.json> <rid_...> | \
             sync <source.atto> <registry.json> <new-registry.json> \
             [--moved-from=old/source.atto]".into()),
    };
    let source_path = Path::new(source_path);
    let registry_path = Path::new(registry_path);
    let (_physical, unit) = rooted_source(source_path, registry_path)?;
    let source = read_regular_bounded(source_path)?;
    let registry = read_regular_bounded(registry_path)?;
    let verified = match (operation, query) {
        ("check", None) => check_source_ids(&source, &unit, &registry)?,
        ("locate", Some(id)) => locate_source_routine(&source, &unit, &registry, id)?,
        ("sync", Some(destination)) => {
            let manifest = sync_source_ids(
                &source, &unit, &registry, moved_from, new_os_routine_id)?;
            write_proposal(Path::new(destination), source_path, registry_path, &manifest)?;
            return Ok(format!("proposed_registry_created: {destination}"));
        }
        _ => return Err("unknown telemetry ID operation".into()),
    };
    serde_json::to_string_pretty(&verified).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
        Ok(report) => { println!("{report}"); ExitCode::SUCCESS }
        Err(message) => { eprintln!("atto_telemetry_ids: {message}"); ExitCode::from(1) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_commands_and_missing_arguments() {
        assert!(run(&[]).unwrap_err().contains("usage:"));
        assert!(run(&["sync".into()]).is_err());
        assert!(run(&["locate".into(), "a.atto".into(), "ids.json".into()]).is_err());
    }

    #[test]
    fn rejects_package_root_source_as_filename() {
        let current = env::current_dir().unwrap();
        assert!(rooted_source(&current, Path::new("Cargo.toml")).is_err());
    }

    #[test]
    fn refuses_source_registry_aliases() {
        let manifest = Path::new("Cargo.toml");
        assert!(rooted_source(manifest, manifest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn denies_source_and_manifest_that_alias_the_same_hardlink_inode() {
        let file_name = format!("__atto_source_link_{}.atto", std::process::id());
        let linked = Path::new(&file_name);
        assert!(!linked.exists());
        fs::hard_link("Cargo.toml", linked).expect("create local hardlink test input");
        let result = rooted_source(linked, Path::new("Cargo.toml"));
        fs::remove_file(linked).expect("remove local test hardlink");
        assert!(result.unwrap_err().contains("hardlink aliases"));
    }

    #[test]
    fn detects_in_place_input_mutation_from_saved_metadata() {
        let file_name = format!("__atto_mutation_{}.json", std::process::id());
        let path = Path::new(&file_name);
        assert!(!path.exists());
        fs::write(path, b"one").expect("create private test input");
        let before = fs::metadata(path).expect("snapshot original input");
        fs::write(path, b"longer").expect("mutate input bytes");
        let after = fs::metadata(path).expect("snapshot changed input");
        fs::remove_file(path).expect("remove temporary test input");
        assert!(!same_snapshot_metadata(&before, &after));
    }

    #[test]
    fn checks_and_locates_real_atto_fixture_without_native_codegen() {
        let source = "examples/source-ids-main.atto".to_string();
        let registry = "examples/source-ids-main.registry.json".to_string();
        let checked: serde_json::Value = serde_json::from_str(
            &run(&["check".to_owned(), source.clone(), registry.clone()]).unwrap()
        ).unwrap();
        assert_eq!(checked["instrumentation_status"], "not_implemented");
        assert_eq!(checked["functions"][0]["routine_id"],
                   "rid_a458b8330ea5475ba8a0cae2c64b48bc");
        let located: serde_json::Value = serde_json::from_str(
            &run(&["locate".to_owned(), source.clone(), registry.clone(),
                   "rid_a458b8330ea5475ba8a0cae2c64b48bc".into()]).unwrap()
        ).unwrap();
        assert_eq!(located["symbol"], "main():int");
        assert_eq!(located["status"], "verified_source_only_not_deployed_binary");
        assert!(run(&["locate".to_owned(), source, registry,
                     "sid_a458b8330ea5475ba8a0cae2c64b48bc".into()]).is_err());
    }

    #[test]
    fn rejects_bad_move_without_publishing_a_registry() {
        let output = "examples/__atte_source_id_invalid_move_proposal.json";
        assert!(!Path::new(output).exists());
        let rejected = run(&[
            "sync".into(),
            "examples/source-ids-main.atto".into(),
            "examples/source-ids-main.registry.json".into(),
            output.into(),
            "--moved-from=../escape.atto".into(),
        ]);
        assert!(rejected.is_err());
        assert!(!Path::new(output).exists());
    }

    #[test]
    fn permits_new_root_level_proposals_without_overwriting_existing_files() {
        let destination = format!("__atto_source_ids_proposal_{}.json", std::process::id());
        let path = Path::new(&destination);
        assert!(!path.exists());
        let fixture = serde_json::json!({"schema": "fixture"});
        write_proposal(path, Path::new("Cargo.toml"), Path::new("Cargo.lock"), &fixture)
            .expect("a root-level exclusive proposal must be supported");
        assert_eq!(fs::read_to_string(path).unwrap(), "{\n  \"schema\": \"fixture\"\n}\n");
        assert!(write_proposal(path, Path::new("Cargo.toml"),
            Path::new("Cargo.lock"), &fixture).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn proposal_budget_counts_appended_newline_before_file_creation() {
        let output = format!("__atto_identity_budget_{}.json", std::process::id());
        let path = Path::new(&output);
        assert!(!path.exists());
        let baseline = serde_json::to_vec_pretty(&serde_json::json!({"k": ""}))
            .expect("baseline JSON serializer");
        let value = serde_json::json!({
            "k": "x".repeat(MAX_BYTES as usize - baseline.len())
        });
        assert_eq!(serde_json::to_vec_pretty(&value).unwrap().len(), MAX_BYTES as usize);
        let error = write_proposal(path, Path::new("Cargo.toml"),
            Path::new("Cargo.lock"), &value).unwrap_err();
        assert!(error.contains("plus newline"));
        assert!(!path.exists(), "oversize proposals must never create a file");
    }

    #[test]
    fn rejects_missing_source_without_creating_files() {
        let missing = Path::new("__atto_identity_missing_source_test.atto");
        let result = read_regular_bounded(missing).unwrap_err();
        assert!(!result.is_empty());
        assert!(!missing.exists());
    }
}
