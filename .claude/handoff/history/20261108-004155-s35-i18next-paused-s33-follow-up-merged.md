# Handoff: S33 follow-up translation merged; S35 (i18next behind t()) paused after a deep TRD review + an English-source direction change

## Read first

- S35 is **paused by the user** (「나중에 토큰 남을때」). Do not start coding. Resume = revise PRD/TRD/ADR for the English-source decision below, then re-review.
- `preset-library` (worktree `wt1`) is still open. Its session translates its own files after pulling master (user agreed). Guide: `.claude/handoff/attachments/s33-translation-kit/GUIDE-translate-your-branch.md` (untracked kit, absolute path readable from other worktrees). Do not translate files in `git diff origin/master...v0.5.0/feat/preset-library`.
- Review is Claude + codex now (codex worked this session). Keep ≤4 workers in parallel.

## One-line state · next first action

- State: S33 translation fully merged for everything except preset-library's files (master `d4b31567`). S35 branch has ADR-0349, research report, approved PRD, TRD ed.1 with the deep-review findings recorded but not applied. No S35 code.
- Next first action (when the user resumes S35): ask the two open questions under Decisions → Open, then revise PRD/TRD + write the ADR amending ADR-0349 + edit CLAUDE.md 「언어」, re-run `/engram-review trd` (deep), then `/engram-implement` coding.

## Repo state (measured 2026-11-08 00:41)

- Worktree `wt2`, branch `v0.5.0/feat/i18next-backend`, HEAD `64e3262e`, 5 ahead / 0 behind `origin/master`, pushed (0/0 vs its remote). Working tree clean (only pre-existing untracked `.claude/handoff/attachments/*` and old `.claude/handoff/history/*` from other sessions). This handoff's commit comes after this text.
- master = `d4b31567` (S33 follow-up merge), pushed. Branches `v0.5.0/docs/english-translation`, `-sweep`, `-2` are merged (not deleted).

## Verification

- S33 follow-up (merged): per batch `comments.py verify` (code byte-identical minus comments), crate tests, `npx vitest run` 124 files / 2804 tests, CI green on `v0.5.0/docs/english-translation-2` (run 42382655206). Docs: `check_doc.py` per batch; 3 accepted ERR lines (TRDs whose source has unbalanced backticks: trd-2-3 part03 L16, trd-1-3 §5-1, trd-2-1 L232 — prose inside pseudo-spans translated, identifiers checked byte-identical).
- S35: no code, nothing to run. TRD review = deep, FIX ×3 (details in `docs/process/S35-i18next-backend/trd.md` §10).

## Failed approaches (do not repeat)

- `finalize.sh` copies `<b>/out/*.md` into `docs/decisions/` by **flat name**; with staging that uses `docs__decisions__…` flat names it created bogus files. For flat-named batches use the `${n//__//}` copy (as `finalize_other.sh`), then `adr.mjs index --write`.
- i18next's own interpolation cannot keep our `{\w+}` grammar (matcher `\{(.+?)\}`, trims names, `{-x}`, `{x,fmt}`) — reproduced by a probe (`scratchpad/s35-parity-probe.mjs`, may be gone).
- `rm -rf` is blocked; use `bash ~/.claude/references/recycle.sh`. Foreground `sleep N && …` is blocked; use `until …; do sleep 5; done` in background.

## Resume command

`git -C I:/Engram/apps/engram-dashboard-wt2 log --oneline origin/master..v0.5.0/feat/i18next-backend`

## Decisions

User decisions:
- (묻지 않음) LLM-facing doc comments · 상황: schemars / declare_commands doc comments become runtime text an LLM reads · 선택지 translate ★ / keep Korean · 고른 것 「맞긴한데 … 어쨋든 진행해」 · 추천과 같음.
- integrate S33 now · 상황: S33 avoided open-branch files, merge clean · 선택지 integrate now ★ / wait for open branches · 고른 것 「ㅇㅇ 이번에 하고 다른데 업뎃 받으라고하고 다른데 다 되면 말하면 그때 번역하면 될듯. 진행해.」 · 추천과 같음.
- other branch translates itself · 상황: preset-library session asked to translate its own files · 선택지 S33 translates after merge ★ / that branch translates · 고른 것 「저쪽에서 번역한다고하네」 + 「다른한개 번역은 너가 가이드 줘봐」 · 갈림.
- localization library · 상황: home-made `t()` + `ko.ts` (ADR-0069), research shows i18next majority · 선택지 i18next now / later ★(main's second view) / react-intl · 고른 것 「ㅇㅇ 교체하고」 · 갈림 (main first recommended now, then later; user chose now) · 근거: 「반대로 얼마 안되면 그냥 지금 바꾸면 되는거아님?」.
- S35 scope stage 1 only · 고른 것 「1단계 부터 해. 로컬라이제이션은 나중에 토큰 여유있을때.」 · 추천과 같음.
- UI source / default language · 상황: CLAUDE.md says app screen text = Korean · 선택지 logs only / English source shown as Korean ★ / English source and default · 고른 것 「화면 원본 언어도 영어로 하고 한국어 보이는걸 나중에」 · 갈림.
- logs to English · 고른 것 「로그 영어화는 하고」 · 추천과 같음.
- pause S35 · 고른 것 「이거 나중에 토큰 남을때 하면 될것같아서」, 「핸드오프 하면 될듯」.

Claude decisions:
- S35 interpolation: keep our own `{\w+}` interpolate and use i18next for lookup only (`skipInterpolation: true`) — exact parity; rejected filtering param names (leaves trim / `{x,fmt}` gaps). Lead reviewer filed it as `hold` (PRD/ADR say "replace lookup-and-interpolate") → confirm with user.
- S35 includes the Rust tray (4) + refusal dialog strings under "English UI source" (main default, user may veto).
- 앞잡이 줄 = read-ahead queue etc.: glossary extended (transport terms, resume marker).

Open (not decided):
- S35: confirm the lookup-only design (hold H1); confirm Rust tray/dialog in scope.
- S36 (not opened): English for logs (yes) + panic/assert messages + internal error strings + LLM-facing strings (main recommends 1-4, excludes test names) — user asked what 5 (test names) is, did not answer 1-4 yet. Needs `docs/reference/logging-conventions.md` rule change ("log message = one line of Korean").
- Residue: Korean prose inside fenced ```text blocks (TRD §0 summaries, a few comment blocks); `#[tool(description = …)]` Korean string literals (part of S36 LLM-facing strings).

## References

- `docs/process/S35-i18next-backend/` — `prd.md`, `trd.md` (§10 review record + pending decisions), `log-localization-notes-2026-11-08.md`.
- `docs/decisions/0349-*.md` (amends ADR-0069); `docs/research/i18n-oss-library-reevaluation-2026-11-07.md`.
- `docs/process/step-log.md` § S33 and § S35.
- Kit: `.claude/handoff/attachments/s33-translation-kit/` (`progress.md`, `GUIDE-translate-your-branch.md`, tools).
