# Handoff: S21 stage 3 transport merged to master · next = connection UI as its own simple topic

## Know first
- **Mode: autonomous.** User wants plain words, no invented labels, no agenda items — 「뭐 자꾸 안건 꺼내지 말고」. Ask only what changes visible behaviour; decide internals and report.
- **User's bar for fixes (2026-11-10):** fix only real breaks seen in normal use — **data loss, leftover processes, hangs**. Connection churn (an extra redial, a band a moment late) is accepted — 「연결 안되는건 그냥 다시 연결하면 되잖아」. Do not take review 권장 items for rare orderings (Engram feedback `78c7435`); user called the earlier over-fixing 「별 필요없는 부분까지 다 수정하는 느낌」 · 「transport 그냥 연결만 하는 문제였는데 뭔 복잡하게 하냐고」. Keep the next topic small and simple.
- **Untracked `.engram-dev.qa-bak-engram-qa-20261109-111846/` holds a daemon token** — never stage it (`git add -A -- . ':(exclude).engram-dev.qa-bak-*'`). `recycle.sh` fails on it (`code=0x78`); the user will delete it after a restart.
- `scripts/build-release.ps1` needs **`pwsh`** (PowerShell 7). All cargo `target` folders under `I:\Engram\apps` were cleaned 2026-11-10 — first builds are cold.
- Check builds left in `%TEMP%`: `engram-check-df13d853\`, `engram-check-05cd2b1d\` (release builds for manual checks; app may still be running from the latter) — delete when convenient.

## One-line state · next first action
Stage 3 transport is **done and merged into master** (`0f9e88ec`): the daemon accepts over the transport, the net crate is gone, and the stop leaves the shell link open (ADR-0352) so the agent tree updates on a stop; the user checked a release build: 「잘 되는것 같아」.
**Next first action:** start the **connection UI topic** (what the user called 「프론트엔드 메세지」) as its own topic from master — get a new `S<n>` per `docs/handbook/branch-commit.md` (branch `v<next>/<type>/<slug>` off master). Material: the parked frontend work `v0.4.1/feat/connection-ui-parked` (`4d25dfa7`) and the mechanism inventory `.review-s33/connection-ui-audit.md` (Korean, git-excluded, local only). First, show the user in plain words what the connection UI will do (bands 「연결 안 됨」/「버전 다름」 with 「다시 연결」/「데몬 끄기」, LLM commands `connection.status` / `connection.reconnect` / `daemon.stop`) and which mechanisms from U5a/U5b-1 to strip — keep it minimal. Then multi-daemon (3-2 U6 seat was deferred there).

## Repo state (measured 2026-11-10 14:13)
- `master` = `0f9e88ec` (merge of `v0.4.1/refactor/transport-attach`), pushed; `origin/master` equal.
- Branch `v0.4.1/refactor/transport-attach` = `1233d706` — 1 docs-only commit (step-log merge note) after the merge, **not on master and not pushed** (the handoff commit comes after this text). Carry it with the next chore/misc or merge it; it is docs only (CI path-filtered).
- Parked branch `v0.4.1/feat/connection-ui-parked` = `4d25dfa7` (based on `093b0eff`; local only, not pushed; review round 1 applied, not re-reviewed, no QA). It predates ADR-0352 (stopping mode removed) — rebase or cherry-pick with care.
- Engram repo: feedback commit `78c7435` — not pushed.

## Verification state
- Merged code: CI green on `05cd2b1d` (backend · frontend · gates); later commits are `.claude/handoff` / `docs` only.
- Stop-order fix `05cd2b1d`: review full PASS/PASS (bar = data loss / leftover processes / hangs); QA backend lines + gates 31 + frontend 2804 + tsc PASS; shell real-daemon lane 3/3 (cross-version skipped); mutation 2/2; one workspace-run failure `ws_e2e case09_slow_consumer_closed_others_unaffected` = flaky (T-83, daemon test, passes alone); **user GUI check on a release build: OK**.
- 3-3 U1–U6: reviews passed, QA full regression PASS each; GUI earlier PARTIAL (tray clicks) — covered by the user's manual check of `05cd2b1d`.
- Not verified: codex agent in GUI was part of the user's check list (user reported overall OK, details not itemised); cross-version stop (v0.3.2 daemon) not rerun since U5a.

## Decisions
User-decided (this session):
- T-82 closed without action · 「아직 배포도 안했어. 그냥 무시하고 진행해」 · 조건: 「뭐 남길 필요도 없고」.
- U6 (TRD): a stop sets no band; reply `{outcome, reason}` · 「ㅇㅇ 빼. 구지 바로 할 필요없음.」 (after a briefly chosen "① at flight end").
- Version band keeps 「데몬 끄기」 (ADR-0344, user's own 11-05 decision) · 「그럼 있다치고 뭐가 이슈인데?」; failed stop needs no text · 「버튼이 원래대로 돌아오는게 사인아님?」.
- Park U5b-2, finish transport first · 「일단 transport 부터 끝내고 진행해 뭐하는거야」.
- 3-3 = pure replacement · 「일단 기존거 교체하는 정도로 구현하라고 뭐 자꾸 안건 꺼내지 말고」 · 「배선 까지는 어쨋든 기존 동작이 되야해. 여기서 마스터 푸쉬 한번 가고 진행하면 될듯」 → 3-2 U6 deferred to multi-daemon, U7 and T-80 out.
- Order: wiring → connection UI → multi-daemon · 「배선하고 이상한 UI 한다음에 멀티데몬」.
- Stop-order: discard the patch, restore the order · 선택지 root fix ★ / patch · 「일단 땜질은 버리고」 · 같음 · 근거: 「연결 안되는건 그냥 다시 연결하면 되잖아. 데이터적으로 중요한 문제 없으면 상관이없을것같은데」 → ADR-0352.
- Delete all build caches · 「전부 (약 380 GB)」.
- Merge after the GUI check · 「ㅇㅇ 잘 되는것 같아. 머지 하면…」 → merged `0f9e88ec`.
Claude-decided: daemon gate 1/2 replace net gates (net gate 2a's platform wall not carried); 3-3 parity differences accepted (close codes, Host check, 16 MiB both ways, `send` closes on saturation); T-83 recorded, not fixed.
Open: none blocking.

## Do-not / lessons
- Don't take review 권장 for rare orderings; ask back one-sided must instead of fixing by reflex.
- Don't let a user decision add a second writer of the same state without saying so.
- Don't bundle UI features into a plumbing topic (U5 grew from "move the stop client onto transport" into the whole connection UI).
- QA drivers must have a deadline (a driver polling for `__EXIT` without a timeout hung QA for ~1 h).

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -3 master && git -C I:/Engram/apps/engram-dashboard log --oneline -1 v0.4.1/feat/connection-ui-parked && git -C I:/Engram/apps/engram-dashboard status -sb | head -2`

## References
- ADR-0352, ADR-0350, ADR-0344 (version band button), ADR-0345 (「다시 연결」 on the band).
- `docs/process/S21-crate-boundaries/trd-3-2-u5-stop-and-band.md` (edition 5.3 — §4 bands / §4-4 LLM commands are the starting design for the UI topic; R-rules about stopping mode are gone).
- `.review-s33/connection-ui-audit.md` (local) — mechanism inventory with keep/remove recommendations.
- `docs/process/step-log.md` S21 stage-3 entries 2026-11-09~10; `docs/tracking.md` T-80, T-81, T-83.
