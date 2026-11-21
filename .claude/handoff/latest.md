# Handoff: multi-daemon (S37) — U2 pieces B (other accounts, FAT refusal) and C1 (peer identity + trusted list storage) committed; next = C2 (PeerWire + peer link over iroh)

## Know first
- Mode: autonomous. The user is a game programmer; big picture first, short, plain words.
- **Never use coined labels in user-facing text** — no "piece C1", "trusted list", "U2", "T49", work numbers. Use real file / command names or describe what happens. The user was angry about this repeatedly (2026-11-21, recorded in `~/.claude/references/global-feedback.md`). Before asking a decision, check every noun is a word the user used, a real file/command name, or a general term.
- Do not stop for minor races / display-only issues; decide and report (user 2026-11-21 「멈출정도는 아니지 않나?」). Small scope / order choices are Claude's.
- Agent cap: at most 5 subagents running at the same time (global-rules, changed 2026-11-21).
- Untracked `.claude/explain/` is old research HTML — leave it. `.claude/review-tmp/` (git-excluded) holds this session's briefs, diffs and TRD extracts (`trd-U2C-extract.md` is reusable for C2/C3).
- Company PC: Engram is a personal project; the user asked about using the peer link between the company PC (Cato Client) and home — Claude said it cannot judge company policy; check with security. Do not decide it.

## One-line state · next first action
S37 U2: platform OS functions, other-accounts measures (`632e1678`), FAT/exFAT refusal (same commit), identity + trusted list + local identity lock (`6cfc5618`) are done. U2 still needs **C2 = PeerWire + peer link over iroh** (§3-2, §3-3, §4-1, §4-2: accept gate, dial, tie-break, re-arm, duplicate detection, state, lifecycle, endpoint only while ≥1 trusted PC) and **C3 = approvals / ask-the-user seam + `peer.*` commands + typed caller** (§4-8, T18 · T19 · T29 · T32 · T38 · T41–T43), then U3a/U3b (remote mail), U4 (app UI), U5 (two-PC check).
**Next first action:** run `/engram-implement` for C2 with `.claude/review-tmp/trd-U2C-extract.md` (or regenerate from the TRD). Carry into the C2 brief: call `PeerStore` operations off the async workers; wire `identity::sweep_temps` at daemon start (ADR-0291 R12); run `NetworkSettings::check_for_endpoint` at endpoint build — a refused stored helper URL is an endpoint error, never a fallback to n0 defaults; a `set_network` that changes only the port is refused while a stored helper URL fails the adapter check.

## Repo state (measured 2026-11-21 21:15)
- Branch `v0.5.0/feat/multi-daemon` @ `6cfc5618`, in sync with origin (0/0). Against origin/master: 78 ahead, **4 behind** (master moved; merge origin/master before integration or when a binding seems missing).
- Untracked: `.claude/explain/` only. This handoff commit comes after these numbers.

## Verification state
- B (`632e1678`): review B1 deep 3 rounds PASS, B2 full 4 rounds PASS; QA full regression + shell real-daemon lane (incl. v0.3.2 cross-version stop) + claude/codex lanes + mutation 2/2 + GUI (connect, token file DACL, codex `engram` CLI and claude MCP pass the account check, +~1 ms per control request, graceful stop) — PARTIAL items (tray click, volume band in GUI) moved to `docs/unverified-tests.md`.
- C1 (`6cfc5618`): review deep 4 rounds + corrupt-list delta PASS; QA full regression (backend 5731, gates 32/32, frontend 2948), claude lane, mutation 2/2; real measurement n/a (module unwired).
- Re-run: `cargo test -p engram-dashboard-daemon -- --test-threads=4`, `cargo test -p engram-dashboard-platform -- --test-threads=4`, `node scripts/gen-third-party-licenses.mjs "$TEMP/tpl.txt"` (through `scripts/run-detached.ps1` per the qa binding).
- Not verified (all listed in `docs/unverified-tests.md`): tray Stop click; real exFAT drive; mixed elevation; a second Windows account; the identity lock across sessions / accounts / elevation; two PCs on one router (mDNS); two networks (user will use phone tethering, or a non-corporate VPN).

## Do-not / lessons
- License gate: any change that makes a shipped crate pull new deps must run `node scripts/gen-third-party-licenses.mjs` early; iroh needed CDLA-Permissive-2.0 (user-approved) and upstream MIT texts for 8 crates.
- Do not put Korean in code/config comments (the Hangul gate fails) — quote user approvals in English.
- Before QA, any new per-test `#[cfg(windows)]` outside `platform` must be added to `OS_CFG_FILES` in `.github/workflows/ci.yml`.
- The auto-mode classifier once blocked a SendMessage to a coder right after the user's angry interrupt; resending after the user said 「진행 해」 worked.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -6 && git -C I:/Engram/apps/engram-dashboard status -sb | head -3`

