# ADR-0389: Daemon identity is an iroh node key per OS account and machine, sealed by the OS; the trusted list is MAC'd with it and duplicate identities are detected

- 상태: 확정 (2026-11-20, basis: user decisions P-A 「ㅇㅇ A+B로 가」 and P-D 「ㅇㅇ 계정단위로해줘」 2026-11-20 + X5 2026-11-19 + S37 PRD+TRD review PASS, 4 rounds, 2026-11-20)
- 관련: Amends ADR-0310 (decision 4 D3: persistent DaemonId as a claimed UUID identity on Hello) · ADR-0310 (decision 6 D5 ③: the daemon↔daemon handover — now built by S37 · D2: own Wire · Registry · Listener) · ADR-0388 (the iroh link) · ADR-0394 (human approval, the ask-the-user seam) · ADR-0395 (the OS account as the trust boundary) · ADR-0266 (OS code in platform) · ADR-0291 (base `file` rules) · ADR-0135 (one daemon per data folder) · `docs/process/S37-multi-daemon/trd.md` (§3-1 · §4-2 · §8 X5 · §9 P1 · T4 · T20 · T31 · T34 · T37 · T39 · T40)

## 맥락
ADR-0310 decision 4 (D3) planned a persistent `DaemonId` (a UUID v4 in the daemon state folder, announced on `Hello`) as a claimed, unverified id, to be bound to a key later by pairing; decision 6 (D5 ③) handed the daemon↔daemon relationship to the multi-daemon stage. S37 is that stage: the iroh link (ADR-0388) authenticates each side by its node key, so the key itself can be the identity. `DaemonId` / `Hello.daemon_id` were never built (TRD §9 P1), so nothing migrates.

Review found that a portable data folder copied to another PC or account would clone the identity (P-A), and that two accounts sharing one data folder would displace each other's key and list (P-D). The user chose sealing per account plus duplicate detection (P-A 「ㅇㅇ A+B로 가」) and per-account files (P-D 「ㅇㅇ 계정단위로해줘」 · 「계정마다 key 와 json을 만들자는거지? ㅇㅋㅇㅋ」).

