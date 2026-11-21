//! Which OS account and machine this process runs on, and which account owns one end of a local TCP connection —
//!   [`current_account_id`], [`machine_id`], [`tcp_client_account`] / [`tcp_server_account`]. Account ids are one text form everywhere in this module, so a caller
//!   compares them with `==`.
//!
//! Windows only for now: outside Windows every function is `Unsupported` without trying (no OS code); the behaviour defined for other OSes
//!   is written on each function and is built with T-118 / T-119.

// ADR-0266, ADR-0389, ADR-0395

use std::fmt;
use std::io;
use std::net::SocketAddrV4;

#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

#[cfg(windows)]
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HANDLE, NO_ERROR,
};
#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPROW_OWNER_MODULE, MIB_TCPTABLE_OWNER_MODULE, MIB_TCP_STATE_ESTAB,
    MIB_TCP_STATE_LISTEN, TCP_TABLE_OWNER_MODULE_ALL,
};
#[cfg(windows)]
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
#[cfg(windows)]
use windows::Win32::Security::{GetTokenInformation, TokenUser, PSID, TOKEN_QUERY, TOKEN_USER};
#[cfg(windows)]
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[cfg(windows)]
use crate::win::{io_error, LocalMem};

/// The id of the OS account this process runs under. Windows = the user SID of the process token as text (`S-1-5-21-…`) — the same
///   text [`tcp_client_account`] / [`tcp_server_account`] return. Outside Windows = `Unsupported` (defined: the numeric uid as text — T-118 / T-119).
#[cfg(windows)]
pub fn current_account_id() -> io::Result<String> {
    // SAFETY: the pseudo-handle of this process needs no closing; the token handle goes straight into an owner that closes it.
    let token = unsafe {
        let mut raw = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw).map_err(io_error)?;
        OwnedHandle::from_raw_handle(raw.0)
    };
    token_user_sid(&token)
}
#[cfg(not(windows))]
pub fn current_account_id() -> io::Result<String> {
    Err(io::ErrorKind::Unsupported.into())
}

/// This machine's id. Windows = `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`, read from the 64-bit registry view (a 32-bit build
///   would otherwise read the WOW6432Node view, which has no such value). Returned as stored and not validated — a caller that puts it in
///   a file name checks its characters. A missing or empty value is an error. Outside Windows = `Unsupported` (defined: Linux
///   `/etc/machine-id` — T-118 / T-119).
#[cfg(windows)]
pub fn machine_id() -> io::Result<String> {
    let subkey = windows::core::w!("SOFTWARE\\Microsoft\\Cryptography");
    let value = windows::core::w!("MachineGuid");
    let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY;
    let mut bytes: u32 = 0;
    // SAFETY: constant null-terminated names; asks only the size into a stack u32.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey,
            value,
            flags,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(rc.0 as i32));
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: `buf` holds `bytes` bytes; RegGetValueW null-terminates a REG_SZ inside that size.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey,
            value,
            flags,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(rc.0 as i32));
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let id = String::from_utf16(&buf[..len])
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "MachineGuid is empty",
        ));
    }
    Ok(id)
}
#[cfg(not(windows))]
pub fn machine_id() -> io::Result<String> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Why [`tcp_client_account`] / [`tcp_server_account`] gave no account. ★Every variant means "cannot verify"★ — a caller deciding trust
///   on the answer refuses on any of them (fail closed); they are apart only so the caller can word the refusal.
#[derive(Debug)]
pub enum TcpOwnerError {
    /// No established TCP connection with these two ends exists on this machine (never opened, already gone, or one end is on another
    ///   machine), or its row has no owning process (PID 0), or the connection changed while it was being read.
    NoSuchConnection,
    /// The table names an owner PID that cannot be proven to be the asked end's process: the process holding that PID now was created
    ///   after the endpoint (its creator exited — another process can keep the socket, the table keeps the creator's PID — and the number
    ///   was reused); or a server end has no listening row that predates it; or the end asked about as a client is a server end (its
    ///   owner listens on that port — the two addresses were passed the wrong way round).
    OwnerUnproven,
    /// Opening the owning process was refused (access denied) — what a process of another account gives: a standard account's process
    ///   cannot be opened by another account at all, while this account's own processes (elevated ones included) open for the query
    ///   this lookup needs. The account is not read, so this is still not a proof — callers word it as "another account" and refuse.
    ProcessAccessDenied,
    /// The TCP table could not be read, or the owning process could not be opened for another reason, or its creation time or token
    ///   could not be read (it exited, or access was denied after opening — e.g. an elevated process's token asked about by a
    ///   non-elevated one). Outside Windows = `Unsupported`.
    Os(io::Error),
}

