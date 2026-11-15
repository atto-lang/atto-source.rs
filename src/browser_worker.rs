//! Browser tenant artifact entrypoint for the Rust frontend.
//! This is a compiler-produced metadata inspection step, not a security proof:
//! the browser host must still validate the module and enforce capabilities.

use crate::{compile, Diagnostic, Pos};

pub const TENANT_PROTOCOL: &str = "atto.browser.tenants.v1";
pub const GUEST_ABI: &str = "atto.browser.guest.v2";
pub const MAX_ACTOR_TYPES_PER_TENANT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserTenantArtifact {
    pub wasm: Vec<u8>,
    pub wire_protocol: &'static str,
    pub guest_abi: &'static str,
    pub actor_names: Vec<String>,
}

/// Compile a browser guest and expose compiler-emitted actor type names so a
/// bundler can construct its tenant manifest. Every actor retains a distinct
/// Wasm instance, private globals, and memory in the browser host.
pub fn compile_browser_tenant(source: &str) -> Result<BrowserTenantArtifact, Diagnostic> {
    let wasm = compile(source)?;
    let actor_names = read_actor_names(&wasm).map_err(|_| {
        Diagnostic::new(
            Pos { line: 1, col: 1 },
            "E_WORKER_ABI",
            "compiler-produced Wasm has an invalid or over-budget atto.actors section",
        )
    })?;
    Ok(BrowserTenantArtifact {
        wasm,
        wire_protocol: TENANT_PROTOCOL,
        guest_abi: GUEST_ABI,
        actor_names,
    })
}

fn read_uleb(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    let mut value = 0u64;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if value > u64::from(u32::MAX) { return None; }
        if byte & 0x80 == 0 { return Some(value as u32); }
    }
    None
}

fn read_actor_names(wasm: &[u8]) -> Result<Vec<String>, ()> {
    if !wasm.starts_with(b"\0asm\x01\0\0\0") { return Err(()); }
    let mut cursor = 8usize;
    let mut found = None;
    while cursor < wasm.len() {
        let section_id = *wasm.get(cursor).ok_or(())?;
        cursor += 1;
        let len = read_uleb(wasm, &mut cursor).ok_or(())? as usize;
        let end = cursor.checked_add(len).filter(|end| *end <= wasm.len()).ok_or(())?;
        if section_id == 0 {
            let mut name_start = cursor;
            let name_len = read_uleb(wasm, &mut name_start).ok_or(())? as usize;
            let name_end = name_start.checked_add(name_len).filter(|end| *end <= cursor + len).ok_or(())?;
            if wasm.get(name_start..name_end) == Some(&b"atto.actors"[..]) {
                if found.is_some() { return Err(()); }
                found = Some(&wasm[name_end..end]);
            }
        }
        cursor = end;
    }
    let Some(payload) = found else { return Ok(Vec::new()) };
    let mut index = 0usize;
    let count = read_uleb(payload, &mut index).ok_or(())? as usize;
    if count > MAX_ACTOR_TYPES_PER_TENANT { return Err(()); }
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let len = read_uleb(payload, &mut index).ok_or(())? as usize;
        let end = index.checked_add(len).filter(|end| *end <= payload.len()).ok_or(())?;
        let name = std::str::from_utf8(&payload[index..end]).map_err(|_| ())?;
        if name.is_empty() || name.len() > 256 { return Err(()); }
        names.push(name.to_owned());
        index = end;
    }
    if index != payload.len() { return Err(()); }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_sections_fail_closed() {
        assert!(read_actor_names(b"not wasm").is_err());
        assert!(read_actor_names(b"\0asm\x01\0\0\0\0\x7f").is_err());
    }
    #[test]
    fn scalar_has_no_actor_section() {
        let result = compile_browser_tenant("pub fnc main(): int { return 42; }").unwrap();
        assert!(result.actor_names.is_empty());
        assert_eq!(result.wire_protocol, TENANT_PROTOCOL);
        assert_eq!(result.guest_abi, GUEST_ABI);
        assert_eq!(result.wasm, compile("pub fnc main(): int { return 42; }").unwrap());
    }
}
