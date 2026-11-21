# ADR-0401: The local peer identity lock is a private object namespace bounded by the account SID

- 상태: 확정 (2026-11-21, basis: S37 U2 piece C1 code review (lead + other-account lens + codex), 3 rounds — the `Global\` mutex design could be squatted by another account and a lock file left traces in the profile)
- 관련: ADR-0389 (decision 9: a `Global\` named mutex with an owner check) · ADR-0134 (decision 1: deleting the distribution folder leaves no trace) · ADR-0395 (same-account processes are trusted) · ADR-0266 (OS code in platform) · `docs/process/S37-multi-daemon/trd.md` (§3-1 · §4-2 · T40) · Amends ADR-0389 (decision 9 local identity lock)

## 맥락
ADR-0389 decision 9 refuses a second Engram copy with the same peer identity on the same machine and account through a machine-wide named lock (`Global\`, named by the account SID and the identity, owner checked), and lets a copy start without the lock when the name exists under another account. The C1 review found that the name is built from public facts (the SID is in `daemon.json`; the ID is shared with other PCs), so another account could pre-create it and silently switch the local refusal off. The first replacement — an exclusively opened lock file under the account's local app-data folder — closed the squat but left files in the user profile, against ADR-0134 decision 1.

## 결정
1. **The lock is a private object namespace whose boundary holds only the current account's SID**, named `engram-peer-identity.<ID>` (platform `account_lock::acquire`). Creating the namespace is holding the lock; "already exists" means another copy of this account holds it (identity in use); dropping the guard destroys it; process death frees it.
2. **"Runs without the lock" applies only to an OS failure** (account SID unreadable, boundary or namespace calls failing for another reason). A name held by another account no longer exists as a case: another account cannot create a namespace with this account's SID in its boundary.
3. **Nothing is written to disk** for the lock; the namespace's DACL is the creator token's default DACL and its handle is not inheritable, so agent child processes cannot keep it alive.
4. A copy refused as "identity in use" retries the lock on `retry()`, so it recovers when the other copy exits or a transient holder lets go.

## 거부한 대안
- **`Global\` named mutex with an owner check (ADR-0389 d9 as written)** — another account can pre-create the name from public facts and make the copy run without the lock (C1 risk lens, code-traced).
- **Exclusively opened, owner-only lock file under `%LOCALAPPDATA%`** — closes the squat, but leaves `Engram\peer-identity\<ID>.lock` files and folders after the distribution folder is deleted, against ADR-0134 decision 1 (C1 lead review).
- **Lock file inside the data folder** — copies of one data folder (the case the lock exists for) would each have their own file and never collide.
- **`FILE_FLAG_DELETE_ON_CLOSE` lock file** — still leaves the folders after a crash; the namespace leaves nothing.

## 근거
- Win32 docs (Object Namespaces, `CreatePrivateNamespace`): the caller must be within the boundary for the create to succeed; a namespace is identified by its name and its boundary.
- Measured in platform tests: a boundary carrying a SID the caller lacks is refused (`PermissionDenied`); two holders collide; closing without destroy (what process death does) frees the name.
- Not measured here (recorded in `docs/unverified-tests.md`): two sessions of one account, another account's attempt, elevated vs non-elevated copies.

## 영향 / 불변식
- The lock must stay outside the data folder (it must collide across copied data folders) and must leave no trace on disk (ADR-0134 d1).
- Do not reintroduce a name another account can create; do not let "held by someone else" lead to running without the lock.
- ADR-0389's other decisions (per-account sealing, the MAC, duplicate detection on the network) stand; the network duplicate detection still covers copies on other machines.