impl TcpOwnerError {
    /// The connection or its owning process went away during the lookup — no such connection (any more), or the owner PID names no
    ///   process now (Windows `ERROR_INVALID_PARAMETER` from opening it). A later attempt can answer differently, so a caller may treat
    ///   it as one failed attempt; it is still never a pass. Access denied (e.g. elevation) and an unproven owner are not gone.
    pub fn is_gone(&self) -> bool {
        // The OS code of a PID that no longer names a process, as `OpenProcess` reports it.
        const ERROR_INVALID_PARAMETER: i32 = 87;
        match self {
            Self::NoSuchConnection => true,
            Self::Os(e) => cfg!(windows) && e.raw_os_error() == Some(ERROR_INVALID_PARAMETER),
            Self::OwnerUnproven | Self::ProcessAccessDenied => false,
        }
    }
}

impl fmt::Display for TcpOwnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchConnection => {
                f.write_str("no established TCP connection with these two ends on this machine")
            }
            Self::OwnerUnproven => {
                f.write_str("the process owning this end of the connection could not be confirmed")
            }
            Self::ProcessAccessDenied => f.write_str(
                "the process owning this end of the connection may not be opened (another account's process)",
            ),
            Self::Os(e) => write!(f, "could not read the connection's owning account: {e}"),
        }
    }
}

impl std::error::Error for TcpOwnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoSuchConnection | Self::OwnerUnproven | Self::ProcessAccessDenied => None,
            Self::Os(e) => Some(e),
        }
    }
}

/// The account (the text of [`current_account_id`]) running the **client** end of the established IPv4 TCP connection from `client`
///   to `server` — what a server asks about a connection it accepted (`client` = the observed peer address, `server` = its own address).
///   Both ends must be on this machine (the table lists only local sockets), so in practice both are loopback. How the owner is proven:
///   [`tcp_server_account`]. Outside Windows = `Os(Unsupported)` (defined: the socket owner's uid — T-118 / T-119).
#[cfg(windows)]
pub fn tcp_client_account(
    client: SocketAddrV4,
    server: SocketAddrV4,
) -> Result<String, TcpOwnerError> {
    owner(client, server, End::Client, real_creation_time).map(|(_, account)| account)
}
#[cfg(not(windows))]
pub fn tcp_client_account(
    _client: SocketAddrV4,
    _server: SocketAddrV4,
) -> Result<String, TcpOwnerError> {
    Err(TcpOwnerError::Os(io::ErrorKind::Unsupported.into()))
}

/// The account running the **server** end of the established IPv4 TCP connection between `server` and `client` — what a client asks
///   about its own connection (`server` = the address it connected to, `client` = its own local address; ask after connecting, so the
///   port cannot change hands in between).
///
/// How the owner is proven (both functions): the TCP-table row of that end (state established) names a PID and the time the endpoint
///   was made; the process is opened, the row is read again and must still name that PID (while the handle is open the PID cannot be
///   reused), and the process must have been created no later than the endpoint — ★the table keeps the creating process's PID even
///   after it exits while another process (a child it shared the socket with) still holds the socket★, so a later process that took the
///   number is refused ([`TcpOwnerError::OwnerUnproven`]). Dating ([`date_endpoint`]): a connecting socket's row carries its creation
///   time; an accepted socket's row carries the accept time, so a server end is dated by its listening row instead. The two addresses
///   passed the wrong way round end in `OwnerUnproven` both ways (a client end has no listener; a server end asked as a client has one).
///   Outside Windows = `Os(Unsupported)` (defined: the socket owner's uid — T-118 / T-119).
#[cfg(windows)]
pub fn tcp_server_account(
    server: SocketAddrV4,
    client: SocketAddrV4,
) -> Result<String, TcpOwnerError> {
    owner(server, client, End::Server, real_creation_time).map(|(_, account)| account)
}
#[cfg(not(windows))]
pub fn tcp_server_account(
    _server: SocketAddrV4,
    _client: SocketAddrV4,
) -> Result<String, TcpOwnerError> {
    Err(TcpOwnerError::Os(io::ErrorKind::Unsupported.into()))
}

