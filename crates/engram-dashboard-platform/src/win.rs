//! Small Win32 helpers that the Windows halves of several modules share — a null-terminated wide string, the windows-rs error as an
//!   `io::Error` with a usable `kind()`, and memory the OS hands out with `LocalAlloc`.

// ADR-0266

use std::ffi::{c_void, OsStr};
use std::io;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{LocalFree, HLOCAL};

/// `s` as a null-terminated UTF-16 string. An inner NUL would end the string early at the OS, so it is refused as `InvalidInput`.
pub(crate) fn to_wide(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = s.encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the string contains NUL, so it cannot be passed as a null-terminated string",
        ));
    }
    wide.push(0);
    Ok(wide)
}

/// A windows-rs error as an `io::Error`. A FACILITY_WIN32 HRESULT becomes its Win32 code, so `kind()` reads it (`AlreadyExists`,
///   `PermissionDenied` …); any other HRESULT stays whole in `Other` — `from_raw_os_error` on an HRESULT garbles the message.
pub(crate) fn io_error(e: windows::core::Error) -> io::Error {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((hr & 0xFFFF) as i32)
    } else {
        io::Error::other(e)
    }
}

/// Memory the OS allocated with `LocalAlloc` (security descriptors, SID strings, DPAPI output), freed on drop.
pub(crate) struct LocalMem(*mut c_void);

impl LocalMem {
    /// # Safety
    /// `memory` is null or came from an API documented to allocate it with `LocalAlloc`, and nothing else frees it.
    pub(crate) unsafe fn new(memory: *mut c_void) -> Self {
        Self(memory)
    }
}

impl Drop for LocalMem {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `new`'s contract — LocalAlloc'd memory freed exactly once. LocalFree only fails for an invalid pointer, which the
            //   contract rules out.
            unsafe {
                let _ = LocalFree(HLOCAL(self.0));
            }
        }
    }
}
