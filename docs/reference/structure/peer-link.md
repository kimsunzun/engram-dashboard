# Peer link (daemon ↔ daemon)

> Live reference doc — edit in place when the code moves. Anchors are file and symbol names only.
> Decisions and their reasons: ADR-0389 (identity per OS account and machine, sealing, list MAC, duplicate detection, local identity lock; its form is ADR-0401) · ADR-0401 (the local identity lock is a private namespace bounded by the account SID) · ADR-0291 (data file rules) · ADR-0395 (the OS account is the local trust boundary) · ADR-0266 (OS code lives in platform) · ADR-0388 (the iroh link adapter).

**Whole picture** = `docs/reference/architecture.md` · **Transport crate** = `docs/reference/structure/transport.md` · **Shell link** = `docs/reference/structure/daemon-link.md`

## What it is

`crates/engram-dashboard-daemon/src/link/peer/` is the daemon's link to the daemons of the same user on other PCs, beside the shell link in `link/`. What exists so far is this PC's identity and its trusted list: a per-account-and-machine identity key sealed for the current OS account, the PC ID derived from it, the trusted list with its network settings and MAC, and one state object (`PeerStore`) the rest of the peer module reads and changes. Nothing is wired into the daemon's start-up yet and nothing touches the network; the wire, links, approvals and `peer.*` commands are to build on `PeerStore`.

## Modules