/// Which end of a connection is asked about — it decides how the endpoint is dated ([`date_endpoint`]).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Client,
    Server,
}

#[cfg(windows)]
fn real_creation_time(process: HANDLE) -> Option<u64> {
    crate::process::creation_time_of(process)
}

/// The owning PID and account of the `local` end of the connection `local` ↔ `remote` (the proof: [`tcp_server_account`]).
///   `creation_time` reads the opened process's creation time — the tests pass a fake to drive the time check on a real connection.
#[cfg(windows)]
fn owner(
    local: SocketAddrV4,
    remote: SocketAddrV4,
    end: End,
    creation_time: impl FnOnce(HANDLE) -> Option<u64>,
) -> Result<(u32, String), TcpOwnerError> {
    let first = match endpoint(local, remote, end)? {
        Some(found) if found.pid != 0 => found,
        _ => return Err(TcpOwnerError::NoSuchConnection),
    };
    // SAFETY: opens with the least right that reads a creation time and a token's owner; the handle goes straight into an owner that
    //   closes it.
    let process = unsafe {
        let raw = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, first.pid)
            .map_err(|e| denied_or_os(io_error(e)))?;
        OwnedHandle::from_raw_handle(raw.0)
    };
    let held = match endpoint(local, remote, end)? {
        Some(found) if found.pid == first.pid => found,
        _ => return Err(TcpOwnerError::NoSuchConnection),
    };
    let created = creation_time(HANDLE(process.as_raw_handle())).ok_or_else(|| {
        TcpOwnerError::Os(io::Error::other(
            "the owning process's creation time is unreadable",
        ))
    })?;
    if !created_in_time(created, held.birth) {
        return Err(TcpOwnerError::OwnerUnproven);
    }
    // SAFETY: the process handle is open for the whole call; the token handle goes straight into an owner that closes it.
    let token = unsafe {
        let mut raw = HANDLE::default();
        OpenProcessToken(HANDLE(process.as_raw_handle()), TOKEN_QUERY, &mut raw)
            .map_err(|e| TcpOwnerError::Os(io_error(e)))?;
        OwnedHandle::from_raw_handle(raw.0)
    };
    let account = token_user_sid(&token).map_err(TcpOwnerError::Os)?;
    Ok((first.pid, account))
}

/// An `OpenProcess` failure as a [`TcpOwnerError`]: access denied is [`TcpOwnerError::ProcessAccessDenied`] (another account's process);
///   anything else stays an OS error.
#[cfg_attr(not(windows), allow(dead_code))]
fn denied_or_os(e: io::Error) -> TcpOwnerError {
    #[cfg(windows)]
    if e.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) {
        return TcpOwnerError::ProcessAccessDenied;
    }
    TcpOwnerError::Os(e)
}

/// One end as the table reports it: the owner PID and the endpoint's time (FILETIME scale).
#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
struct Endpoint {
    pid: u32,
    birth: i64,
}

#[cfg(windows)]
fn endpoint(
    local: SocketAddrV4,
    remote: SocketAddrV4,
    end: End,
) -> Result<Option<Endpoint>, TcpOwnerError> {
    let buf = tcp_table().map_err(TcpOwnerError::Os)?;
    date_endpoint(rows(&buf).map_err(TcpOwnerError::Os)?, local, remote, end)
}

