# Handoff: multi-daemon (S37) — PRD/TRD approved, U1 (iroh adapter) and U2 piece A (platform OS functions) committed; next = U2 piece B (other-account measures), one user question first

## Know first
- Mode: autonomous (user: 「자율모드 시동이다」 · 「쭉 진행해」). The user is a game programmer learning networking: big picture first, short, plain words; **no work / tracking numbers in user-facing text** (user 2026-11-20: 「번호 필요없어」 — call things by name, e.g. "멀티데몬", "iroh 통로"); explain with timelines and data movement when asked; do not ask about small scope / order choices (「개발순서 관련해서는 큰 볼륨아니면 나한테 물어보지말고 알아서」) — decide and report.
- Explain format (user criterion, to be turned into an engram-explain format via skill-factory later): first explanation = overall structure picture + block-level code structure + main runtime flows, picture-first, minimal text, details collapsed at the bottom; no decision lists. `docs/process/S37-multi-daemon/trd.html` is the first try of that format.
- Agent cap: at most 5 new subagents per user instruction (codex and SendMessage-continued workers not counted).
- `566fef91` (checkHangul test 30 s timeouts) is a small-fix commit marked "review pending" — batch it into a later review.
- Untracked `.claude/explain/` is old research HTML, not part of this work — leave it.

## One-line state · next first action
PRD approved; TRD draft 8 reviewed (4 rounds, all PASS); 8 ADRs (0388–0395); U1 iroh adapter done (`83c4098e`); U2 piece A platform OS functions done (`72a10e88`). U2 has two more pieces.
**Next first action:** ask the user the exFAT question below (one question, plain words), then run `/engram-implement` for U2 piece B — other-account measures (TRD §4-9 / T47 / T48): token split (`daemon.json` = shared lock + owner SID, `daemon-token.<account>` owner-only via `platform::owner_only::create_owner_only`), `mcp-config/*.json` owner-only, daemon accept-time account check in `link/server.rs` and the control listener (`platform::account::tcp_client_account`), shell-side check at the four `daemon.json` read sites (`src-tauri/src/discovery/mod.rs` `FileReader::read`, `src-tauri/src/daemon_client/link/endpoint.rs` `judge`, `src-tauri/src/daemon_client/link/stop.rs`, `scripts/engram.mjs`) using `tcp_server_account` after connecting, `DaemonInfo` owner-SID field with `#[serde(default)]` + `PROTOCOL_VERSION` bump, `docs/reference/structure/daemon-link.md`. Then piece C (link/peer/ core + `peer.*` + ask_user seam + identity named lock, which piece A did not include).

## Open — ask the user (one at a time)
- **exFAT / no-ACL volume (needed by piece B):** `create_owner_only` now returns `Unsupported` on volumes without persistent ACLs (FAT/exFAT USB). The token file then cannot be protected. Options: (a) the daemon refuses to start there with a clear message; (b) continue with a warning and rely only on the accept-time account check (works once the platform lookup is used). Claude has no recommendation recorded yet — weigh: (b) keeps portable-USB use working, (a) is strict. Also note: if TEMP is on FAT the approval dialog returns Unsupported (manifest goes through `create_owner_only`) → caller treats it as "dialog unavailable".

## Repo state (measured 2026-11-20 22:51)
- Branch `v0.5.0/feat/multi-daemon` @ `38fd7670`, in sync with origin (0/0); 70 commits ahead of origin/master, 0 behind (origin/master merged at `e21258f5` this session to get `qa-mutate.json`).
- Untracked: `.claude/explain/` only. This handoff commit comes after these numbers.

## Verification state
- U1 (`83c4098e`): deep review (lead + link-lifetime lens + codex), 2 fix rounds, PASS. QA PASS: workspace 5519, transport --all-features 478 (iroh_link 28 incl. a 30 s real idle-out), gates 32/32, frontend 2928, GUI smoke (app↔daemon link, claude reply), mutation 2/2.
- U2 piece A (`72a10e88`): deep review (lead + Win32-security lens with measured probes + codex), 2 fix rounds, PASS. QA PASS: workspace 5556, platform 122, gates 32/32, frontend 2928, codex lane (CLI 0.171.0, lanes.tsv updated), GUI smoke (daemon start_time = process start; codex reply), mutation 2/2, real dialog open/dismiss, TaskDialog load ×3.
- Re-run: `cargo test -p engram-dashboard-transport --all-features` · `cargo test -p engram-dashboard-platform -- --test-threads=4` (through `scripts/run-detached.ps1` per the qa binding).
- Not verified / not tested: a dial cancelled mid-connect (U1); delete-by-path cleanup on a no-ACL volume and in-process repeated TaskDialog load (piece A); iroh_link tests cannot be mutation-checked (wrapper passes no cargo features); TaskDialog `&` rendering (manual); elevated-process lookup, real session 0, second-account cases (need U5 / real accounts).
- Measured facts now in docs: a dead peer is detected at the QUIC connection idle timeout, 30 s (`iroh::QUIC_IDLE_TIMEOUT`), not 15 s; TCP table rows of a dead creator keep its PID and are dated at socket creation (lens probes, cases A–D).

