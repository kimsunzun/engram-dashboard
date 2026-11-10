# Handoff: S21 stage 3 transport done (3-3 replace-only + stop-order fix) · next = user's GUI check, then master merge

## Know first
- **Mode: autonomous.** User wants plain words, no invented labels, no agenda items — 「뭐 자꾸 안건 꺼내지 말고」. Ask only what changes visible behaviour; decide internals and report.
- **User's bar for fixes (2026-11-10):** fix only real breaks seen in normal use — **data loss, leftover processes, hangs**. Connection churn (an extra redial, a band a moment late) is accepted — 「연결 안되는건 그냥 다시 연결하면 되잖아」. Do not take review 권장 items for rare orderings (logged in Engram feedback `78c7435`); user called the earlier over-fixing 「별 필요없는 부분까지 다 수정하는 느낌」 · 「transport 그냥 연결만 하는 문제였는데 뭔 복잡하게 하냐고」.
- **A release build of HEAD `05cd2b1d` is running for the user's manual check** (app PID 33644, daemon PID 35112) from `C:\Users\kimsunzun\AppData\Local\Temp\engram-check-05cd2b1d\` (data next to the exe, separate from `.engram-dev`). An older check build exists at `...\engram-check-df13d853\` — can be deleted.
- **Untracked `.engram-dev.qa-bak-engram-qa-20261109-111846/` holds a daemon token** — never stage it (`git add -A -- . ':(exclude).engram-dev.qa-bak-*'`). `recycle.sh` fails on it (`code=0x78`); user will delete it after a restart.
- Build script `scripts/build-release.ps1` needs **`pwsh`** (PowerShell 7); Windows PowerShell 5 fails on `GetFullPath` overload.
- All cargo `target` folders under `I:\Engram\apps` were cleaned 2026-11-10 (≈ 456 GiB) — first builds are cold.

## One-line state · next first action
Stage 3 transport is complete on branch `v0.4.1/refactor/transport-attach` (daemon on the transport, net crate deleted, stop-order regression fixed); CI for `05cd2b1d` was **in progress** at save time. **Next first action:** check CI (`gh run list -w ci.yml -c 05cd2b1d --json status,conclusion`), get the user's result of the manual GUI check (below), then merge to master per `docs/handbook/branch-commit.md` (push → CI green → `--no-ff`, merge without checkout via `merge-tree`/`commit-tree`/`update-ref`, list chunks in the merge body). The user already said 「이거 문제없으면 마스터푸쉬 하면 될듯」.
- Manual GUI check items given to the user: (1) claude agent answers a real prompt to the end, (2) codex agent the same (never GUI-tested), (3) tray 「데몬 끄기」 → agent tree rows turn stopped (this was the regression) and icon gray; 「데몬 켜기」 → reconnects, (4) tray 「완전 종료」 with agents running → app exits at once.

## Repo state (measured 2026-11-10 13:12)
- Branch `v0.4.1/refactor/transport-attach`, HEAD `05cd2b1d`, **ahead 0 / behind 0** vs its origin branch (pushed; the handoff commit comes after this text), **24 ahead / 0 behind origin/master** (master already merged into the branch at `94777ae5`).
- Commits this session: `3467d53d` T-82 closed · `093b0eff` 3-2 U5b-1 · `befab4a6` 3-3 U1·U2 · `70804191` 3-3 U4·U5 · `80d1fecf` 3-3 U6 · `94777ae5` merge master · `df13d853` step-log/tracking · `5b54db2e`/`fc1cd8c0` ADR-0352 · `7b86663a` T-83 · `05cd2b1d` stop-order fix.
- Parked branch `v0.4.1/feat/connection-ui-parked` = `4d25dfa7` (3-2 U5b-2 frontend band buttons / confirm / LLM commands; review round 1 applied, not re-reviewed, no QA; local only, not pushed).
- Working tree: clean except the token folder above. Engram repo: feedback commit `78c7435` (and later `8535305` by someone else) — not pushed.