/// The established row with local address `local` and remote address `remote` in `rows`, dated per `end`. `Ok(None)` = no such row.
/// - Client end: dated by its own row — a connecting socket's row carries the socket's creation time (measured 2026-11-20: a socket made
///   or bound by one process and connected later by another keeps the earlier time). A LISTEN row on the same port with the same PID
///   means this end is a server end → `OwnerUnproven`.
/// - Server end: an accepted socket's row carries the accept time, so it is dated by the earliest LISTEN row with the same PID and port,
///   on the same local address or the wildcard, made no later than the accept (a server listens before it accepts) — none →
///   `OwnerUnproven`.
#[cfg(windows)]
fn date_endpoint(
    rows: &[MIB_TCPROW_OWNER_MODULE],
    local: SocketAddrV4,
    remote: SocketAddrV4,
    end: End,
) -> Result<Option<Endpoint>, TcpOwnerError> {
    let Some(row) = rows.iter().find(|r| {
        r.dwState == MIB_TCP_STATE_ESTAB.0 as u32
            && address(r.dwLocalAddr, r.dwLocalPort) == local
            && address(r.dwRemoteAddr, r.dwRemotePort) == remote
    }) else {
        return Ok(None);
    };
    let mut listeners = rows.iter().filter(|r| {
        r.dwState == MIB_TCP_STATE_LISTEN.0 as u32
            && r.dwOwningPid == row.dwOwningPid
            && address(r.dwLocalAddr, r.dwLocalPort).port() == local.port()
    });
    let birth = match end {
        End::Client => {
            if listeners.next().is_some() {
                return Err(TcpOwnerError::OwnerUnproven);
            }
            row.liCreateTimestamp
        }
        End::Server => listeners
            .filter(|r| {
                let at = *address(r.dwLocalAddr, r.dwLocalPort).ip();
                (at == *local.ip() || at.is_unspecified())
                    && r.liCreateTimestamp <= row.liCreateTimestamp
            })
            .map(|r| r.liCreateTimestamp)
            .min()
            .ok_or(TcpOwnerError::OwnerUnproven)?,
    };
    Ok(Some(Endpoint {
        pid: row.dwOwningPid,
        birth,
    }))
}

/// Was the process created no later than the endpoint (both FILETIME scale, 100 ns)? An unset (zero or negative) time proves nothing.
#[cfg_attr(not(windows), allow(dead_code))]
fn created_in_time(process_created: u64, endpoint_birth: i64) -> bool {
    endpoint_birth > 0 && process_created <= endpoint_birth as u64
}

/// The whole IPv4 TCP table with owner PIDs and creation times (`TCP_TABLE_OWNER_MODULE_ALL` — listening rows included), in u64 storage
///   so the 8-byte timestamps are aligned.
#[cfg(windows)]
fn tcp_table() -> io::Result<Vec<u64>> {
    const AF_INET: u32 = 2;
    // The table can grow between asking its size and reading it, so the size is asked a few times before giving up.
    const ATTEMPTS: u32 = 4;
    let mut buf: Vec<u64> = Vec::new();
    for _ in 0..ATTEMPTS {
        let mut bytes = (buf.len() * 8) as u32;
        let out = if buf.is_empty() {
            None
        } else {
            Some(buf.as_mut_ptr().cast())
        };
        // SAFETY: `buf` holds `bytes` bytes (None/0 when empty); the call writes at most that and reports the size it needs.
        let rc = unsafe {
            GetExtendedTcpTable(
                out,
                &mut bytes,
                false,
                AF_INET,
                TCP_TABLE_OWNER_MODULE_ALL,
                0,
            )
        };
        if rc == NO_ERROR.0 && !buf.is_empty() {
            return Ok(buf);
        }
        if rc != ERROR_INSUFFICIENT_BUFFER.0 && rc != NO_ERROR.0 {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        buf = vec![0u64; (bytes as usize).div_ceil(8) + 1];
    }
    Err(io::Error::other(format!(
        "GetExtendedTcpTable did not settle on a buffer size within {ATTEMPTS} attempts"
    )))
}

/// The rows of the `MIB_TCPTABLE_OWNER_MODULE` in `buf`, bounded by the buffer — a count larger than the buffer is an error, not a read
///   past it.
#[cfg(windows)]
fn rows(buf: &[u64]) -> io::Result<&[MIB_TCPROW_OWNER_MODULE]> {
    let table = buf.as_ptr().cast::<MIB_TCPTABLE_OWNER_MODULE>();
    // SAFETY: `buf` is 8-byte aligned and non-empty (`tcp_table` returns only a filled buffer), so the count field is inside it.
    let count = unsafe { (*table).dwNumEntries } as usize;
    let header = std::mem::offset_of!(MIB_TCPTABLE_OWNER_MODULE, table);
    let fits =
        (buf.len() * 8).saturating_sub(header) / std::mem::size_of::<MIB_TCPROW_OWNER_MODULE>();
    if count > fits {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the TCP table reports more rows than its buffer holds",
        ));
    }
    // SAFETY: the row array starts at `header` inside `buf` (the pointer keeps the whole buffer's provenance — `addr_of!`, not a
    //   reference to the one-element field), is aligned, and `count` rows fit (checked above); zero rows give an empty slice over a
    //   valid pointer.
    Ok(unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!((*table).table).cast::<MIB_TCPROW_OWNER_MODULE>(),
            count,
        )
    })
}