## 결정
1. **Identity = the iroh secret key (32 bytes); the PC's ID = its public half** as text in the daemon's own format. The daemon owns the ID text format (from the raw 32 bytes) and the grouped full-ID display used in approval questions; transport takes and returns raw key bytes. This replaces ADR-0310 decision 4; the rest of ADR-0310 stands.
2. **Per OS account and machine.** The key and the trusted list live in `DataLayout::daemon_state_dir` as `peer-key.<account>.<machine>` and `peers.<account>.<machine>.json` (`<account>` = Windows SID, uid elsewhere; `<machine>` = Windows `MachineGuid`, Linux `/etc/machine-id`). Files of other names are left untouched. This account's files under another machine id are detected and the user is asked through the ask-the-user seam whether to adopt them or start fresh — never automatically.
3. **Sealed for the current OS account through one platform function**: Windows DPAPI `CryptProtectData` CurrentUser called directly (blob in the file), macOS Keychain, Linux the raw secret in an owner-only 0600 file. Created once, on first need (first `peer.*` call or first add), written temp → sync → rename without replace. The daemon never replaces the key on its own.
4. **Load outcomes**: no file for this account and machine (and none of this account under another machine id) → fresh identity on first need, no prompt; a file DPAPI refuses as not-yours / bad data, a corrupt file, or a missing key while the list names PCs → "key unreadable", the peer feature does not start, `peer.list` and the app say so (X5 flow); any other unseal error or IO failure → transient, retried with backoff, regenerate not offered, recovers with the old ID.
5. **A new key only on a human's OK** (X5): the old file is kept by rename to `….bak-<UTC time>` (a repeated regenerate never overwrites an earlier backup); the trusted list is not re-signed; the result tells the user that every other PC must remove the old ID and pair again.
6. **The trusted list carries a MAC**: file `{version, mac, body}`, HMAC-SHA256 over the exact bytes of the `body` string, key = HKDF-SHA256 of the identity secret with a fixed label, constant-time compare; checked on every load, rewritten on every save, under the base `file` rules (ADR-0291). A mismatch → the peer feature stays off, "trusted list was changed outside Engram", and a human is asked through the ask-the-user seam whether to accept (re-sign) the list as it is; agents cannot resolve it. After a regenerate the old MAC fails under the new key, so the question returns on every start until a human accepts.
7. **The peer files are protected by sealing and the MAC, not by folder ACLs.** Folder-wide ACLs stay outside S37.
8. **Duplicate-identity detection from fresh evidence**: every `Hello` carries `instance` (≥ 64 random bits drawn at daemon start). (a) The same node completes handshakes with two instances and both links are proven alive by a `Probe` / `ProbeAck` (within 10 s) after the second handshake, or (b) a replaced instance returns in a fresh handshake within 5 min → the newer link is refused (terminal) and both sides warn with the structured `Duplicate` message. (c) The same node's link replaced 3 times within 10 min → "suspected duplicate", no refusal. The warning first suggests looking for a second running copy, then offers regenerate.
9. **A second copy on the same machine and account is refused locally** by a machine-wide named lock (`Global\` namespace, named by the account SID and the identity, owner checked). If the name exists but belongs to another account or cannot be opened, the copy starts without the lock (network detection still applies).
10. **Our own ID offered to `peer.add` is a plain refusal** ("that is this PC's own ID"), not a duplicate warning. The relay's `SameEndpointIdConnected` is not used.

## 거부한 대안
- **ADR-0310 D3's claimed UUID `DaemonId`** — a claimed, unverified id could carry no persistent trust until bound to a key; the node key is key-bound identity by construction, and `DaemonId` was never built (TRD §9 P1), so replacing it costs nothing.
- **An owner-only DACL check on the key file** (TRD draft 3) — replaced by sealing: a copy on another PC or account cannot unseal, and the check works the same without folder ACLs (exFAT) and on every OS; user rejected directly (P-A 「ㅇㅇ A+B로 가」).
- **Windows Credential Manager / the `keyring` crate** — the `keyring` Windows store roams by default; Linux Secret Service is often absent on servers and keyutils does not persist.
- **One identity per machine** — per-machine is the norm only for privileged services (Tailscale, Docker Desktop); unprivileged tools are per-user (Syncthing, Reason Network, hcom, KDE Connect, Dropbox / OneDrive); user rejected directly (P-D).
- **File names by account SID alone** — a domain / Entra SID is the same on every machine, so a moved folder would raise a false "unreadable" and a roaming-DPAPI copy would become a duplicate.
- **Regenerate on any load failure** (Syncthing `LoadOrGenerateCertificate`) — user rejected directly (X5: 「이번만 파일을 못읽는다거나 그럴 수 있는데 날려버리면 … UI 쪽에 메세지 안내하고 유저가 ok 하면 백업만들고 새로 만들면」); silent recreation under existing pairings would change the ID.
- **Re-signing the trusted list at regenerate** — it would bless a list nobody verified.
- **Adopting this account's files under another machine id automatically** — a roaming-DPAPI copy looks the same.
- **Any second instance seen counts as a duplicate** — a late frame from an abruptly restarted peer's old link would raise a false alarm; only links proven alive after the second handshake count.
- **The duplicate warning as close-reason text** — close-reason text is best-effort and shared with "not paired".
- **The relay's `SameEndpointIdConnected`** — `iroh-1.4.1` only logs it (`socket/transports/relay/actor.rs`, `warn!`), and the gate never sees a running clone.
- **Refusing start when the identity lock name is held by another account** — it would hand another account a way to block start-up.

## 근거
- User decisions P-A and P-D (2026-11-20) and X5 (2026-11-19), quoted above.
- Precedents: Chromium's DPAPI-wrapped key; Tailscale seals to the TPM "to make node cloning more difficult" and shows a duplicate-node-key badge; Reason Network keeps `machine.key` in the user's home and stops on a corrupt key (`load_key`).
- Another account cannot open this account's key, so it cannot forge the MAC.
- S37 PRD+TRD review, PASS after 4 rounds (2026-11-20).

## 영향 / 불변식
- Seal / unseal, the account id, the machine id and the named lock live in platform (ADR-0266); S37 builds the Windows side, Linux / macOS are defined and built with T-118 / T-119.
- The daemon never writes a new key without a human's OK; a missing key under a non-empty list is never silently recreated.
- The MAC scope is the OS-account boundary (ADR-0395): it catches naive edits and other accounts' edits only; a same-account process can unseal the key and recompute it, and rollback to an older signed list is not defended.
- Accepting a changed list and approving a regenerate are human-only (ADR-0394).
- Detection reach: both holders of one key must link to a common PC; a cloned VM or disk keeps the machine id, so only network detection catches it.
- Sealing loss: a Windows forced password reset makes the key unreadable and every PC must re-pair; a reinstall with a new SID starts fresh silently.
- Replacing the link adapter must not change stored IDs (the daemon owns the text format).
- ADR-0310 decision 6's handover is carried out: the daemon↔daemon relationship now has its own `PeerWire` · `Registry` · `Listener` (ADR-0310 D2).
