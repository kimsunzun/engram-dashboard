//! Files only the current OS account can open — creating one with that protection in place from the first instant
//!   ([`create_owner_only`]), and checking whether an existing file is still like that ([`is_owner_only`]). Jupyter `secure_write`
//!   precedent: the protection is passed to the create call itself, never set after it.
//!
//! Windows only for now: outside Windows both are `Unsupported` without trying (no OS code); the behaviour defined for other OSes (mode
//!   0600 at create, owner uid checked) is built with T-118 / T-119.

// ADR-0266, ADR-0395

use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle};

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
#[cfg(windows)]
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
#[cfg(windows)]
use windows::Win32::Security::{
    GetAce, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
    DACL_SECURITY_INFORMATION, INHERITED_ACE, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    PSID, SECURITY_ATTRIBUTES, SE_DACL_PROTECTED,
};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetVolumeInformationByHandleW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

#[cfg(windows)]
use crate::account::{current_account_id, sid_to_string};
#[cfg(windows)]
use crate::win::{io_error, to_wide, LocalMem};

/// Well-known SIDs allowed on an owner-only file besides the current account: LocalSystem and BUILTIN\Administrators.
#[cfg(windows)]
const SYSTEM_SID: &str = "S-1-5-18";
#[cfg(windows)]
const ADMINISTRATORS_SID: &str = "S-1-5-32-544";

/// Creates `path` as a new file — `AlreadyExists` if anything is there (`CREATE_NEW`) — whose security goes into the create call
///   itself, so the file never exists with the folder's inherited access: owner = the current account, DACL protected (nothing inherited)
///   granting full access to the current account, SYSTEM and Administrators only. Returns it opened read + write; other opens share read,
///   write and delete, as with `std::fs::File::create`. ★A volume that does not store ACLs (FAT, exFAT) cannot keep the protection★ —
///   the file is removed again and `Unsupported` returned (also when the volume's flags cannot be read); what to do then is the caller's
///   decision.
/// Outside Windows = `Unsupported` (defined: `create_new` with mode 0600 — T-118 / T-119).
#[cfg(windows)]
pub fn create_owner_only(path: &Path) -> io::Result<File> {
    let me = current_account_id()?;
    // O: = owner (set explicitly — an elevated token's default owner is Administrators); D:P = protected DACL; FA = file all access.
    create_with_sddl(
        path,
        &format!("O:{me}D:P(A;;FA;;;{me})(A;;FA;;;SY)(A;;FA;;;BA)"),
    )
}
#[cfg(not(windows))]
pub fn create_owner_only(_path: &Path) -> io::Result<File> {
    Err(io::ErrorKind::Unsupported.into())
}

/// [`create_owner_only`] with the security descriptor given as SDDL — the tests create files of other shapes with it.
#[cfg(windows)]
fn create_with_sddl(path: &Path, sddl: &str) -> io::Result<File> {
    let sddl = to_wide(sddl.as_ref())?;
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: a null-terminated SDDL string and an output pointer on the stack.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(io_error)?;
    // SAFETY: the descriptor ConvertStringSecurityDescriptorToSecurityDescriptorW allocated with LocalAlloc, freed once.
    let _free = unsafe { LocalMem::new(sd.0) };
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let wide = to_wide(path.as_os_str())?;
    // SAFETY: a null-terminated path and security attributes that stay alive for the call; the handle goes straight into a `File`.
    let file = unsafe {
        let handle = CreateFileW(
            PCWSTR(wide.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            Some(&attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        )
        .map_err(io_error)?;
        File::from_raw_handle(handle.0)
    };
    let acls = volume_flags(&file)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!("whether the volume stores file ACLs could not be read: {e}"),
            )
        })
        .and_then(require_persistent_acls);
    match acls {
        Ok(()) => Ok(file),
        Err(e) => {
            drop(file);
            match std::fs::remove_file(path) {
                Ok(()) => Err(e),
                Err(removing) => Err(io::Error::new(
                    e.kind(),
                    format!("{e} — and the file could not be removed again: {removing}"),
                )),
            }
        }
    }
}