/// Addresses are in network byte order in memory and ports in network byte order in the low 16 bits (IP Helper's documented layout).
#[cfg(windows)]
fn address(raw: u32, port: u32) -> SocketAddrV4 {
    SocketAddrV4::new(raw.to_ne_bytes().into(), u16::from_be(port as u16))
}

/// The user SID of an open token, as text.
#[cfg(windows)]
fn token_user_sid(token: &OwnedHandle) -> io::Result<String> {
    let token = HANDLE(token.as_raw_handle());
    let mut bytes: u32 = 0;
    // SAFETY: asks only the size; the expected failure is ERROR_INSUFFICIENT_BUFFER with `bytes` set.
    match unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut bytes) } {
        Err(e) if e.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {}
        Err(e) => return Err(io_error(e)),
        Ok(()) => {
            return Err(io::Error::other(
                "GetTokenInformation(TokenUser) asked no buffer",
            ))
        }
    }
    // u64 storage keeps TOKEN_USER (pointer-aligned) aligned.
    let mut buf = vec![0u64; (bytes as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `bytes` bytes, aligned for TOKEN_USER.
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            bytes,
            &mut bytes,
        )
    }
    .map_err(io_error)?;
    // SAFETY: the call above filled a TOKEN_USER at the start of `buf`; its SID points inside `buf`, which outlives the conversion.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    sid_to_string(sid)
}