- `link/peer/mod.rs` — module root; link adapter types never enter here.
- `link/peer/id.rs` — `PeerId`: the 32-byte public key, its canonical text (64 lowercase hex), the grouped full-ID display (`grouped`) and parsing (`parse` also takes the grouped and uppercase forms; stored IDs must be canonical).
- `link/peer/identity.rs` — `Secret` (zeroed on drop, never printed; `id` derives the PC ID through transport's `iroh::public_key`), `PeerFiles` (the per-account-and-machine names in the state folder, and this account's key files under other machine ids), `load_key` → `KeyLoad` (loaded / missing / unreadable / transient / unavailable; the unsealed plaintext is wiped), `create_key` (never replaces), `replace_key` (backup copy, then atomic swap), `sweep_temps` (leftover temp files of the key and list — for the daemon to call at start; not wired yet).
- `link/peer/mac.rs` — `sign` / `verify`: HMAC-SHA256 over the stored body bytes with an HKDF-SHA256 key from the identity secret (`ring`); pinned by a known-answer test.
- `link/peer/trusted.rs` — `TrustedList` / `TrustedPeer` / `Permission` / `NetworkSettings` / `Helper`, the entry rules (`check_entries`, `check_name`), the default listen ports (`RELEASE_DEFAULT_PORT`, `DEV_DEFAULT_PORT`), the file `SPEC` for base `file`, `read_list` → `ListFile`, `write_list`.
- `link/peer/os.rs` — `PeerOs`: the OS seam (seal, unseal, identity lock, wall clock); `OsPeer` = platform `secret` and `account_lock` (lock name `engram-peer-identity.<ID>`).
- `link/peer/store.rs` — `PeerStore` (the state object), `PeerStatus`, `PeerError`, `PeerEdit`, `Removal`, `Regenerated`; `store/tests.rs` runs it over real temp folders with a fake `PeerOs`.
- Platform (`crates/engram-dashboard-platform/src/`): `secret.rs` (`seal` / `unseal`, DPAPI CurrentUser), `account.rs` (`current_account_id`, `machine_id`), `account_lock.rs` (`acquire` — a private object namespace bounded by the account's SID; nothing on disk), `fs.rs` (`rename_no_replace`). Base `file.rs`: `write_atomic_new` / `copy_atomic_new` (temp → sync → rename that never replaces).

## Files (daemon state folder, `DataLayout::daemon_state_dir`)

- `peer-key.<account>.<machine>` — the sealed secret blob, nothing else. `<account>` = OS account id (Windows SID), `<machine>` = machine id (Windows `MachineGuid`); both are checked to be letters, digits and `-` before they become names.
- `peer-key.<account>.<machine>.bak-<UTC time>[-n]` — copies of the keys a regenerate replaced; never overwritten, never read back.
- All writes are base `file` atomic writes (temp names `<name>.tmp<pid>.<n>`); the key and the backups are created with a rename that never replaces.
- `peers.<account>.<machine>.json` — `{version, mac, body}`: `body` is the list serialized as a JSON string and `mac` covers its exact bytes. Under base `file` rules (`SPEC`): a newer version or a failed read refuses writes; a corrupt file (including a body that does not parse or breaks an entry rule) puts an empty list with default settings in memory, but once a key is loaded the feature stays off (`ListCorrupt`) until `accept_list`, whose write copies the corrupt file to `.corrupt` first; a missing file is just an empty list; the overwrite decision is made again right before every write.
- Files of any other name are never touched. Two accounts sharing the folder never see each other's files.

## Flow and promises

- Load (`PeerStore::load` / `open`): name the files → read the list file → read and unseal the key → decide the key state (no key file: `KeyUnreadable(MissingUnderList)` if the list names PCs, else `AdoptOrFresh` if this account has keys under other machine ids, else `NoIdentityYet`) → check the list MAC under the key (no key: not checked) → take the identity lock when a key is loaded. Loading creates nothing.
- `PeerStatus` is the one answer to "may the peer feature run" — only `Ready` does. Priority: unavailable → key transient / unreadable / adopt-or-fresh → no key yet (list refused, else no identity) → identity in use → list refused → list changed outside / list corrupt → ready.
- A key is written only by `ensure_identity` (from `NoIdentityYet`; `add` and `set_network` call it first, the other list writes need `Ready`), `regenerate`, `start_fresh` and `adopt`. A list file that must not be written over (newer version, failed read) wins over the adopt-or-fresh question, so none of them runs then. Regenerate keeps a backup and does not re-sign the list, so the list reads as changed outside until `accept_list`; a restart in between gives the same state. Regenerate is refused while the key is transient or the adopt-or-fresh question is open.
- `adopt` copies the other machine's key (create, never replace) and list, then reloads — a key this account cannot open lands in `KeyUnreadable`. A list file already here is handled by base `file`'s write policy first: refused (newer, failed read) → nothing is written; otherwise it is set aside as `.corrupt` before the other list is copied over it. The other machine's files stay.
- List writes apply to a copy, sign it, write it, and keep it only on success; `remove` revokes in memory first and reports a refused save in `Removal::saved`. A file refusal answers `PeerError::Conflict`.
- Identity lock: platform `account_lock` — a private object namespace named `engram-peer-identity.<ID>` whose boundary holds only this account's SID; the process that creates it holds the lock, and it goes when that process closes it or ends. Nothing is written to disk. Other accounts cannot create it (Win32: the creator must be within the boundary — measured: a boundary SID the caller lacks is refused), and a boundary without a session number spans sessions. Held for as long as a key is loaded and moved on a regenerate. Held elsewhere → `IdentityInUse`, and `retry` tries again (the other copy may have ended); an OS failure → the copy runs without it (`lock_note`). ADR-0401 replaced ADR-0389 decision 9's `Global\` named mutex, which another account could pre-create.
- `PeerStore` does no gating (human vs agent, approvals), no retry timing (`retry` reloads from a transient key or a failed list read and retakes a lock held elsewhere; the caller decides when) and no logging.
- Not defended (ADR-0395): a process of the same account can unseal the key and recompute the MAC; an older correctly signed list or a key backup put back by anyone who can write the folder is accepted.

## Where to look

- State and operations: `store.rs` (`PeerStore::load`, `status`, `ensure_identity`, `add`, `edit`, `remove`, `set_network`, `regenerate`, `accept_list`, `adopt`, `start_fresh`, `retry`; internals `reload`, `after_key_change`, `save`).
- Key file rules: `identity.rs` (`load_key`, `create_key`, `replace_key`, `PeerFiles::other_machines`).
- List file rules: `trusted.rs` (`SPEC`, `read_list`, `write_list`, `check_entries`).
- The lock's OS side: platform `account_lock.rs` (`acquire`, `AccountLock`); the no-replace rename: platform `fs.rs` (`rename_no_replace`) and base `file.rs` (`write_atomic_new`, `copy_atomic_new`).
