//! Candidate \`atto.browser.http.v1\` async request/resume wire contract.
//!
//! This module is a typed compiler/runtime *contract* and a bounded,
//! generation-fenced reference admission table. It is NOT a Wasm lowering pass
//! and does not grant HTTP imports to emitted \`atto.browser.guest.v2\` modules.
//! See the stacked website PR for a separately tested, non-shipped JS/Wasm host
//! harness. Do not promote an Atto \`await fetch()\` preview based on this code.

use std::collections::BTreeSet;

pub const VERSION_SECTION: &str = "atto.http.v1";
pub const VERSION_MARKER: &[u8] = &[1];
pub const IMPORT_MODULE: &str = "atto_http_v1";
pub const IMPORT_REQUEST: &str = "request";
pub const IMPORT_CANCEL: &str = "cancel";
pub const EXPORT_RESERVE: &str = "atto_http_v1_reserve";
pub const EXPORT_RESUME: &str = "atto_http_v1_resume";
pub const MAX_REQUESTS: usize = 64;
pub const MAX_URL_BYTES: usize = 2048;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_RESPONSE_BYTES: usize = 1024 * 1024;
pub const DEFAULT_TIMEOUT_MS: u32 = 30_000;
pub const MAX_TIMEOUT_MS: u32 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    I32,
}
pub const REQUEST_PARAMS: [ValueType; 3] = [ValueType::I32; 3];
pub const REQUEST_RESULTS: [ValueType; 1] = [ValueType::I32];
pub const CANCEL_PARAMS: [ValueType; 1] = [ValueType::I32];
pub const CANCEL_RESULTS: [ValueType; 1] = [ValueType::I32];
pub const RESERVE_PARAMS: [ValueType; 1] = [ValueType::I32];
pub const RESERVE_RESULTS: [ValueType; 1] = [ValueType::I32];
pub const RESUME_PARAMS: [ValueType; 5] = [ValueType::I32; 5];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Admission {
    Accepted = 1,
    BadRequest = 2,
    Closed = 3,
    Busy = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CompletionTag {
    Ok = 0,
    Error = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ErrorCode {
    Permission = 1,
    Aborted = 2,
    Timeout = 3,
    Quota = 4,
    Size = 5,
    Redirect = 6,
    Opaque = 7,
    Body = 8,
    Closed = 9,
    Transport = 255,
}

/// A positive i32 guest request number, scoped to one physical Wasm instance.
/// Never identify a request using this number alone across actor restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestId(i32);
impl RequestId {
    pub fn new(value: i32) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }

    pub fn value(self) -> i32 {
        self.0
    }
}

/// Opaque host-side binding for one actor incarnation + its guest request ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    pub incarnation: u64,
    pub id: RequestId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketError {
    InvalidConfiguration,
    InvalidId,
    Duplicate,
    Busy,
    Closed,
    Stale,
}

/// A deterministic reference admission table for compiler/host conformance.
/// A real host still needs cancellation, deadline and physical lifetime fencing.
#[derive(Debug)]
pub struct PendingRequests {
    incarnation: u64,
    max: usize,
    active: BTreeSet<RequestId>,
    closed: bool,
}
impl PendingRequests {
    pub fn new(incarnation: u64, max: usize) -> Result<Self, TicketError> {
        if incarnation == 0 || max == 0 || max > MAX_REQUESTS {
            return Err(TicketError::InvalidConfiguration);
        }
        Ok(Self {
            incarnation,
            max,
            active: BTreeSet::new(),
            closed: false,
        })
    }

    pub fn admit(&mut self, raw_id: i32) -> Result<Ticket, TicketError> {
        if self.closed {
            return Err(TicketError::Closed);
        }
        let id = RequestId::new(raw_id).ok_or(TicketError::InvalidId)?;
        if self.active.contains(&id) {
            return Err(TicketError::Duplicate);
        }
        if self.active.len() >= self.max {
            return Err(TicketError::Busy);
        }
        self.active.insert(id);
        Ok(Ticket {
            incarnation: self.incarnation,
            id,
        })
    }

    pub fn settle(&mut self, ticket: Ticket) -> Result<(), TicketError> {
        if self.closed {
            return Err(TicketError::Closed);
        }
        if ticket.incarnation != self.incarnation || !self.active.remove(&ticket.id) {
            return Err(TicketError::Stale);
        }
        Ok(())
    }

    /// Irrevocably retire all pending requests; never call back the old guest.
    pub fn revoke(&mut self) -> usize {
        self.closed = true;
        let n = self.active.len();
        self.active.clear();
        n
    }

    pub fn pending(&self) -> usize {
        self.active.len()
    }
}

/// Validate the memory slice returned by the guest's reserve(len) export.
/// This guards integer overflow and wrong-memory/dangling response pointers.
pub fn validate_response_slice(
    ptr: u32,
    len: u32,
    guest_memory_bytes: u32,
    max_response_bytes: u32,
) -> Result<(), TicketError> {
    if len > max_response_bytes || len > MAX_RESPONSE_BYTES as u32 ||
        ptr > guest_memory_bytes || len > guest_memory_bytes - ptr {
        return Err(TicketError::InvalidConfiguration);
    }
    Ok(())
}

/// Error completions carry no pointer or length; successful responses have
/// HTTP status (200..599) and an in-bounds bounded guest-owned buffer.
pub fn validate_completion(
    tag: CompletionTag,
    code: i32,
    ptr: u32,
    len: u32,
    guest_memory_bytes: u32,
    max_response_bytes: u32,
) -> Result<(), TicketError> {
    match tag {
        CompletionTag::Ok if (200..=599).contains(&code) => {
            validate_response_slice(ptr, len, guest_memory_bytes, max_response_bytes)
        }
        CompletionTag::Error if code > 0 && ptr == 0 && len == 0 => Ok(()),
        _ => Err(TicketError::InvalidConfiguration),
    }
}