/// The file system flags of the volume `file` is on.
#[cfg(windows)]
fn volume_flags(file: &File) -> io::Result<u32> {
    let mut flags: u32 = 0;
    // SAFETY: a handle `file` keeps open; only the flags output is asked for.
    unsafe {
        GetVolumeInformationByHandleW(
            HANDLE(file.as_raw_handle()),
            None,
            None,
            None,
            Some(&mut flags),
            None,
        )
    }
    .map_err(io_error)?;
    Ok(flags)
}

/// `FILE_PERSISTENT_ACLS` must be set — without it the volume (FAT, exFAT) does not keep the DACL passed at create.
#[cfg(windows)]
fn require_persistent_acls(volume_flags: u32) -> io::Result<()> {
    // The literal of FILE_PERSISTENT_ACLS — its windows-rs constant lives behind `Win32_System_SystemServices`, not turned on
    //   (Cargo.toml).
    const FILE_PERSISTENT_ACLS: u32 = 0x8;
    if volume_flags & FILE_PERSISTENT_ACLS == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the volume does not store file ACLs (FAT or exFAT), so the file cannot be made owner-only",
        ));
    }
    Ok(())
}

/// Is the existing file at `path` owner-only in the sense of [`create_owner_only`]: owned by the current account, its DACL present and
///   protected with no inherited entry, and every allow entry for the current account, SYSTEM or Administrators (deny entries only
///   narrow access and are accepted; any other entry type is not; an empty DACL grants no one and counts). `Ok(false)` = a wider or
///   foreign-owned file — callers delete and recreate it; a file whose security this account may not even read (a foreign owner
///   denying it `READ_CONTROL`) is `Ok(false)` too. `Err` = any other failure to read it (a missing file = `NotFound`).
/// Outside Windows = `Unsupported` (defined: mode 0600 and owner uid = the current uid — T-118 / T-119).
#[cfg(windows)]
pub fn is_owner_only(path: &Path) -> io::Result<bool> {
    let me = current_account_id()?;
    match read_acl(path) {
        Ok(acl) => Ok(verdict(&acl, &me)),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) => Ok(false),
        Err(e) => Err(e),
    }
}
#[cfg(not(windows))]
pub fn is_owner_only(_path: &Path) -> io::Result<bool> {
    Err(io::ErrorKind::Unsupported.into())
}

/// The rule of [`is_owner_only`] on a DACL already read back.
#[cfg(windows)]
fn verdict(acl: &FileAcl, me: &str) -> bool {
    acl.owner == me
        && acl.protected
        && acl.entries.as_ref().is_some_and(|entries| {
            entries.iter().all(|e| {
                !e.inherited
                    && match e.kind {
                        EntryKind::Allow => {
                            [me, SYSTEM_SID, ADMINISTRATORS_SID].contains(&e.sid.as_str())
                        }
                        EntryKind::Deny => true,
                        EntryKind::Other => false,
                    }
            })
        })
}