/// A SID as its `S-1-…` text.
#[cfg(windows)]
pub(crate) fn sid_to_string(sid: PSID) -> io::Result<String> {
    let mut text = windows::core::PWSTR::null();
    // SAFETY: `sid` points at a valid SID for the call; the output string is LocalAlloc'd and freed by `LocalMem`.
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(io_error)?;
    // SAFETY: ConvertSidToStringSidW allocates the string with LocalAlloc; freed once.
    let _free = unsafe { LocalMem::new(text.0.cast()) };
    // SAFETY: on success `text` is a null-terminated wide string.
    unsafe { text.to_string() }.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_the_owner_refused_is_typed_and_other_failures_stay_os_errors() {
        let denied = denied_or_os(io::Error::from_raw_os_error(5));
        if cfg!(windows) {
            assert!(
                matches!(denied, TcpOwnerError::ProcessAccessDenied),
                "{denied:?}"
            );
        } else {
            assert!(matches!(denied, TcpOwnerError::Os(_)), "{denied:?}");
        }
        assert!(matches!(
            denied_or_os(io::Error::from_raw_os_error(87)),
            TcpOwnerError::Os(_)
        ));
    }

    #[test]
    fn only_a_vanished_connection_or_process_is_gone() {
        assert!(TcpOwnerError::NoSuchConnection.is_gone());
        assert!(!TcpOwnerError::OwnerUnproven.is_gone());
        assert!(!TcpOwnerError::ProcessAccessDenied.is_gone());
        assert!(
            !TcpOwnerError::Os(io::Error::from_raw_os_error(5)).is_gone(),
            "access denied"
        );
        assert!(!TcpOwnerError::Os(io::ErrorKind::Unsupported.into()).is_gone());
        assert_eq!(
            TcpOwnerError::Os(io::Error::from_raw_os_error(87)).is_gone(),
            cfg!(windows),
            "a PID that names no process"
        );
    }

    #[test]
    fn account_id_is_a_stable_non_empty_sid_on_windows() {
        let first = current_account_id();
        if cfg!(windows) {
            let first = first.expect("the current account id is readable");
            assert!(first.starts_with("S-1-"), "a SID string: {first}");
            assert_eq!(
                current_account_id().expect("second read"),
                first,
                "stable across calls"
            );
        } else {
            assert_eq!(first.unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
    }

    #[test]
    fn machine_id_is_stable_and_non_empty_on_windows() {
        let first = machine_id();
        if cfg!(windows) {
            let first = first.expect("MachineGuid is readable");
            assert!(!first.is_empty());
            assert_eq!(
                machine_id().expect("second read"),
                first,
                "stable across calls"
            );
        } else {
            assert_eq!(first.unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
    }

    fn v4(a: std::net::SocketAddr) -> SocketAddrV4 {
        match a {
            std::net::SocketAddr::V4(a) => a,
            std::net::SocketAddr::V6(a) => panic!("bound to IPv4 loopback, got {a}"),
        }
    }

    /// A real loopback connection inside this test, its ends as (server side, client side) plus the sockets that keep it open.
    #[cfg(windows)]
    fn loopback() -> (
        SocketAddrV4,
        SocketAddrV4,
        std::net::TcpListener,
        std::net::TcpStream,
        std::net::TcpStream,
    ) {
        use std::net::{Ipv4Addr, TcpListener, TcpStream};

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
        let client =
            TcpStream::connect(listener.local_addr().expect("listening address")).expect("connect");
        let (accepted, observed_client) = listener.accept().expect("accept");
        let server_side = v4(accepted.local_addr().expect("accepted local"));
        let client_side = v4(observed_client);
        assert_eq!(client_side, v4(client.local_addr().expect("client local")));
        (server_side, client_side, listener, client, accepted)
    }

    /// Both ends of a connection inside this test resolve to this test's own account.
    #[cfg(windows)]
    #[test]
    fn both_ends_of_a_loopback_connection_resolve_to_this_account() {
        let (server_side, client_side, _listener, _client, _accepted) = loopback();
        let me = current_account_id().expect("own account");
        let who_is_client =
            tcp_client_account(client_side, server_side).expect("the client end resolves");
        assert_eq!(who_is_client, me);
        let who_is_server =
            tcp_server_account(server_side, client_side).expect("the server end resolves");
        assert_eq!(who_is_server, me);
    }

    /// The two addresses the wrong way round fail closed in both functions — not the asker's own account.
    #[cfg(windows)]
    #[test]
    fn swapped_addresses_are_refused_both_ways() {
        let (server_side, client_side, _listener, _client, _accepted) = loopback();
        let as_client = tcp_client_account(server_side, client_side);
        assert!(
            matches!(as_client, Err(TcpOwnerError::OwnerUnproven)),
            "a server end asked as a client: {as_client:?}"
        );
        let as_server = tcp_server_account(client_side, server_side);
        assert!(
            matches!(as_server, Err(TcpOwnerError::OwnerUnproven)),
            "a client end asked as a server: {as_server:?}"
        );
    }

    /// The time check on a real connection: a process "created" after the endpoint (the fake reader) is refused for either end; the
    ///   real reader passes (the test above).
    #[cfg(windows)]
    #[test]
    fn a_process_created_after_the_endpoint_is_refused() {
        let (server_side, client_side, _listener, _client, _accepted) = loopback();
        let late = |_: HANDLE| Some(u64::MAX);
        for outcome in [
            owner(client_side, server_side, End::Client, late),
            owner(server_side, client_side, End::Server, late),
        ] {
            assert!(
                matches!(outcome, Err(TcpOwnerError::OwnerUnproven)),
                "{outcome:?}"
            );
        }
    }

    /// The client end lives in another process (PowerShell holding a `TcpClient`), so each direction must name its own process: the
    ///   client end the child, the server end this test. The public functions are asked too, so a swap inside either is caught (it
    ///   would land on the refusal of the test above).
    #[cfg(windows)]
    #[test]
    fn each_direction_names_the_process_holding_that_end() {
        use std::net::{Ipv4Addr, TcpListener};
        use std::process::{Command, Stdio};

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
        let port = listener.local_addr().expect("listening address").port();
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "$c = New-Object System.Net.Sockets.TcpClient; $c.Connect('127.0.0.1', {port}); Start-Sleep -Seconds 30"
                ),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("launch powershell");
        listener.set_nonblocking(true).expect("non-blocking accept");
        let (accepted, observed_client) =
            crate::testing::wait_until("the child's connection", || listener.accept().ok());

        let server_side = v4(accepted.local_addr().expect("accepted local"));
        let client_side = v4(observed_client);
        let client_account = tcp_client_account(client_side, server_side);
        let server_account = tcp_server_account(server_side, client_side);
        let client_owner = owner(client_side, server_side, End::Client, real_creation_time);
        let server_owner = owner(server_side, client_side, End::Server, real_creation_time);
        let _ = child.kill();
        let _ = child.wait();

        let me = current_account_id().expect("own account");
        assert_eq!(client_account.expect("the client end resolves"), me);
        assert_eq!(server_account.expect("the server end resolves"), me);
        let (pid, _) = client_owner.expect("the client end resolves");
        assert_eq!(pid, child.id(), "the client end belongs to the child");
        let (pid, _) = server_owner.expect("the server end resolves");
        assert_eq!(
            pid,
            std::process::id(),
            "the server end belongs to this test"
        );
    }

    /// A pair that no connection has — a port pair on the loopback with nothing behind it.
    #[test]
    fn a_pair_with_no_connection_is_no_such_connection() {
        use std::net::Ipv4Addr;

        let nowhere = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1);
        let also_nowhere = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 2);
        for outcome in [
            tcp_client_account(nowhere, also_nowhere),
            tcp_server_account(nowhere, also_nowhere),
        ] {
            if cfg!(windows) {
                assert!(
                    matches!(outcome, Err(TcpOwnerError::NoSuchConnection)),
                    "{outcome:?}"
                );
            } else {
                assert!(
                    matches!(outcome, Err(TcpOwnerError::Os(ref e)) if e.kind() == io::ErrorKind::Unsupported)
                );
            }
        }
    }

    /// One table row, addresses in the table's byte order.
    #[cfg(windows)]
    fn row(
        state: windows::Win32::NetworkManagement::IpHelper::MIB_TCP_STATE,
        local: SocketAddrV4,
        remote: SocketAddrV4,
        pid: u32,
        created: i64,
    ) -> MIB_TCPROW_OWNER_MODULE {
        let raw = |a: SocketAddrV4| {
            (
                u32::from_ne_bytes(a.ip().octets()),
                u32::from(a.port().to_be()),
            )
        };
        let ((local_addr, local_port), (remote_addr, remote_port)) = (raw(local), raw(remote));
        MIB_TCPROW_OWNER_MODULE {
            dwState: state.0 as u32,
            dwLocalAddr: local_addr,
            dwLocalPort: local_port,
            dwRemoteAddr: remote_addr,
            dwRemotePort: remote_port,
            dwOwningPid: pid,
            liCreateTimestamp: created,
            OwningModuleInfo: [0; 16],
        }
    }

    /// A process created after the endpoint is the stale-PID case (the creator exited and the number was reused) — it cannot be staged
    ///   without steering PID reuse; the real-connection test above drives the comparison with a fake creation time, and these drive
    ///   the dating on synthetic rows.
    #[cfg(windows)]
    #[test]
    fn a_client_end_is_dated_by_its_row_and_refused_when_its_owner_listens_there() {
        use std::net::Ipv4Addr;
        let at = |port| SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        let (client, server) = (at(50_000), at(7_000));
        let connection = row(MIB_TCP_STATE_ESTAB, client, server, 10, 500);
        let elsewhere = row(MIB_TCP_STATE_LISTEN, at(7_001), at(0), 10, 100);
        assert_eq!(
            date_endpoint(&[connection, elsewhere], client, server, End::Client).unwrap(),
            Some(Endpoint {
                pid: 10,
                birth: 500
            })
        );
        let same_port = row(MIB_TCP_STATE_LISTEN, at(50_000), at(0), 10, 100);
        assert!(matches!(
            date_endpoint(&[connection, same_port], client, server, End::Client),
            Err(TcpOwnerError::OwnerUnproven)
        ));
        let other_pid = row(MIB_TCP_STATE_LISTEN, at(50_000), at(0), 11, 100);
        assert!(date_endpoint(&[connection, other_pid], client, server, End::Client).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn a_server_end_is_dated_by_a_matching_listener_that_predates_the_accept() {
        use std::net::Ipv4Addr;
        let at = |port| SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        let wildcard = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 7_000);
        let other_address = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 7_000);
        let (server, client) = (at(7_000), at(50_000));
        let accepted = row(MIB_TCP_STATE_ESTAB, server, client, 10, 500);
        let date = |listeners: &[MIB_TCPROW_OWNER_MODULE]| {
            let mut rows = vec![accepted];
            rows.extend_from_slice(listeners);
            date_endpoint(&rows, server, client, End::Server)
        };
        let listening =
            |address, pid, created| row(MIB_TCP_STATE_LISTEN, address, at(0), pid, created);

        assert_eq!(
            date(&[listening(server, 10, 100)]).unwrap(),
            Some(Endpoint {
                pid: 10,
                birth: 100
            }),
            "dated by the listener, not the accept"
        );
        assert_eq!(
            date(&[listening(wildcard, 10, 200)]).unwrap(),
            Some(Endpoint {
                pid: 10,
                birth: 200
            }),
            "a wildcard listener counts"
        );
        assert_eq!(
            date(&[listening(server, 10, 300), listening(server, 10, 100)]).unwrap(),
            Some(Endpoint {
                pid: 10,
                birth: 100
            }),
            "the earliest"
        );
        for (what, listeners) in [
            ("no listener", vec![]),
            (
                "a listener made after the accept",
                vec![listening(server, 10, 600)],
            ),
            (
                "a listener on another address",
                vec![listening(other_address, 10, 100)],
            ),
            (
                "a listener of another process",
                vec![listening(server, 11, 100)],
            ),
        ] {
            assert!(
                matches!(date(&listeners), Err(TcpOwnerError::OwnerUnproven)),
                "{what}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_row_in_another_state_is_not_the_connection() {
        use std::net::Ipv4Addr;
        use windows::Win32::NetworkManagement::IpHelper::MIB_TCP_STATE_TIME_WAIT;
        let at = |port| SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        let closing = row(MIB_TCP_STATE_TIME_WAIT, at(50_000), at(7_000), 10, 500);
        assert_eq!(
            date_endpoint(&[closing], at(50_000), at(7_000), End::Client).unwrap(),
            None
        );
    }

    #[test]
    fn only_a_process_created_no_later_than_the_endpoint_is_its_owner() {
        assert!(created_in_time(100, 500));
        assert!(created_in_time(500, 500));
        assert!(
            !created_in_time(501, 500),
            "created after the endpoint = a reused PID"
        );
        assert!(!created_in_time(0, 0), "an unset time proves nothing");
        assert!(!created_in_time(0, -1));
    }
}
