//! Independent Rust/Cranelift *source identity* admission.
//! No shared Java/LLVM MIR, JIT hook, OpenTelemetry exporter or logging syntax.
//! This stage proves a source-controlled routine registry against THIS compiler's
//! validated scalar MIR, and produces an object-adjacent map for the same compile.
use crate::mir;
use serde::Deserialize;
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema: String,
    routines: Vec<RoutineEntry>,
    sites: Vec<SiteEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineEntry {
    id: String,
    source: String,
    symbol: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteEntry {
    id: String,
    source: String,
    routine_id: String,
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
    fn does_not_accept_unimplemented_guest_logging_syntax() {
        let unsupported = "pub fnc main():int{log.info(\"hi\");return 42;}";
        assert!(frontend::parse(unsupported).is_err());
    }
}