/// A file's owner and DACL as read back from the OS. `entries: None` = a null DACL (everyone has full access).
#[cfg(windows)]
#[derive(Debug, Clone)]
struct FileAcl {
    owner: String,
    protected: bool,
    entries: Option<Vec<Entry>>,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct Entry {
    kind: EntryKind,
    /// The entry's SID text; empty for an `Other` entry, whose layout is not read.
    sid: String,
    inherited: bool,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Allow,
    Deny,
    Other,
}

#[cfg(windows)]
fn read_acl(path: &Path) -> io::Result<FileAcl> {
    // Literals of the Win32 ACE type numbers — their windows-rs constants live behind `Win32_System_SystemServices`, not turned on
    //   (Cargo.toml).
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;

    let wide = to_wide(path.as_os_str())?;
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: a null-terminated path and output pointers on the stack; `owner` and `dacl` point inside `sd`, which `LocalMem` frees
    //   after the last use below.
    let rc = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut sd,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(rc.0 as i32));
    }
    // SAFETY: the descriptor GetNamedSecurityInfoW allocated with LocalAlloc, freed once.
    let _free = unsafe { LocalMem::new(sd.0) };

    let mut control: u16 = 0;
    let mut revision: u32 = 0;
    // SAFETY: `sd` is the valid descriptor returned above.
    unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) }.map_err(io_error)?;
    let owner = if owner.is_invalid() {
        String::new()
    } else {
        sid_to_string(owner)?
    };

    let entries = if dacl.is_null() {
        None
    } else {
        // SAFETY: `dacl` points at the valid ACL inside `sd`.
        let count = unsafe { (*dacl).AceCount };
        let mut entries = Vec::with_capacity(count as usize);
        for index in 0..count as u32 {
            let mut ace: *mut std::ffi::c_void = std::ptr::null_mut();
            // SAFETY: `index` is below the ACL's count; `ace` points inside `sd`.
            unsafe { GetAce(dacl, index, &mut ace) }.map_err(io_error)?;
            // SAFETY: every ACE starts with an ACE_HEADER.
            let header = unsafe { *ace.cast::<ACE_HEADER>() };
            let kind = match header.AceType {
                ACCESS_ALLOWED_ACE_TYPE => EntryKind::Allow,
                ACCESS_DENIED_ACE_TYPE => EntryKind::Deny,
                _ => EntryKind::Other,
            };
            let sid = if kind == EntryKind::Other {
                String::new()
            } else {
                // SAFETY: allowed and denied ACEs share ACCESS_ALLOWED_ACE's layout; the SID starts at `SidStart`.
                let sid =
                    unsafe { std::ptr::addr_of_mut!((*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart) };
                sid_to_string(PSID(sid.cast()))?
            };
            entries.push(Entry {
                kind,
                sid,
                inherited: u32::from(header.AceFlags) & INHERITED_ACE.0 != 0,
            });
        }
        Some(entries)
    };
    Ok(FileAcl {
        owner,
        protected: control & SE_DACL_PROTECTED.0 != 0,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "engram-platform-owner-only-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[cfg(not(windows))]
    #[test]
    fn both_are_unsupported_off_windows() {
        let path = fresh_path();
        assert_eq!(
            create_owner_only(&path).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            is_owner_only(&path).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_created_file_has_only_the_owner_system_and_administrators() {
        use std::io::{Read, Seek, Write};

        let path = fresh_path();
        let mut file = create_owner_only(&path).expect("create");
        file.write_all(b"secret")
            .expect("write through the returned handle");
        file.rewind().expect("rewind");
        let mut back = String::new();
        file.read_to_string(&mut back)
            .expect("read through the returned handle");
        assert_eq!(back, "secret");
        drop(file);
        assert!(path.exists());

        let me = current_account_id().expect("own account");
        let acl = read_acl(&path).expect("read the DACL back");
        assert_eq!(acl.owner, me, "owner = the current account");
        assert!(acl.protected, "inheritance is off");
        let entries = acl.entries.expect("a DACL, not a null one");
        assert!(!entries.is_empty());
        for entry in &entries {
            assert!(!entry.inherited, "nothing inherited: {entry:?}");
            assert_eq!(entry.kind, EntryKind::Allow, "{entry:?}");
            for wide_group in ["S-1-1-0", "S-1-5-32-545", "S-1-5-11"] {
                assert_ne!(
                    entry.sid, wide_group,
                    "no Everyone / Users / Authenticated Users entry"
                );
            }
        }
        let mut sids: Vec<&str> = entries.iter().map(|e| e.sid.as_str()).collect();
        sids.sort_unstable();
        let mut want = vec![me.as_str(), SYSTEM_SID, ADMINISTRATORS_SID];
        want.sort_unstable();
        assert_eq!(sids, want);

        assert!(
            is_owner_only(&path).expect("check"),
            "the check accepts what create made"
        );
        std::fs::remove_file(&path).expect("clean up");
    }

    #[cfg(windows)]
    #[test]
    fn a_second_create_on_the_same_name_fails_with_already_exists() {
        let path = fresh_path();
        drop(create_owner_only(&path).expect("first create"));
        let e = create_owner_only(&path).expect_err("CREATE_NEW refuses an existing name");
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{e:?}");
        std::fs::remove_file(&path).expect("clean up");
    }

    /// No FAT / exFAT volume is at hand, so the flag rule is driven directly; the temp volume (NTFS) passes it in every create above.
    #[cfg(windows)]
    #[test]
    fn a_volume_without_persistent_acls_is_unsupported() {
        let e = require_persistent_acls(0).expect_err("no ACLs on the volume");
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
        require_persistent_acls(0x8).expect("FILE_PERSISTENT_ACLS");
        require_persistent_acls(0x8 | 0x2 | 0x1).expect("other flags do not matter");
    }

    /// A file made the ordinary way inherits the temp folder's entries and is not protected, so the check refuses it even when those
    ///   entries name the same three accounts.
    #[cfg(windows)]
    #[test]
    fn the_check_refuses_a_file_with_inherited_access() {
        let path = fresh_path();
        std::fs::write(&path, b"").expect("ordinary create");
        let acl = read_acl(&path).expect("read the DACL back");
        assert!(!acl.protected, "an ordinary file is not protected: {acl:?}");
        assert!(
            !is_owner_only(&path).expect("check"),
            "an ordinary file is not owner-only"
        );
        std::fs::remove_file(&path).expect("clean up");
    }

    /// Real files of shapes the check must refuse, made with their security at create (no elevation needed); each is owned by this
    ///   account, so only the DACL decides. An unprotected explicit DACL cannot be made this way — a DACL passed at create reads back
    ///   protected (measured 2026-11-20, NTFS) — so the unprotected case is the ordinary file above and the verdict table below.
    #[cfg(windows)]
    #[test]
    fn the_check_refuses_real_files_with_a_wider_or_null_dacl() {
        let me = current_account_id().expect("own account");
        let cases = [
            (
                "Users may read",
                format!("O:{me}D:P(A;;FA;;;{me})(A;;FR;;;BU)"),
            ),
            ("null DACL", format!("O:{me}D:NO_ACCESS_CONTROL")),
        ];
        for (what, sddl) in cases {
            let path = fresh_path();
            drop(create_with_sddl(&path, &sddl).unwrap_or_else(|e| panic!("{what}: create: {e}")));
            let acl = read_acl(&path).unwrap_or_else(|e| panic!("{what}: read back: {e}"));
            assert_eq!(acl.owner, me, "{what}");
            assert!(
                !is_owner_only(&path).expect("check"),
                "{what} must be refused: {acl:?}"
            );
            std::fs::remove_file(&path).expect("clean up");
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_check_of_a_missing_file_is_not_found() {
        let e = is_owner_only(&fresh_path()).expect_err("nothing to read");
        assert_eq!(e.kind(), io::ErrorKind::NotFound, "{e:?}");
    }

    #[cfg(windows)]
    #[test]
    fn the_verdict_accepts_only_the_owner_only_shape() {
        const ME: &str = "S-1-5-21-1-2-3-1001";
        let entry = |kind, sid: &str, inherited| Entry {
            kind,
            sid: sid.to_string(),
            inherited,
        };
        let owner_only = FileAcl {
            owner: ME.to_string(),
            protected: true,
            entries: Some(vec![
                entry(EntryKind::Allow, ME, false),
                entry(EntryKind::Allow, SYSTEM_SID, false),
                entry(EntryKind::Allow, ADMINISTRATORS_SID, false),
            ]),
        };
        let with = |change: &dyn Fn(&mut FileAcl)| {
            let mut acl = owner_only.clone();
            change(&mut acl);
            verdict(&acl, ME)
        };
        let push =
            |e: Entry| move |acl: &mut FileAcl| acl.entries.as_mut().unwrap().push(e.clone());

        assert!(verdict(&owner_only, ME));
        assert!(
            with(&push(entry(EntryKind::Deny, "S-1-1-0", false))),
            "a deny entry only narrows"
        );
        assert!(
            with(&|acl| acl.entries = Some(Vec::new())),
            "an empty DACL grants no one"
        );

        type Change<'a> = &'a dyn Fn(&mut FileAcl);
        let refused: [(&str, Change); 6] = [
            ("foreign owner", &|acl| {
                acl.owner = "S-1-5-21-9-9-9-1002".to_string()
            }),
            ("null DACL", &|acl| acl.entries = None),
            ("not protected", &|acl| acl.protected = false),
            (
                "inherited entry on a protected DACL",
                &push(entry(EntryKind::Allow, ME, true)),
            ),
            (
                "extra allow entry",
                &push(entry(EntryKind::Allow, "S-1-5-32-545", false)),
            ),
            (
                "other entry type",
                &push(entry(EntryKind::Other, "", false)),
            ),
        ];
        for (what, change) in refused {
            assert!(!with(change), "{what} must be refused");
        }
    }
}