## Verification state
- 3-3 U1·U2: review light PASS; QA full regression PASS + claude lane PASS.
- 3-3 U4·U5: review deep (codex lifetime must withdrawn after ask-back); QA full regression PASS, mutation 1 survived → fixed in U6; GUI PARTIAL (tray not clickable — session locked).
- 3-3 U6: review full, 1 FIX round → PASS; QA full regression PASS, mutation 3/3; GUI: launch/attach, spawn + I/O PASS, stop/start via `daemon_shutdown`/`daemon_start` IPC PASS, quit via `quit_app` PASS — **tray clicks not run**.
- Stop-order fix `05cd2b1d`: review full PASS/PASS (bar = data loss / leftover processes / hangs); QA backend lines + gates 31 + frontend 2804 + tsc PASS; workspace run 1 failure `ws_e2e case09_slow_consumer_closed_others_unaffected` (daemon test, change is shell-only, passes alone on base → flaky, **T-83**); shell real-daemon lane 3/3 (cross-version skipped); mutation 2/2 killed. **Not verified: GUI** (user's manual check pending).
- GUI smoke of `093b0eff` (pre-3-3) PASS 5/5 with real tray clicks.
- Rerun full regression: per `.claude/skill-bindings/qa-verify.md` (backend lines, gates from ci.yml, frontend).

## Decisions
User-decided (this session):
- T-82 (old daemons' `daemon.json` at the root) · 상황: none of those releases matter · 선택지 C ★ release note / A / B · 고른 것 「아직 배포도 안했어. 그냥 무시하고 진행해」 · 갈림(no note) · 조건: 「뭐 남길 필요도 없고」.
- Band after a successful stop (first: A ① at flight end) · 고른 것 「A: 끈 즉시 「연결 안 됨」」 · 같음 — then reversed: (묻지 않음, Claude proposed after two rounds of ordering findings) 「ㅇㅇ 빼. 구지 바로 할 필요없음.」 → U6: a stop sets no band; reply `{outcome, reason}` only.
- U5b-1 GUI check · 선택지 combine with U5b-2 ★ / after work · 고른 것 「U5b-2와 묶기」 · 같음 (later done on `093b0eff`).
- Version band keeps the 「데몬 끄기」 button (user questioned it, reminded of their 2026-11-05 ADR-0344 decision) · 고른 것 「그럼 있다치고 뭐가 이슈인데?」 — kept.
- Failed band stop needs no text · (묻지 않음) 「버튼이 원래대로 돌아오는게 사인아님?」.
- Park U5b-2, finish transport first · 「일단 transport 부터 끝내고 진행해 뭐하는거야」.
- 3-3 scope = pure replacement · 「일단 기존거 교체하는 정도로 구현하라고 뭐 자꾸 안건 꺼내지 말고」 · 「배선 까지는 어쨋든 기존 동작이 되야해. 여기서 마스터 푸쉬 한번 가고 진행하면 될듯」 → 3-2 U6 deferred to multi-daemon, U7 out, T-80 out.
- Order after transport: connection UI (simplified, own topic) → multi-daemon (user: 「배선하고 이상한 UI 한다음에 멀티데몬」).
- Stop-order regression: discard the patch, restore the order · 선택지 root fix ★ / patch · 고른 것 「일단 땜질은 버리고」 · 같음 · 근거: 「연결 안되는건 그냥 다시 연결하면 되잖아. 데이터적으로 중요한 문제 없으면 상관이없을것같은데」 → ADR-0352.
- Delete all build caches · 선택지 all / all but the working folder · 고른 것 「전부 (약 380 GB)」.
- No handoff during the night run (「핸드오프 하지말고 쭉쭉 진행해」); now 「마무리 지을거 다 짓고 핸드오프 하면 될듯」.
Claude-decided: daemon gate 1/2 replace net gates 1/2b/4 (net gate 2a's platform wall not carried); 3-3 parity differences accepted (close codes, Host check, 16 MiB both ways, send closes on saturation); `BOOT_START_MARGIN` 5 s; T-83 recorded instead of fixing.
Open: none blocking. Next topic = connection UI simplification — inventory (Korean) at `.review-s33/connection-ui-audit.md` (git-excluded, local only): 56 mechanisms, 22 core, 26 removable (≈ 2,400 lines); the stopping-mode bundle it lists is already removed by `05cd2b1d`.

## Do-not / lessons
- Don't take review 권장 for rare orderings; ask back one-sided must instead of fixing by reflex (rule in Engram feedback `78c7435`).
- Don't let a user decision add a second writer of the same state without saying so — the "① at flight end" override produced two rounds of ordering findings.
- Don't bundle UI features into a plumbing topic (U5 grew from "move the stop client onto transport" into the whole connection UI).
- QA drivers must have a deadline: a driver polling for `__EXIT` without a timeout hung the QA for ~1 h after its build was interrupted.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -4 && gh run list -w ci.yml -c 05cd2b1d --json status,conclusion && git -C I:/Engram/apps/engram-dashboard rev-list --left-right --count HEAD...origin/master`

## References
- ADR-0352 (stop leaves the link open; amends ADR-0350 decision 3), ADR-0350.
- `docs/process/step-log.md` S21 stage-3 entry 2026-11-09~10; `docs/tracking.md` T-80, T-83.
- `docs/process/S21-crate-boundaries/trd-3-transport.md` §6-3 (3-3 units), `trd-3-2-u5-stop-and-band.md` (edition 5.3).
- Code: `crates/engram-dashboard-daemon/src/link/` (server, auth, handshake, wire), `crates/engram-dashboard-daemon/src/conn_port.rs`, `src-tauri/src/daemon_client/shutdown.rs` (stop path), `src-tauri/src/daemon_client/link/route.rs` (bands).
