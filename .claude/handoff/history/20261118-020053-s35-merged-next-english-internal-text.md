# Handoff: S35 (UI language — English source, Korean selectable, live switch) merged to master; next = English-only internal text topic

## Read first

- S35 is done and on master (`36a055da`). Do not reopen it. The next topic is the one queued before S35's scope grew: logs · panic/assert messages · internal error strings · LLM-facing text (MCP descriptions, `engram help`, `#[tool(description)]`, settings `desc`) to **English only** (not switchable), plus rewriting `docs/reference/logging-conventions.md` ("log = one line of Korean") and Korean prose in reference docs' fenced blocks. The number S36 is taken (connection band) — get a new `S<n>` per `docs/handbook/branch-commit.md`. After it: test names and test messages to English.
- The user wants the LLM help (`prompts/engram-help.md`) reorganized wholesale in that next topic (「도움말도 나중에 한꺼번에 고쳐야」). S35 only added one `ui.language` line.
- Two other sessions were told to pull master and adapt (cross-session messages sent 2026-11-18 ~02:00): `engram-dashboard-wt1-67` (preset-library, wt1) and `engram-dashboard-80` (main worktree). Nothing to do here unless they report problems.
- Integration lesson: after a red CI run, a docs-only push creates **no** CI run (path filter). Check that a run exists right after push; if not, `gh workflow run ci.yml --ref <branch>` (handbook). A 5-hour wait was lost to this.

## One-line state · next first action

- State: S35 merged (`36a055da`, CI green run 44127577152 on `0ab01c9d`). Branch `v0.5.0/feat/i18next-backend` is fully pushed and merged (not deleted, per rule).
- Next first action (when the user asks): start the English-only internal text topic — get `S<n>`, cut `v0.5.0/<type>/<slug>` from master, PRD first (`/engram-implement`). The earlier scope notes are in `docs/process/S35-i18next-backend/log-localization-notes-2026-11-08.md` (OSS practice: logs stay English, user-facing errors localized via keys).

## Repo state (measured 2026-11-18 02:00)

- Worktree `wt2`, branch `v0.5.0/feat/i18next-backend`, HEAD `0ab01c9d`; 0/0 vs its remote; vs `origin/master` 1 behind (the merge commit) / 0 ahead. Working tree clean (untracked `.claude/handoff/**` and `bindings/` are old, not ours). This handoff's commit comes after this text.
- master = `36a055da` (S35 merge), pushed.
- Engram repo (I:/Engram) got feedback commits `36af747`, `54493ff` (not pushed — feedback rule says no push).

## Verification

- S35 QA PASS: U1 tree `72b43e2d`; U2+U3 tree `9cdedfa6` — CI three jobs locally (cargo workspace 5513, vitest 2912, tsc 0, 31 gate steps), mutation caught on 4 lines, GUI scene (fresh English; live `ko` with popout + running terminal: menus, open flyout, tray, tab names, OS titles switch, terminal kept; reload paints Korean first; restart keeps Korean; reset → English; `fr` refused; Korean state file loads as auto names).
- CI green on `0ab01c9d` (workflow_dispatch run 44127577152). Re-run: `gh run view 44127577152`.
- Not verified: T-113 (below) on screen; the connection band in the GUI (never shown during QA; unit-tested).

## Failed approaches (do not repeat)

- Long ADR titles: `adr.mjs` makes the file name from the H1 with no length cap; a 238-char name broke the GitHub Windows checkout (`Filename too long`). Keep ADR H1s short (fixed by renaming ADR-0373; feedback row in engram-adr).
- Waiting on `gh run list -c <sha>` for a run that was never created (see "Read first").
- QA "token-free terminal agent": there is no shell backend in production and the `engram` CLI needs an agent token; use `invoke('forward_daemon_command', …)` from the webview for settings commands in GUI QA (feedback row in engram-qa).

## Resume command

`git -C I:/Engram/apps/engram-dashboard-wt2 log --oneline -3 origin/master`

## Decisions

