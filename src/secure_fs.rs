//! Linux descriptor-relative, symlink-free package file reads.
//!
//! Every path component is opened relative to an already-open directory FD
//! with O_NOFOLLOW and O_DIRECTORY where appropriate. File content is read
//! from the returned descriptor, not via a previously checked path. This
//! removes symlink-swap and directory-replacement TOCTOU escapes for this
//! profile. It is Linux-only and needs separate audited platform ports.
#![cfg(target_os = "linux")]
use crate::diag::{Diagnostic, Pos};
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Component, Path};
use std::os::raw::{c_char, c_int};

type R<T> = Result<T, Diagnostic>;
const O_RDONLY: c_int = 0;
const O_DIRECTORY: c_int = 0o200000;
const O_NOFOLLOW: c_int = 0o400000;
const O_CLOEXEC: c_int = 0o2000000;

extern "C" {
    fn openat(dirfd: c_int, pathname: *const c_char, flags: c_int, ...) -> c_int;
}

fn fail(code: &'static str, detail: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Pos::default(), code, detail)
}
fn open_child(directory: &File, name: &str, directory_only: bool, code: &'static str) -> R<File> {
    let c_name = CString::new(name)
        .map_err(|_| fail(code, "NUL in package file component"))?;
    let flags = O_RDONLY | O_NOFOLLOW | O_CLOEXEC |
        if directory_only { O_DIRECTORY } else { 0 };
    // SAFETY: openat's name is a NUL-terminated CString, the directory FD is
    // alive for the duration of this call, and no mode is required for an
    // O_RDONLY open. Ownership of a successful raw FD transfers to File.
    let fd = unsafe { openat(directory.as_raw_fd(), c_name.as_ptr(), flags) };
    if fd < 0 {
        return Err(fail(code, format!("cannot open checked component '{name}': {}",
            std::io::Error::last_os_error())));
    }
    // SAFETY: the successful openat result is a new, uniquely owned FD.
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub struct LockedRoot { directory: File }

impl LockedRoot {
    pub fn open(path: &Path) -> R<Self> {
        let absolute = if path.is_absolute() { path.to_path_buf() } else {
            std::env::current_dir().map_err(|e| fail("E_IMPORT_PATH",
                format!("cannot locate current directory: {e}")))?.join(path)
        };
        let mut current = File::open("/").map_err(|e| fail("E_IMPORT_PATH",
            format!("cannot open filesystem root: {e}")))?;
        for component in absolute.components() {
            match component {
                Component::RootDir | Component::CurDir => {},
                Component::Normal(name) => {
                    let name = name.to_str().ok_or_else(|| fail("E_IMPORT_PATH",
                        "non-UTF8 package directory name"))?;
                    current = open_child(&current, name, true, "E_IMPORT_PATH")?;
                }
                _ => return Err(fail("E_IMPORT_PATH",
                    "package root must have no parent traversal or platform prefixes")),
            }
        }
        if !current.metadata().map_err(|e| fail("E_IMPORT_PATH",
            format!("cannot inspect checked package root: {e}")))?.is_dir() {
            return Err(fail("E_IMPORT_PATH", "package root must be a directory"));
        }
        Ok(Self { directory: current })
    }

    pub fn read(&self, relative: &str, max_bytes: usize,
                diagnostic_code: &'static str) -> R<String> {
        if relative.is_empty() || relative.starts_with('/') ||
           relative.contains('\\') || relative.contains(':') ||
           relative.split('/').any(|part| part.is_empty() || part == "." ||
               part == ".." || part.contains('\0')) {
            return Err(fail(diagnostic_code, "invalid descriptor-relative source path"));
        }
        let mut directory: Option<File> = None;
        let mut parts = relative.split('/').peekable();
        let mut file: Option<File> = None;
        while let Some(name) = parts.next() {
            let parent = directory.as_ref().unwrap_or(&self.directory);
            if parts.peek().is_some() {
                directory = Some(open_child(parent, name, true, diagnostic_code)?);
            } else {
                file = Some(open_child(parent, name, false, diagnostic_code)?);
            }
        }
        let mut file = file.ok_or_else(|| fail(diagnostic_code, "empty source file path"))?;
        let metadata = file.metadata().map_err(|e| fail(diagnostic_code,
            format!("cannot stat pinned source FD: {e}")))?;
        if !metadata.is_file() {
            return Err(fail(diagnostic_code, "package source must be a regular file"));
        }
        if metadata.len() > max_bytes as u64 {
            return Err(fail(if diagnostic_code == "E_MANIFEST" { "E_MANIFEST" }
                else { "E_SOURCE_LIMIT" }, "package source exceeds file size budget"));
        }
        let mut bytes = Vec::new();
        file.take((max_bytes as u64) + 1).read_to_end(&mut bytes)
            .map_err(|e| fail(diagnostic_code,
                format!("cannot read pinned source descriptor: {e}")))?;
        if bytes.len() > max_bytes {
            return Err(fail(if diagnostic_code == "E_MANIFEST" { "E_MANIFEST" }
                else { "E_SOURCE_LIMIT" }, "package source grew beyond budget"));
        }
        String::from_utf8(bytes).map_err(|_| fail(diagnostic_code,
            "package source is not valid UTF-8"))
    }
}