## Do-not / lessons
- Do not assume a binding file is missing — first check whether this branch is behind master (`qa-mutate.json` existed on master; merging origin/master fixed it).
- Reviewer contexts grew large (the PRD/TRD lead reached ~534k); start fresh reviewers per topic.
- Codex `--resume-last` fails across session restarts ("No previous Codex task thread") — pass earlier findings in a brief file and run fresh.
- The TCP owner lookup must not be trusted from the PID alone (stale/reused PIDs): use the timestamp rule in `platform::account`; do not add a clock tolerance (reopens PID reuse).

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -4 && git -C I:/Engram/apps/engram-dashboard status -sb | head -3`

## Decisions (this session)
User-decided:
- PRD approval · 상황: every PRD item came from the user's decisions, review closed · 선택지 approve / — · 고른 것 「prd는 얘기 다 한거 아님? 나하고?」 · — .
- Portable copy identity (P-A) · 선택지 A+B ★ (seal per OS account + duplicate detection) / B / C · 고른 것 「ㅇㅇ A+B로 가」 · 같음 · 근거(Claude): Tailscale TPM / Chromium DPAPI precedent; detection covers roaming/Linux.
- Bare name on several PCs (P-B0) · 선택지 refuse + `name@*` form ★ / auto fan-out / refuse only · 고른 것 「ㅇㅇ 그렇게 하면 될듯?」, `name@*` timing 「알아서해」 → deferred T-117 · 같음.
- Unknown directory not counted (P-B) · 선택지 persist copies ★ / refuse / keep · 고른 것 「그냥 놔두면 되지 않아? 유저 잘못이지」 → keep, amend D2 · 갈림.
- Trust-changing commands (P-C) · 선택지 human approves ★ / approve + bypass switch / agent direct · 고른 것 「사람이 승인 하는게 맞다」; UI: app if open, else native OS dialog 「디폴트 쓰자」, 「플랫폼으로 분리되서 실행되는 공용」 · 같음 (terminal yes/no rejected after precedent check).
- Identity per OS account (P-D) · 선택지 per account ★ / per PC · 고른 것 「계정단위로해줘」 + per-account SID files 「ㅇㅋㅇㅋ」 · 같음.
- `peer.edit` (P-E) · Claude decided under the user's scope rule.
- Liveness (P-F) · 선택지 align ping/idle ★ / defaults / shorter · 고른 것 「우리 감지를 빼면 안됨? iroh 피드백 받으면」 → rely on iroh only · 갈림.
- Inbound limits (P-G) · 선택지 refuse ★ / defer → reframed → per-agent caps dropped, one per-PC backstop · 고른 것 「1개 에이전트가 60개 한꺼번에 보낼일은 없을테니 그냥 의미없는 수치」 → 「ㅇㅇ 일단 넘어가고」 · 같음 (final proposal).
- Local threat model (P-H) · 선택지 (a′) OS account = boundary, block other accounts on app↔daemon ★ / (b) full same-PC isolation · 고른 것 「같은계정은 막지 않는게 맞고」 · 「다른계정이면 막는다는거잖아」 · 같음.
- One more fix round past the cap (PRD/TRD round 4) · 고른 것 「ㅇㅇ 고치고 진행해」.
- Briefing format · (묻지 않음) · 「전체구조 + 코드 구조 대략적으로」, 「그림 위주」, 「블럭으로 표현될 수 있는 범위」, 「스킬 팩토리 쪽에 얘기하면 될듯」.
- Tracking only: Linux T-118, macOS T-119 (「다른 곳에서 펼치는게」), outbox T-120 (「꼭 적용」), loop detection T-121, attachments T-122, agent-id duplicates T-123, Linux CI compile job T-124 (「별도 주제로」).
Claude-decided: all internal TRD fixes of review rounds 2–4 (draft 3–8: late verdicts 120 s / 7 s, row tags, instance nonce + probe duplicate rule, account × machine file names, lock/token split, typed caller + `humanOnly`, receive queue bounds, in-flight cap 256, dialog Deny default / full ID / cooldown, etc.); U1 internals (`supervisor_keepalive`, acknowledged-before-close, handshake timeout, explicit 30 s idle); piece A internals (direct Task Dialog instead of a crate, Unsupported off Windows, timestamp-proven TCP owner, LOCAL_MACHINE blob refusal); merged origin/master into the branch; small-fix test timeouts `566fef91`.
Open: exFAT policy (above).

## References
- `docs/process/S37-multi-daemon/` — prd.md (approved), trd.md (draft 8; §7 U1 done, U2 pending; §8 decided-after-review list), trd.html (structure briefing), README.
- ADRs `docs/decisions/0388`–`0395` (0388 and 0394 carry dated corrections).
- Reference docs: `docs/reference/structure/{transport,messaging,daemon-link,command-bus}.md` (transport updated with the iroh module).
- Code: `crates/engram-dashboard-transport/src/iroh.rs` (`IrohEndpoint`, `IrohConfig`, `IrohAcceptPolicy`, `QUIC_IDLE_TIMEOUT`); `crates/engram-dashboard-platform/src/{secret,account,owner_only,approval}.rs` (`seal`/`unseal`, `current_account_id`, `machine_id`, `tcp_client_account`/`tcp_server_account`, `create_owner_only`/`is_owner_only`, `ask_approval`/`DialogCloser`).
- Tracking: `docs/tracking.md` T-115…T-124.
- Feedback recorded this session: `~/.claude/references/global-feedback.md` (small scope/order choices = Claude's; no numbers in user text), `engram-explain/feedback.md` (structure-first first explanation), `engram-qa/feedback.md` (mutate wrapper features; fingerprint vs untracked folders).
