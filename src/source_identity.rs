//! Independent Rust/Cranelift *source identity* admission.
//! No shared Java/LLVM MIR, JIT hook, OpenTelemetry exporter or logging syntax.
//! This stage proves a source-controlled routine registry against THIS compiler's
//! validated scalar MIR, and produces an object-adjacent map for the same compile.
use crate::mir;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const REGISTRY_SCHEMA: &str = "atto.telemetry.source-ids.v1";
const MAP_SCHEMA: &str = "atto.telemetry.verified-source-map.v1";
const MAX_ROUTINES: usize = 4096;
const MAX_SITES: usize = 16384;

fn logical_unit(unit: &str) -> Result<(), String> {
    if unit.is_empty()
        || unit.starts_with('/')
        || unit.contains('\\')
        || unit.contains(':')
        || unit.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !(unit.ends_with(".atto") || unit.ends_with(".ores"))
    {
        return Err("source identity requires a package-relative .atto/.ores logical unit without aliases or traversal".into());
    }
    Ok(())
}

fn valid_id(value: &str, prefix: &str) -> bool {
    let Some(hex) = value.strip_prefix(prefix) else {
        return false;
    };
    hex.len() == 32
        && hex.as_bytes().iter().all(|&c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        && hex.bytes().any(|c| c != b'0')
}

// Typed JSON deserialization rejects duplicate fields and unknown keys by
// construction. A generic serde_json::Value silently keeps the last duplicate.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema: String,
    routines: Vec<RoutineEntry>,
    sites: Vec<SiteEntry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RoutineEntry {
    id: String,
    source: String,
    symbol: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SiteEntry {
    id: String,
    source: String,
    routine_id: String,
}

/// Synchronize checked scalar routine IDs without pretending guest log sites
/// are compiled. Existing (source,signature) IDs survive formatting changes.
/// Explicit old_unit permits exact whole-file moves, but not partial renames.
/// Caller injects OS-CSPRNG fresh IDs; returns a proposed manifest, not a write.
pub fn sync_routines<F>(
    module: &mir::Module, source_unit: &str, previous_json: &str,
    old_unit: Option<&str>, mut fresh_routine_id: F,
) -> Result<Value, String>
where F: FnMut() -> Result<String, String>
{
    logical_unit(source_unit)?;
    if let Some(old) = old_unit {
        logical_unit(old)?;
        if old == source_unit { return Err("source move must change the logical path".into()); }
    }
    if previous_json.len() > 1024 * 1024 {
        return Err("source-ID registry exceeds 1 MiB".into());
    }
    let mut registry: Registry = serde_json::from_str(previous_json)
        .map_err(|e| format!("invalid registry JSON: {e}"))?;
    if registry.schema != REGISTRY_SCHEMA {
        return Err("unsupported source-ID registry schema".into());
    }
    if registry.routines.len() > MAX_ROUTINES ||
        registry.sites.len() > MAX_SITES {
        return Err("source-ID registry budget exceeded".into());
    }

    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    let mut owner = HashMap::new();
    for routine in &registry.routines {
        logical_unit(&routine.source)?;
        if routine.symbol.is_empty()
            || !valid_id(&routine.id, "rid_")
            || !ids.insert(routine.id.clone())
            || !keys.insert((routine.source.clone(), routine.symbol.clone())) {
            return Err("duplicate, malformed or ambiguous source routine registry entry".into());
        }
        owner.insert(routine.id.clone(), routine.source.clone());
    }
    let mut site_ids = HashSet::new();
    for site in &registry.sites {
        logical_unit(&site.source)?;
        if !valid_id(&site.id, "sid_") ||
           !site_ids.insert(site.id.as_str()) ||
           owner.get(&site.routine_id) != Some(&site.source) {
            return Err("duplicate or forged static source site entry".into());
        }
        if site.source == source_unit || old_unit == Some(site.source.as_str()) {
            return Err("cannot sync source logging sites before compiler site lowering".into());
        }
    }
    let mut expected = HashSet::new();
    for fnc in &module.functions {
        let signature = format!("{}({}):int", fnc.symbol,
            vec!["int"; fnc.params.len()].join(","));
        if !expected.insert(signature) {
            return Err("duplicate compiled Rust scalar callable signature".into());
        }
    }
    if expected.is_empty() { return Err("empty source unit cannot receive routine IDs".into()); }
    if expected.len() > MAX_ROUTINES {
        return Err("compiled routine budget exceeded".into());
    }
    if let Some(old) = old_unit {
        let from_old: HashSet<String> = registry.routines.iter()
            .filter(|e| e.source == old)
            .map(|e| e.symbol.clone()).collect();
        if from_old != expected {
            return Err("explicit move requires exact callable inventory, not partial moves or renames".into());
        }
        if registry.routines.iter().any(|e| e.source == source_unit) {
            return Err("source move target already has source-ID entries".into());
        }
        for entry in &mut registry.routines {
            if entry.source == old { entry.source = source_unit.to_string(); }
        }
    }
    let mut existing: HashSet<String> = registry.routines.iter()
        .filter(|e| e.source == source_unit)
        .map(|e| e.symbol.clone()).collect();
    if !existing.is_subset(&expected) {
        return Err("stale checked-in routine ID; refuse silent renaming or removal".into());
    }
    for fnc in &module.functions {
        let signature = format!("{}({}):int", fnc.symbol,
            vec!["int"; fnc.params.len()].join(","));
        if existing.insert(signature.clone()) {
            let id = fresh_routine_id()?;
            if !valid_id(&id, "rid_") || !ids.insert(id.clone()) {
                return Err("fresh routine ID allocator returned invalid, zero or duplicate ID".into());
            }
            registry.routines.push(RoutineEntry {
                id, source: source_unit.to_string(), symbol: signature,
            });
            if registry.routines.len() > MAX_ROUTINES {
                return Err("registry exceeds routine budget after synchronization".into());
            }
        }
    }
    // Revalidate against THIS compiler's independent, checked MIR. This
    // output is not an instrumented native artifact or an OTLP event.
    let proposed = serde_json::to_value(&registry).map_err(|e| e.to_string())?;
    let encoded = serde_json::to_string(&proposed).map_err(|e| e.to_string())?;
    let verified = verify_routines(module, source_unit, &encoded)?;
    if verified["instrumentation_status"] != "not_implemented" {
        return Err("unexpected claimed Rust guest log instrumentation".into());
    }
    Ok(proposed)
}

/// A routine ID can be reused across generations of the same source routine;
/// an invocation's OpenTelemetry trace/span IDs MUST be generated separately.
///
/// Current Rust scalar frontend cannot parse log/spans. Any declared *site*
/// for this unit is rejected rather than falsely claiming instrumented logs.
pub fn verify_routines(
    module: &mir::Module,
    source_unit: &str,
    registry_json: &str,
) -> Result<Value, String> {
    logical_unit(source_unit)?;
    if registry_json.len() > 1024 * 1024 {
        return Err("source-ID registry exceeds 1 MiB".into());
    }
    let root: Registry = serde_json::from_str(registry_json)
        .map_err(|e| format!("invalid source-ID registry JSON: {e}"))?;
    if root.schema != REGISTRY_SCHEMA {
        return Err("unsupported source-ID registry schema".into());
    }
    if root.routines.len() > MAX_ROUTINES {
        return Err("source-ID registry routine budget exceeded".into());
    }
    if root.sites.len() > MAX_SITES {
        return Err("source-ID registry site budget exceeded".into());
    }
    let mut seen_ids = HashSet::new();
    let mut seen_symbols = HashSet::new();
    let mut by_symbol = HashMap::new();
    let mut routine_sources = HashMap::new();
    for entry in &root.routines {
        let id = entry.id.as_str();
        let source = entry.source.as_str();
        let symbol = entry.symbol.as_str();
        if symbol.is_empty() { return Err("empty source routine symbol".into()); }
        logical_unit(source)?;
        if !valid_id(id, "rid_") {
            return Err(format!("invalid 128-bit lowercase routine ID '{id}'"));
        }
        if !seen_ids.insert(id) {
            return Err(format!("duplicate routine ID '{id}'"));
        }
        if !seen_symbols.insert((source, symbol)) {
            return Err(format!("duplicate source routine identity '{source}:{symbol}'"));
        }
        routine_sources.insert(id, source);
        if source == source_unit {
            by_symbol.insert(symbol, id);
        }
    }
    let mut site_ids = HashSet::new();
    for site in &root.sites {
        let id = site.id.as_str();
        let source = site.source.as_str();
        let routine_id = site.routine_id.as_str();
        logical_unit(source)?;
        if !valid_id(id, "sid_") || !site_ids.insert(id) {
            return Err(format!("invalid or duplicated 128-bit source site ID '{id}'"));
        }
        let Some(routine_source) = routine_sources.get(routine_id) else {
            return Err(format!("site references unknown routine ID '{routine_id}'"));
        };
        if source != *routine_source {
            return Err(format!("source site '{id}' points to a routine in another source unit"));
        }
        if source == source_unit {
            return Err("Rust scalar backend cannot verify log/span sites yet: fail closed".into());
        }
    }
    let mut functions = Vec::with_capacity(module.functions.len());
    for function in &module.functions {
        let signature = format!("{}({}):int", function.symbol, vec!["int"; function.params.len()].join(","));
        let id = by_symbol.remove(signature.as_str())
            .ok_or_else(|| format!("missing checked-in routine ID for '{source_unit}:{signature}'"))?;
        functions.push(json!({
            "symbol": signature,
            "routine_id": id,
            "site_count": 0,
            "instrumented": false
        }));
    }
    if let Some(stale) = by_symbol.keys().next() {
        return Err(format!("stale registry entry for uncompiled routine '{source_unit}:{stale}'"));
    }
    Ok(json!({
        "schema": MAP_SCHEMA,
        "backend": "oreslang-source.rs",
        "source_unit": source_unit,
        "source_identity_status": "routine_registry_verified",
        "instrumentation_status": "not_implemented",
        "functions": functions,
        "sites": []
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{frontend, mir};

    const SOURCE: &str =
        "pub fnc main():int{return add(40,2);} fnc add(int a,int b):int{return a+b;}";
    const REGISTRY: &str = r#"{
      "schema":"atto.telemetry.source-ids.v1",
      "routines":[
        {"id":"rid_11111111111111111111111111111111","source":"src/app.atto","symbol":"main():int"},
        {"id":"rid_22222222222222222222222222222222","source":"src/app.atto","symbol":"add(int,int):int"}
      ],"sites":[]}"#;

    fn checked_module() -> mir::Module {
        mir::lower(frontend::parse(SOURCE).expect("source parses"))
            .expect("independent Rust MIR validates")
    }
    #[test]
    fn preserves_checked_in_ids_for_compiled_scalar_mir() {
        let m = verify_routines(&checked_module(), "src/app.atto", REGISTRY).unwrap();
        assert_eq!(m["functions"][0]["routine_id"], "rid_11111111111111111111111111111111");
        assert_eq!(m["functions"][1]["routine_id"], "rid_22222222222222222222222222222222");
        assert_eq!(m["instrumentation_status"], "not_implemented");
    }
    #[test]
    fn formatting_and_comments_do_not_reallocate_ids() {
        let spaced = SOURCE.replace("fnc add(", "\n // moved\n fnc add(");
        let module = mir::lower(frontend::parse(&spaced).unwrap()).unwrap();
        let a = verify_routines(&checked_module(), "src/app.atto", REGISTRY).unwrap();
        let b = verify_routines(&module, "src/app.atto", REGISTRY).unwrap();
        assert_eq!(a, b);
    }
    #[test]
    fn rejects_duplicate_and_stale_routines() {
        let duplicate = REGISTRY.replace("rid_22222222222222222222222222222222", "rid_11111111111111111111111111111111");
        assert!(verify_routines(&checked_module(), "src/app.atto", &duplicate).unwrap_err().contains("duplicate routine"));
        let stale = REGISTRY.replace("add(int,int):int", "stale():int");
        assert!(verify_routines(&checked_module(), "src/app.atto", &stale).unwrap_err().contains("missing checked-in"));
    }
    #[test]
    fn rejects_fake_sites_and_nonportable_ids() {
        let fake = REGISTRY.replace(r#""sites":[]"#, r#""sites":[{"id":"sid_33333333333333333333333333333333","routine_id":"rid_11111111111111111111111111111111","source":"src/app.atto"}]"#);
        assert!(verify_routines(&checked_module(), "src/app.atto", &fake).unwrap_err().contains("fail closed"));
        assert!(verify_routines(&checked_module(), "../src/app.atto", REGISTRY).is_err());
        let bad = REGISTRY.replace("rid_11111111111111111111111111111111", "rid_NOT_A_128_BIT_ID");
        assert!(verify_routines(&checked_module(), "src/app.atto", &bad).is_err());
    }
    #[test]
    fn rejects_duplicate_json_keys_and_unknown_fields() {
        // serde_json::Value alone would silently keep the last value of a
        // duplicated field; the typed schema must reject this ambiguity.
        let duplicate_root = REGISTRY.replacen(
            r#""schema":"atto.telemetry.source-ids.v1""#,
            r#""schema":"atto.telemetry.source-ids.v1","schema":"atto.telemetry.source-ids.v1""#,
            1,
        );
        assert!(verify_routines(&checked_module(), "src/app.atto", &duplicate_root).is_err());
        let duplicated_id = REGISTRY.replacen(
            r#""id":"rid_11111111111111111111111111111111""#,
            r#""id":"rid_11111111111111111111111111111111","id":"rid_11111111111111111111111111111111""#,
            1,
        );
        assert!(verify_routines(&checked_module(), "src/app.atto", &duplicated_id).is_err());
        let unknown = REGISTRY.replacen(
            r#""symbol":"main():int""#,
            r#""symbol":"main():int","instrumented":true"#,
            1,
        );
        assert!(verify_routines(&checked_module(), "src/app.atto", &unknown).is_err());
    }

    #[test]
    fn rejects_cross_source_site_provenance_and_duplicate_site_ids() {
        // A foreign-source site may be listed in the same manifest but must
        // reference a routine declared in that SAME source, not this function.
        let foreign = REGISTRY.replace(
            r#""sites":[]"#,
            r#""sites":[{"id":"sid_33333333333333333333333333333333",
               "routine_id":"rid_11111111111111111111111111111111",
               "source":"foreign/other.atto"}]"#,
        );
        assert!(verify_routines(&checked_module(), "src/app.atto", &foreign)
            .unwrap_err().contains("another source unit"));
        let dup = REGISTRY.replace(
            r#""sites":[]"#,
            r#""sites":[{"id":"sid_33333333333333333333333333333333",
               "routine_id":"rid_11111111111111111111111111111111",
               "source":"src/app.atto"},
               {"id":"sid_33333333333333333333333333333333",
               "routine_id":"rid_11111111111111111111111111111111",
               "source":"src/app.atto"}]"#,
        );
        assert!(verify_routines(&checked_module(), "src/app.atto", &dup)
            .unwrap_err().contains("duplicated"));
    }

    #[test]
    fn sync_preserves_checked_ids_across_formatting_and_orders() {
        let first = sync_routines(
            &checked_module(), "src/app.atto", REGISTRY, None,
            || panic!("a previously checked callable must never reallocate its ID"),
        ).unwrap();
        let spaced = SOURCE.replace("fnc add(", "\n// formatting changed\nfnc add(");
        let checked = mir::lower(frontend::parse(&spaced).unwrap()).unwrap();
        let again = sync_routines(
            &checked, "src/app.atto", &first.to_string(), None,
            || panic!("source formatting is not a new identity"),
        ).unwrap();
        assert_eq!(first, again);
    }

    #[test]
    fn sync_creates_new_routines_only_from_explicit_generator() {
        let empty = r#"{"schema":"atto.telemetry.source-ids.v1","routines":[],"sites":[]}"#;
        let mut generated = 0;
        let updated = sync_routines(
            &checked_module(), "src/app.atto", empty, None, || {
                generated += 1;
                Ok(if generated == 1 {
                    "rid_33333333333333333333333333333333".to_string()
                } else {
                    "rid_44444444444444444444444444444444".to_string()
                })
            },
        ).unwrap();
        assert_eq!(generated, 2);
        assert_eq!(updated["routines"].as_array().unwrap().len(), 2);
        assert!(verify_routines(&checked_module(), "src/app.atto",
            &updated.to_string()).is_ok());
        let denied = sync_routines(&checked_module(), "src/app.atto", empty, None,
            || Ok("rid_00000000000000000000000000000000".to_string()));
        assert!(denied.is_err());
        let duplicate = sync_routines(&checked_module(), "src/app.atto", empty, None,
            || Ok("rid_33333333333333333333333333333333".to_string()));
        assert!(duplicate.unwrap_err().contains("duplicate"));
    }

    #[test]
    fn explicit_move_requires_exact_inventory_and_retains_id() {
        let old = REGISTRY.replace("src/app.atto", "lib/old.atto");
        let relocated = sync_routines(
            &checked_module(), "lib/new.atto", &old, Some("lib/old.atto"),
            || panic!("a verified move should not allocate a new identity"),
        ).unwrap();
        assert!(relocated["routines"].as_array().unwrap().iter()
            .all(|entry| entry["source"] == "lib/new.atto"));
        assert!(verify_routines(&checked_module(), "lib/new.atto",
            &relocated.to_string()).is_ok());
        assert!(sync_routines(&checked_module(), "lib/new.atto", &old,
            Some("lib/other.atto"), || Ok(String::new())).is_err());
        assert!(sync_routines(&checked_module(), "lib/new.atto", &old,
            None, || Ok("rid_11111111111111111111111111111111".into())).is_err());
    }

    #[test]
    fn sync_rejects_sites_it_cannot_codegen_and_stale_registry() {
        let unknown_site = REGISTRY.replace(
            r#""sites":[]"#,
            r#""sites":[{"id":"sid_55555555555555555555555555555555",
               "routine_id":"rid_11111111111111111111111111111111",
               "source":"src/app.atto"}]"#,
        );
        assert!(sync_routines(&checked_module(), "src/app.atto",
            &unknown_site, None, || panic!("site must fail closed")).is_err());
        let stale = REGISTRY.replace("add(int,int):int", "old_add(int,int):int");
        assert!(sync_routines(&checked_module(), "src/app.atto",
            &stale, None, || panic!("stale registry must fail")).is_err());
    }

    #[test]
    fn does_not_accept_unimplemented_guest_logging_syntax() {
        let unsupported = "pub fnc main():int{log.info(\"hi\");return 42;}";
        assert!(frontend::parse(unsupported).is_err());
    }
}