User decisions (this session):
- (묻지 않음) localization now · 상황: S35 was paused as "English source only" · 선택지 없음 · 고른 것 「로컬라이징? 그냥 진행시키는게 좋을것같은데 디폴트 영어 선택 한국으로」 · —.
- language switch surface · 상황: no settings screen exists, settings are LLM commands · 선택지 settings UI / LLM command only · 고른 것 「언어를 고르는 설정이 있긴한데 llm 명령으로만 일단 하자고」 · 미상.
- apply timing · 상황: main recommended restart (VS Code style) · 선택지 restart ★ / live · 고른 것 「보통은 로컬적용될때 재부팅하는경우는 없는데」 (live) · 갈림.
- what is localized · 고른 것 「로그는 로컬라이징 대상이 아니고, 내부 코드나 시스템은 다 영어로 하고 내가 보는 UI쪽은 전환되는게 맞고(이것도 기본 영어로 해줘…)」 · —.
- status labels in Korean mode · 상황: Roo/OpenWebUI/LibreChat translate them · 선택지 only `Wait` English ★ / all status labels English · 고른 것 「ㅇㅇ 그렇게 해. 나중에 이상하면 수정지시하면 어차피 테이블만 수정하면 되니」 · 같음 · 조사 결과(번역한다)와 다르게 정함.
- split labels · 고른 것 「가로분할 세로분할이 영어에서는 영문으로 되야지」 (keep literal `Split horizontally/vertically`) · 미상.
- restore modal wording · 선택지 keep "screen" ★ / browser "session" · 고른 것 「관행으로 처리해. 다른곳에 많잖아.」 (session) · 갈림.
- punctuation · 고른 것 「마침표는 공용 룰이니깐 간단하게 리서치하고」 → 「ㅇㅇ 기준대로 해」 (Microsoft/Apple: sentences end with a period; labels/buttons/fragments do not).
- LLM help line · 선택지 add one line ★ / defer · 고른 것 「어쨋든 만들면 되긴하는데 도움말도 나중에 한꺼번에 고쳐야되긴하지」 · 같음.
- unfixed 권장 + T-113 + merge · 선택지 defaults ★ · 고른 것 「ㅇㅇ 기본안대로 하고 마스터에 넣어.」 · 같음.
- other branches · 고른 것 「그냥 우리 작업은 어쨋든 다 하고 다른데서 맞추라고 하는게 맞아보여서」, later 「통합한뒤에 너가 세션에 보내면되고」.

Claude decisions: recorded in ADR-0373 (lookup-only i18next, no kept translated text, layout-held language with version bump, serialized hook off the main thread with posted tray relabel, init-script language memory, `is_word` as a bilingual seed filter); kept `LayoutLeaf`'s `useLanguage()`; accepted the coder's punctuation exception for values ending in `{err}`/`{url}`.

Open (not decided): none blocking. Left as is by the user's "defaults": unfixed recommended items — a flyout left open while its menu moves on a switch keeps its old anchor until re-hovered; a usageFormat test leaves `<html lang>` as `en` at file end; `MenuRow` subscribes to the language on leaf rows too. T-113 (suspected per-keystroke remount of the agent-tree rename input; pre-existing; not reproduced) is in `docs/tracking.md`.

## Process notes for the user's flow review (asked this session)

- TRD carried code-level detail (counts, file inventories) against implement flow §0-1 ("TRD에 코드를 보면 아는 세부는 쓰지 않는다"); those belonged in coder briefs. Counts were corrected twice in a doc nobody reads after freezing — wasted rounds.
- Feedback rows written: engram-qa (token-free CLI path in GUI QA), engram-implement (HTML rule scope ambiguity), engram-adr (slug length cap, 2nd occurrence).

## References

- `docs/process/S35-i18next-backend/` (`prd.md`, `trd.md` Implemented, `README.md`); `docs/reference/structure/i18n.md`; ADR-0372, ADR-0373.
- `docs/process/S35-i18next-backend/log-localization-notes-2026-11-08.md` — input for the next topic.
- `docs/reference/logging-conventions.md` — to be rewritten by the next topic.
