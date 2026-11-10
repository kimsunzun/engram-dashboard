# Handoff: S36 connection band merged to master · next = multi-daemon (structure lecture → PRD HTML → TRD HTML)

## Know first
- **Mode: autonomous.** User wants plain words, no invented labels, few agenda items. Ask only what changes visible behaviour; decide internals and report. Fix bar = real breaks in normal use (data loss, leftover processes, hangs, a visibly failing requirement); rare orderings / connection churn accepted.
- **Commit messages are English** (CLAUDE.md 「언어」 now names them — user corrected this session; S36 commits before `673ac612` are Korean and stay).
- **Multi-daemon process rule (implement binding 「사용자용 HTML」):** PRD and TRD each get a Korean user-facing HTML (`docs/process/<S n>/prd.html`, `trd.html`, written with `/engram-explain`); md (English) stays the canonical text for reviewers/coders. PRD HTML = what/why/how it looks → user approval. TRD HTML = top-down learning (whole structure → processes and links → each part's role → detail); proceed to review/coding only when the user is convinced. Older standing condition (11-05 handoff): give the user the network/structure explanation before multi-daemon work.
- **Review binding:** screen-reader support is not a goal (a11y tree = LLM/test observation channel only).
- Untracked `.engram-dev.qa-bak-engram-qa-20261109-111846/` holds a daemon token — never stage it; `recycle.sh` fails on it; the user deletes it after a restart.
- A release check build of `3592a84d` may still be running (app PID 29096, daemon from `C:\Users\kimsunzun\AppData\Local\Temp\engram-check-3592a84d\`, data next to the exe). Old check folders `engram-check-df13d853\`, `engram-check-05cd2b1d\`, `engram-check-3592a84d\` in `%TEMP%` can be deleted when convenient.
- `scripts/build-release.ps1` needs `pwsh`. In the main session `python` is the Store stub — use `py` or the Edit tool.

## One-line state · next first action
S36 (connection band frontend) is **merged to master `a40aaefe`** and pushed: two bands from the shell's band, 「다시 연결」 on both (no stop button — ADR-0356), LLM `connection.status` / `connection.reconnect` / `daemon.stop`, `daemon_stop` removed, smaller band text.
**Next first action:** start the **multi-daemon topic** from master: get `S<n>` per `docs/handbook/branch-commit.md` (branch `v<next>/<type>/<slug>`), then first give the user the plain-words structure explanation (how shell ↔ daemon(s) ↔ windows connect today and what changes with N daemons), then PRD md + PRD HTML. Consider a light convention survey (multi-host/multi-daemon UI in other tools) before the PRD — none exists locally.

## Repo state (measured 2026-11-10 18:44)
- `master` = `a40aaefe` (merge of `v0.4.1/feat/connection-band` at `a0aad975`), pushed; `origin/master` equal.
- Branch `v0.4.1/feat/connection-band` = `e57858bf` (one docs-only step-log commit after the merge) + this handoff commit — pushed; not on master. Docs only (CI path-filtered); carry with the next chore or merge.
- Parked `v0.4.1/feat/connection-ui-parked` (`4d25dfa7`) — superseded by S36; keep or delete at will.
- Local ref `refs/review/s36-qa` (QA snapshot) and `.review-s36/` (git-excluded review bundles) — scratch, safe to delete.
- Untracked `.claude/explain/2026-11-10-connection-ui.html` — user explainer from before ADR-0356 (still shows the stop button); outdated, delete or keep as history.
- Engram repo: `master` ahead of origin by 55 (includes this session's `129716d` qa feedback row; other sessions' commits too) — not pushed.

## Verification state
- S36 code (`3592a84d`): PRD+TRD review full PASS (2 rounds); code review full PASS (Claude + codex, 2 rounds); QA PASS — CI three jobs locally (workspace 5487, lib_unit 1466, gates 31/31, vitest 2849, tsc), binding sync, real-daemon lane + `v0.3.2` cross-version measured, mutation 3/3, release GUI AC1–AC7 (AC3 via a copied record with a changed token; AC4 cause = missing daemon exe, the unwritable-folder path itself not measured). CI green on `3592a84d` and `a0aad975`.
- **Not reviewed:** `a0aad975` (band text 12px, medium head, tighter padding; also `StateFileNotice`) — class-only; layout + popout vitest 1058 and tsc passed; the user has not seen it on screen yet (no rebuild after it). Cover it with the next reviewed change.
- Rerun: `npm test` · `npx tsc --noEmit` · `cargo test -p engram-dashboard --test lib_unit` (see `.claude/skill-bindings/qa.md`).

## Do-not / lessons
- Don't follow old Korean commit history over CLAUDE.md 「언어」 — the rule lists what is Korean; everything else is English.
- A PRD non-goal "rare ordering" must be traced before it is called rare (S36 review M1: the stale start cause showed on the normal version-mismatch recovery path).
- Don't drop an ADR-decided item from scope silently — record it (T-85 was raised by review because R17/R15 vanished without a note).
- Amend + force-push of a pushed commit is blocked by the auto-mode classifier — fix with a new commit.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -3 master && git -C I:/Engram/apps/engram-dashboard status -sb | head -2`

## Decisions
User-decided (this session):
- Version band stop button · 상황: newer daemon cannot be stopped/downgraded by an older app, stop vs update · 선택지 keep 「데몬 끄기」 for older ★ (ADR-0344 status quo) / notice only · 고른 것 「높고 낮은거 빼고 안내만해. 데몬은 알아서 끄라고하고. 나중에 업데이트 시스템에서 자세히 다루면 될듯」 · 갈림 → ADR-0356, T-84.
- 「다시 연결」 on band ② · 상황: after an external stop the refused link does not redial, ② would stick · 선택지 add 「다시 연결」 ★ / text only · 고른 것 「ㅇㅇ 그냥 다시연결로해. 끄기 말고. 그게 좋아보이네」 · 같음.
- Boot start per window (T-85) · 상황: after a stop, a new window starts the daemon again; pre-existing; ADR-0350 R17 not landed · 선택지 record and defer ★ / fix now (~150 lines) · 고른 것 「나중에 다중에이전트 선택할때 결정하는게 좋을듯? 일단은 이대로 유지하자」 (confirmed = multi-daemon) · 같음.
- Screen reader / keyboard focus on the band · 선택지 leave ★ / fix · 고른 것 요지: 스크린리더 지원 불필요 (「음? 스크린리더 지원이 필요함?」) · 같음 · review binding updated.
- Tray start leaves the slot icon (T-86) · 상황: found in the user's release check · 선택지 defer ★ (if 「다시 연결」 clears it) / investigate before merge · 고른 것 「어차피 멀티데몬할때 트레이도 개선해야되니 나중에 다시 오픈하자」 · 같음 · 조건: reopen with the multi-daemon tray work.
- Band font · (묻지 않음) 띠 폰트 · 선택지 없음 · 고른 것 「띠 폰트좀 기존거랑 동일하게 깔끔하게(그리고 너무큼)」 → `a0aad975`.
- Multi-daemon PRD/TRD as user HTML · (묻지 않음) 진행 방식 · 고른 것 「멀티데몬 만들때 prd trd 각각 html 양식으로 진행해야될듯. trd는 탑다운 식으로 내가 학습하는 식으로 보고 … 내가 어느정도 수긍하면 진행하는 스탭으로」 + 「html은 내가 볼거 md는 영어잖아」 → implement/explain bindings.
- Merge after the user's own test · 「내가 간단히 테스트하고 마스터에 넣으면 될듯」 → merged `a40aaefe`.
Claude-decided: rebuild S36 on master instead of rebasing the parked branch; take the R21 cause guard (review M1); `connection.reconnect` failure = error (no status re-pull); `shutdownDaemon` as a free function (no stop on `__ENGRAM_DAEMON__`); keep `platform::process::kill_tree`; `can_stop` stays on the wire unread; band font 12px / medium; ADR-0350 left unedited (T-85 carries the deferral).
Open: none blocking.

## References
- Multi-daemon material: ADR-0179 (daemon accepts N clients; the shell — not windows — attaches to N daemons); ADR-0310 + `docs/process/S21-crate-boundaries/trd-3-transport.md` (「multiple daemons = the very next stage」; N-peer foundation laid; daemon-to-daemon vocabulary, remote link, security redesign deferred to the multi-daemon PRD; deferred unit 3-2 U6); TRD S21 D2 carry-over (the 「다시 연결」 button targets one daemon; a remote daemon can only be attached).
- Research: `docs/research/multi-agent-hosting-orchestration-research-2026-06-22.md`, `reusable-transport-crate-boundary-2026-09-03.md`, `client-without-daemon-message-2026-11-07.md`.
- Tracking: T-85 (who starts a daemon), T-86 (tray start leaves the slot icon), T-84 (update system), T-80, T-81 (tray out of the app process), T-83.
- S36: `docs/process/S36-connection-band/` (prd.md, trd.md, README.md), ADR-0356, step-log `## S36`.
- Process rules: `.claude/skill-bindings/implement.md` 「사용자용 HTML」, `.claude/skill-bindings/explain.md`.
