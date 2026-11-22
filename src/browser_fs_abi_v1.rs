//! Candidate Atto browser filesystem request/resume boundary.
//!
//! Host-only typed admission; NOT a compiled Atto FS import or a host OS filesystem.
//! Real Atto .atto stdlib compilation, wasm32 memory snapshotting and safe
//! virtual-filesystem dispatch need separate implementation/evidence.
use std::collections::BTreeMap;

pub const VERSION_SECTION: &str = "atto.fs.v1";
pub const VERSION_MARKER: &[u8] = &[1];
pub const IMPORT_MODULE: &str = "atto_fs_v1";
pub const IMPORT_READ: &str = "read_request";
pub const IMPORT_WRITE: &str = "write_request";
pub const IMPORT_CANCEL: &str = "cancel";
pub const EXPORT_RESERVE: &str = "atto_fs_v1_reserve";
pub const EXPORT_RESUME: &str = "atto_fs_v1_resume";
pub const MAX_PATH_BYTES: usize = 256;
pub const MAX_REQUEST_BYTES: u32 = 1024 * 1024;
pub const MAX_REQUESTS: usize = 64;

/// Wire values are checked, non-negative wasm32 i32 addresses and lengths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueType { I32 }
pub const READ_PARAMS: [ValueType; 3] = [ValueType::I32; 3];
pub const WRITE_PARAMS: [ValueType; 5] = [ValueType::I32; 5];
pub const CANCEL_PARAMS: [ValueType; 1] = [ValueType::I32; 1];
pub const RESERVE_PARAMS: [ValueType; 1] = [ValueType::I32; 1];
pub const RESUME_PARAMS: [ValueType; 5] = [ValueType::I32; 5];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation { Read, Write }
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct RequestId(i32);
impl RequestId {
    pub fn new(id: i32) -> Result<Self, FsError> {
        if id <= 0 { Err(FsError::InvalidId) } else { Ok(Self(id)) }
    }
    pub fn value(self) -> i32 { self.0 }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ticket {
    pub tenant: u64,
    pub incarnation: u64,
    pub id: RequestId,
    pub operation: Operation,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsError {
    InvalidConfiguration,
    InvalidId,
    InvalidPath,
    InvalidUtf8,
    OutOfBounds,
    TooLarge,
    Permission,
    Duplicate,
    Busy,
    Stale,
    Closed,
}

/// Paths denote **virtual** tenant-relative names only, not OS paths.
/// OS mounts additionally require secure dirfd/handle confinement, which
/// cannot be inferred from this lexical predicate.
pub fn validate_relative_path(bytes: &[u8]) -> Result<&str, FsError> {
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES { return Err(FsError::InvalidPath); }
    let path = std::str::from_utf8(bytes).map_err(|_| FsError::InvalidUtf8)?;
    if path.starts_with('/') || path.contains('\\') || path.contains(':') ||
        path.chars().any(char::is_control) { return Err(FsError::InvalidPath); }
    let trimmed = path.strip_prefix("./").unwrap_or(path);
    if trimmed.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return Err(FsError::InvalidPath);
    }
    Ok(trimmed)
}

/// Admission must check the checked region before reading from or writing to
/// guest memory. The host must COPY the bytes under the correct Wasm memory
/// ownership guard and must never keep an unpinned guest slice across await.
pub fn validate_guest_region(ptr: u32, len: u32, memory_bytes: u32,
    max_bytes: u32) -> Result<(), FsError> {
    if len > max_bytes || len > MAX_REQUEST_BYTES { return Err(FsError::TooLarge); }
    if ptr.checked_add(len).is_none_or(|end| end > memory_bytes) {
        return Err(FsError::OutOfBounds);
    }
    Ok(())
}

/// One tenant + actor incarnation reference admission table. Never use the
/// guest-supplied integer request ID as a global actor/tenant handle.
#[derive(Debug)]
pub struct PendingFs {
    tenant: u64,
    incarnation: u64,
    read: bool,
    write: bool,
    max: usize,
    closed: bool,
    active: BTreeMap<RequestId, Operation>,
}
impl PendingFs {
    pub fn new(tenant: u64, incarnation: u64, read: bool, write: bool,
        max: usize) -> Result<Self, FsError> {
        if tenant == 0 || incarnation == 0 || max == 0 || max > MAX_REQUESTS {
            return Err(FsError::InvalidConfiguration);
        }
        Ok(Self {tenant,incarnation,read,write,max,closed:false,active:BTreeMap::new()})
    }
    /// Host must invoke this with a validated/copy-pinned UTF-8 path and
    /// separately validated Wasm memory regions. It does not read guest memory.
    pub fn admit(&mut self, id: i32, operation: Operation, path: &[u8],
        payload_len: u32) -> Result<Ticket, FsError> {
        if self.closed { return Err(FsError::Closed); }
        let id = RequestId::new(id)?;
        if self.active.contains_key(&id) { return Err(FsError::Duplicate); }
        if self.active.len() >= self.max { return Err(FsError::Busy); }
        match operation {
            Operation::Read if !self.read => return Err(FsError::Permission),
            Operation::Write if !self.write => return Err(FsError::Permission),
            _ => {}
        }
        validate_relative_path(path)?;
        if payload_len > MAX_REQUEST_BYTES { return Err(FsError::TooLarge); }
        self.active.insert(id,operation);
        Ok(Ticket {tenant:self.tenant,incarnation:self.incarnation,id,operation})
    }
    pub fn settle(&mut self, ticket: Ticket) -> Result<(), FsError> {
        if self.closed { return Err(FsError::Closed); }
        if ticket.tenant != self.tenant || ticket.incarnation != self.incarnation ||
            self.active.get(&ticket.id).copied() != Some(ticket.operation) {
            return Err(FsError::Stale);
        }
        self.active.remove(&ticket.id);
        Ok(())
    }
    /// Irreversibly fence callbacks for the old tenant/actor incarnation.
    pub fn revoke(&mut self) -> usize {
        self.closed = true;
        let count = self.active.len();
        self.active.clear();
        count
    }
    pub fn pending(&self) -> usize { self.active.len() }
}