## Decisions (this session)
User-decided:
- FAT/exFAT policy · 상황: token / control files cannot be owner-only there, FAT rare, opt-in design was reviewed · 선택지 opt-in setting / refuse with no opt-in ★ · 고른 것 「ㅇㅇ 일단 거부로 하고 나중에 필요하면 넣자. 구지 필요없을것같아 요즘 fat 쓰는데가 어딨다고」 · 같음 (Claude had recommended refusal after the precedent check) · 근거: FAT rare → ADR-0399, opt-in parked T-125.
- Setting location (while the opt-in existed) · 선택지 data folder ★ / AppData · 고른 것 「ㅇㅇ A로 가. 어차피 파일 접근할 수 있는권한 있으면 뭔들 못할까」 · 같음 (moot after the opt-in was dropped).
- Settings screen · (묻지 않음) · 「설정화면은 나중에 하고 일단 파일에서 수정」.
- Prove the listener is the Engram daemon (PID match) · 선택지 add ★ / document · 고른 것 「리서치 간단하게 한다음에 다른데도 하면 하고 아니면 말고」 → precedent check found no tool doing it → not added, accepted risk recorded (TRD remaining risks, daemon-link.md) · 갈림 (outcome differs from Claude's first recommendation, by the research).
- PARTIAL QA items · 고른 것 「함수로만 확인하면 일단은 되지. Fat 관련해서는 그냥 놔둬」 → recorded in a new `docs/unverified-tests.md` (name chosen with the user: 「ㅇㅇ 그렇게해」).
- CDLA-Permissive-2.0 · 상황: webpki-roots, not optional in iroh 1.x · 선택지 accept / fork / wait · 고른 것 「이게 뭔지 보고 허용목로에 추가해」 · — .
- Corrupt `peers.<account>.<machine>.json` · 선택지 reset to defaults / feature off until accepted ★ · user did not pick explicitly; Claude took ★ and the user said 「진행 해」 · 같음 → TRD §8 P-J.
- Two-PC test approach · 「테더링으로 할게 나중에 내가 한다고 할때 하면되니깐 나중확인으로 남겨두면 될듯」.
- TRD first-explanation format · 「S37-multi-daemon/trd.html 적당히 괜찮아 보이는데 … 밑에는 … 펼쳐볼 수 있도록 핵심 내용들 추가해도됨. trd 최초 기본? 으로 일단 피드백 올려」 → recorded in engram-explain feedback.
Claude-decided: shell pre-check + typed band cause for FAT; start answers ordered by attempt (latest-started wins); account check: access denied on opening the process = other account; vanished listener retryable (3 in a row → refusal); no kill after an account-refused stop; `scripts/engram.mjs` PowerShell probe (SID + start time vs socket creation, whole seconds); identity lock = private namespace (ADR-0401); create-without-replace via `MoveFileExW`; daemon enables `iroh` early; gate 4d stays `ws`; default ports 47120/47121; no short fingerprint; adapter URL check only on writes; PC names refuse Cf ∪ Default_Ignorable; exceeded the 3-round fix cap once (B2 round 4, a one-line move — reported, feedback recorded).
Open: none blocking. Later: T-125 (FAT opt-in) on trigger.

## References
- `docs/process/S37-multi-daemon/trd.md` (§4-9 T47–T49, §7 U2 status line with C1 differences, §8 P-I · P-J, remaining risks), `prd.md` (R15 · R21 · AC17 · AC18).
- ADRs 0395, 0399, 0401 (and 0389 amended by 0401).
- Code: daemon `src/account_gate.rs`, `src/link/peer/{id,identity,mac,os,store,trusted}.rs`; shell `src-tauri/src/daemon_client/link/{account,route}.rs`, `src-tauri/src/discovery/mod.rs` (`pre_spawn_checks`); platform `src/{account,account_lock,owner_only,fs}.rs`; `scripts/engram-account.mjs`.
- Reference docs: `docs/reference/structure/{daemon-link,peer-link,transport}.md`; `docs/unverified-tests.md`.
- Feedback recorded this session: engram-review (same-class rule stopped autonomous work), engram-qa (fingerprint/untracked, artefact count, env to detached runs), engram-implement (fix-round cap), engram-explain (TRD first-explanation format), global (coined names).
