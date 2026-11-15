//! Browser tenant artifact entrypoint for the Rust frontend.
//! This is a compiler-produced metadata inspection step, not a security proof:
//! the browser host must still validate the module and enforce capabilities.

use crate::{compile, Diagnostic, Pos};
use std::collections::HashSet;

pub const TENANT_PROTOCOL: &str = "atto.browser.tenants.v1";
pub const GUEST_ABI: &str = "atto.browser.guest.v2";
/// Versioned host-side completion and uncertainty contract. Not a Wasm import.
pub const SEND_RECEIPT_PROTOCOL: &str = "atto.browser.send-receipt.v1";

/// A recipient's processing acknowledgement is distinct from admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteReceiptOutcome { Processed, Rejected, Unknown }
impl RemoteReceiptOutcome {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }
}

/// Stable error codes; arbitrary guest error strings are never wire codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteReceiptCode {
    Ok, RouteStale, RouteBackpressure, RouteTimeout, RouteUnavailable,
    RouteCrash, RouteRevoked, RouteReceiver, RouteProtocol, RouteUnknown,
}
impl RemoteReceiptCode {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::RouteStale => "E_ROUTE_STALE",
            Self::RouteBackpressure => "E_ROUTE_BACKPRESSURE",
            Self::RouteTimeout => "E_ROUTE_TIMEOUT",
            Self::RouteUnavailable => "E_ROUTE_UNAVAILABLE",
            Self::RouteCrash => "E_ROUTE_CRASH",
            Self::RouteRevoked => "E_ROUTE_REVOKED",
            Self::RouteReceiver => "E_ROUTE_RECEIVER",
            Self::RouteProtocol => "E_ROUTE_PROTOCOL",
            Self::RouteUnknown => "E_ROUTE_UNKNOWN",
        }
    }
}
/// Host-issued correlation metadata, with no tenant or route capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteReceiptHeader {
    pub receipt_id: u64,
    pub route_id: u64,
    pub source_generation: u64,
    pub target_generation: u64,
    pub actor_id: u32,
    pub outcome: RemoteReceiptOutcome,
    pub code: RemoteReceiptCode,
}
impl RemoteReceiptHeader {
    pub const fn is_valid(self) -> bool {
        self.receipt_id > 0 && self.route_id > 0 &&
        self.source_generation > 0 && self.target_generation > 0 &&
        self.actor_id <= 64 &&
        matches!((self.outcome, self.code),
            (RemoteReceiptOutcome::Processed,RemoteReceiptCode::Ok) |
            (RemoteReceiptOutcome::Rejected,RemoteReceiptCode::RouteStale) |
            (RemoteReceiptOutcome::Rejected,RemoteReceiptCode::RouteBackpressure) |
            (RemoteReceiptOutcome::Rejected,RemoteReceiptCode::RouteUnavailable) |
            (RemoteReceiptOutcome::Rejected,RemoteReceiptCode::RouteRevoked) |
            (RemoteReceiptOutcome::Rejected,RemoteReceiptCode::RouteProtocol) |
            (RemoteReceiptOutcome::Rejected,RemoteReceiptCode::RouteUnknown) |
            (RemoteReceiptOutcome::Unknown,RemoteReceiptCode::RouteStale) |
            (RemoteReceiptOutcome::Unknown,RemoteReceiptCode::RouteBackpressure) |
            (RemoteReceiptOutcome::Unknown,RemoteReceiptCode::RouteTimeout) |
            (RemoteReceiptOutcome::Unknown,RemoteReceiptCode::RouteUnavailable) |
            (RemoteReceiptOutcome::Unknown,RemoteReceiptCode::RouteCrash) |
            (RemoteReceiptOutcome::Unknown,RemoteReceiptCode::RouteReceiver) |
            (RemoteReceiptOutcome::Unknown,RemoteReceiptCode::RouteProtocol) |
            (RemoteReceiptOutcome::Unknown,RemoteReceiptCode::RouteUnknown))
    }
}
pub const MAX_ACTOR_TYPES_PER_TENANT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserTenantArtifact {
    pub wasm: Vec<u8>,
    pub wire_protocol: &'static str,
    pub guest_abi: &'static str,
    /// Receipt/error schema consumed by the browser host; no async guest ABI.
    pub send_receipt_protocol: &'static str,
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
        send_receipt_protocol: SEND_RECEIPT_PROTOCOL,
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
    let mut seen = HashSet::with_capacity(count);
    for _ in 0..count {
        let len = read_uleb(payload, &mut index).ok_or(())? as usize;
        let end = index.checked_add(len).filter(|end| *end <= payload.len()).ok_or(())?;
        let name = std::str::from_utf8(&payload[index..end]).map_err(|_| ())?;
        if name.is_empty() || name.len() > 256 || !seen.insert(name.to_owned()) { return Err(()); }
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
    fn duplicate_actor_metadata_is_rejected() {
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        let mut custom = vec![11u8];
        custom.extend_from_slice(b"atto.actors");
        custom.extend_from_slice(&[2, 1, b'A', 1, b'A']);
        wasm.push(0);
        wasm.push(custom.len() as u8);
        wasm.extend_from_slice(&custom);
        assert!(read_actor_names(&wasm).is_err());
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
