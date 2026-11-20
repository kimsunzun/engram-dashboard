# ADR-0399: Data folders on drives without file ACLs (FAT or exFAT) are refused, with no opt-in

- 상태: 확정 (2026-11-21, basis: user decision P-I 2026-11-21 (「그냥 usb에서는 동작 안되게 하면 되나? 다른 오픈소스는 어떤지 확인해봐」 → after the precedent check 「ㅇㅇ 일단 거부로 하고 나중에 필요하면 넣자. 구지 필요없을것같아 요즘 fat 쓰는데가 어딨다고」) + S37 U2 T49 review PASS, deep, 4 rounds, 2026-11-21)
- 관련: ADR-0395 (owner-only token and agent control files; this ADR keeps its invariant by refusing where it cannot hold) · ADR-0134 (portable data folder under the exe folder; a portable copy now needs an NTFS drive) · ADR-0372 · ADR-0373 (the refusal text is translated where shown) · ADR-0266 (the volume query lives in platform) · ADR-0389 (the list MAC) · `docs/process/S37-multi-daemon/trd.md` (§3-1 · §4-9 T49 · T46 · T49) · PRD R15 · R21 · AC18 · `docs/tracking.md` T-125 · Amends ADR-0395 (decision 1 MAC wording, replay not caught)

## 맥락
ADR-0395 makes the shell token file and the agent control files owner-only, created with the DACL passed at create (`platform::owner_only::create_owner_only`). On a volume that stores no file ACLs (FAT / exFAT — e.g. a portable copy, ADR-0134, on an exFAT USB stick) that function returns `Unsupported`, so those files cannot be protected. The daemon is launched through WMI (`platform::spawn::spawn_outside_job`), passes back nothing but `daemon.json`, and its start-up failures reach the shell only as a discovery timeout — so a refusal inside the daemon alone would show the user "timed out", not the reason. The S37 review of the first design (an opt-in) also found that the trusted-list MAC wording in ADR-0395 decision 1 overstated what it catches.

## 결정
1. **A data folder on a volume without file ACLs is refused; there is no opt-in.** An NTFS drive (an NTFS-formatted stick included) works as usual. An opt-in is deferred (`docs/tracking.md` T-125) until someone needs it.
2. **The shell checks before spawning.** A platform volume query answers `Ok(true)` / `Ok(false)` only when the volume flags were read, `Err` otherwise (Windows only; off Windows with T-118 / T-119). In `ensure_daemon`'s pre-spawn step, after the writable check and on the data root, `Ok(false)` and `Err` both refuse without spawning. Skipped under `ENGRAM_DATA_DIR`, like the writable check.
3. **The refusal reaches the user as a typed cause on the shell `Band`, translated where shown.** Causes: "unprotected volume" and "could not read the drive's properties"; both name the fix — exit Engram, move the Engram folder (for a portable copy, the whole folder with the exe) to an NTFS drive, start it again. Every ensure path carries it (`daemon_start`'s typed code, `EndpointSource::ensure`'s typed error into the link route, the `DaemonClient` setter for the tray and `daemon_start`). It takes precedence over the untyped connection error, survives the link's background retries, and is cleared only by an ensure attempt or a successful connect; a daemon's `Rejected` / version answer replaces it.
4. **The daemon still refuses on its own** when `create_owner_only` fails for the token file (ADR-0395); for a control file only that agent's provision fails.
5. **Trusted-list MAC wording (ADR-0395 decision 1).** The MAC catches naive edits and other accounts' edits, **not replay**: anyone who can write the folder can put back an older correctly signed list or a key backup. ADR-0395's rejected-alternative rationale "only a same-account process can produce a correctly signed list" is read with that exception. Folder-wide ACLs stay outside S37.

## 거부한 대안
- **Opt-in setting in the data folder's `settings.json`** (Jupyter `JUPYTER_ALLOW_INSECURE_WRITES`, git `safe.directory` style; refuse by default, the shell refreshes the key on each ensure and passes a fixed WMI flag, the daemon writes the files with the inherited ACL) — designed and reviewed (PASS), then dropped by the user: FAT / exFAT is rare today and the opt-in added a settings refresh, a WMI argument channel and a second file mode. 약함 — 사용자 직접 기각. Parked in T-125.
- **Opt-in stored per account (AppData)** — only meaningful with an opt-in; the user had chosen the data-folder location ("anyone who can write the drive can replace the exe anyway"), and the opt-in itself was then dropped.
- **Refuse inside the daemon only** — the WMI-launched daemon has no channel back but `daemon.json`; the shell would show a bare discovery timeout (code: `spawn_outside_job` passes only the quoted exe, `ensure_with` reports `DiscoveryError::Timeout`).
- **Treat unreadable volume flags as "has ACLs"** — fails open: an ACL-less volume whose flags cannot be read would get unprotected files.
- **Pass the English `DiscoveryError` string to the band** — the band would keep a stored string across a language switch, against ADR-0372 / ADR-0373 (translate where shown); the shell link's background retries would also overwrite it with a generic "not connected" (`route.rs` `KeyRoute::event`).

## 근거
- Precedent check 2026-11-21: OpenSSH refuses private keys on FAT with no override; Jupyter `secure_write` and git's ownership check refuse by default and add an opt-in; VS Code portable mode and Syncthing are used from USB without a documented FAT check (not confirmed either way).
- On FAT / exFAT anyone who can write the drive (other accounts, whoever holds the stick) can replace the Engram executables, so an opt-in could only guard against accidents — the user judged that not worth its cost now.
- S37 U2 T49 review (lead + other-account lens + codex), 4 rounds: found the WMI timeout-only channel, the band's untyped-error and background-retry overwrite, and the MAC replay overstatement.

## 영향 / 불변식
- Engram never writes the shell token or an agent control file with an inherited ACL — on a volume where owner-only creation is impossible it does not run.
- The shell pre-check and the daemon's own refusal are both kept; removing the pre-check turns the reason into a bare timeout.
- The band cause is typed and translated where shown; no code stores its translated text.
- Adding an opt-in later (T-125) changes decision 1 of this ADR and the ADR-0395 invariant on such volumes — it needs a new ADR.
