# Handoff: S33 English translation — all ADRs and docs/research done; next = other docs/ md (stop before code comments)

## Read first

- Review is **Claude only** from now on (user 2026-11-06: 「아니 codex 리뷰까지 빡세게 하지 않는게 좋을것같아 이건 claude 만으로만 진행해줘.」). Do not call codex for this topic.
- **Avoid files that open branches edit** (user: 「일단 현재 다른 작업하고 겹치지 않는것 위주로 최대한 하면됨.」). Open branches: `v0.5.0/feat/preset-library` (wt1), `v0.4.1/refactor/transport-attach` (main checkout). Re-check with `git diff --name-only master...<branch>` before each new area.
- **Stop before code comments** and present a plan (user: 「그리고 쭉 해도 코드 단계에서는 한번 멈춰줘.」).
- The toolkit scripts in `.claude/handoff/attachments/s33-translation-kit/` hard-code the old session scratchpad path (`C:\Users\kimsunzun\AppData\Local\Temp\claude\...\c358588a-...\scratchpad`). Copy the kit into the new session's scratchpad and fix the `SP=` line in `stage.sh` / `finalize.sh` / `finalize_dir.sh` and the paths inside the brief files.
- On this PC `python` / `python3` in Git Bash are the Windows Store stub (print "Python", exit 49, run nothing). Use `py`.

## One-line state · next first action

- State: ADR-0001..0309 (minus 13 deferred) and all 134 `docs/research` reports are English and committed on `v0.5.0/docs/english-translation`, one commit per batch, each batch reviewed and fixed.
- Next first action: translate the remaining `docs/` markdown listed in `s33-translation-kit/otherdocs-md.txt` (101 files, ~2.75M chars), excluding the deferred set below. Start with the small/medium folders (`otherdocs-folder-sizes.txt`, 91 files ~1.75M chars, grouped ≤100k chars per worker). The 10 files over 90k chars (`otherdocs-big-files.txt`, ~1.67M chars, largest `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` 297k) must be split at headings, translated per chunk, and joined — no split/join script exists yet.
- Exclude from "other docs": `*.html` (user-facing reports stay Korean per CLAUDE.md 「언어」), png, `docs/process/S33-english-translation/` (glossary/plan intentionally contain Korean terms), and the deferred set.

## Repo state (measured at save, 2026-11-06 11:38)

- Branch `v0.5.0/docs/english-translation`, HEAD `5d5d1dc1`, 23 commits ahead of local `master`, 0 behind. **Not pushed** (no remote branch). The handoff commit comes after this text.
- Uncommitted: only `.claude/handoff/latest.md` (this save overwrites it; its old content equals `history/20261029-195219-s29-우편-id-키-master-머지-완료.md`). Untracked `.claude/handoff/attachments/*` (pre-existing + new `s33-translation-kit/`).
- Topic claim: S33 heading + `docs/process/S33-english-translation/{plan.md,glossary.md}` committed locally, not pushed (S33 number not reserved on the remote).

## Verification

- Ran per batch: `py check_doc.py <orig> <out>` (code blocks, backtick spans, link targets, ADR refs, 「」 quotes identical; ADR lint tokens kept; `--no-adr` for non-ADR docs) — all batches 0 ERR. ADR lint after every ADR batch: `node ~/.claude/skills/engram-adr/scripts/adr.mjs lint --anchor-roots crates,src,src-tauri,scripts` → errorCount 0 (1 pre-existing advisory, ADR-0027 anchor in `src-tauri/src/tray/mod.rs`). ADR index regenerated with `adr.mjs index --write`.
- Meaning review: codex xhigh for ADR-0006..0180; Claude `worker-scout` (Opus medium) review-and-fix for ADR-0181..0309 and all research. Every batch had findings (high roughly one per 7–20 files); all applied.
- Not verified: no build/test run (docs only, no code touched). CI not run (branch not pushed; docs paths are excluded by the ci.yml path filter anyway — confirm with `docs/handbook/branch-commit.md` before merge). The 31 research files and ~69 new ADRs that open branches add are not on master and are untranslated.
- Known residual: many 「」 quotes stay Korean, some without an English gloss (earlier batches); a later gloss pass may be wanted. Rough count: 387 Korean lines outside the keep list across 118 ADR files (mostly kept quotes/tags).

## Failed approaches (do not repeat)

- `git add docs/decisions/0{136..180}-*.md` fails as a whole when a number does not exist (ADR-0170 is missing) — stage `docs/decisions/` as a folder.
- Restoring backtick spans / 「」 quotes by position after a first draft swapped "X 의 Y" pairs and containment (research r4, r6). Translators must write those spans verbatim in the first draft.
- codex xhigh reviews of ~230 files exhausted the codex 5-hour then weekly quota (`You've hit your usage limit ... try again at Nov 7th, 2026 8:55 PM.`). Six parallel Sonnet translators plus main hit the Claude session limit once (`You've hit your session limit · resets 6:50am (Asia/Seoul)`). Keep ≤4 workers in parallel.

## Resume command

`git -C I:/Engram/apps/engram-dashboard-wt2 log --oneline master..v0.5.0/docs/english-translation | head -30`

## Decisions

User decisions:
- (묻지 않음) translation scope · 상황: Korean comments/docs across the repo, ~94k lines · 선택지 없음 · 고른 것 「문서 주석만 다 바꾸고 안에 내용? 같은 건 한글로 놔두면 되지 않음?」 then 「내가 전체라고 했잖아. 과거기록도 그렇지.」 · 추천과 — · 조건: string literals stay Korean, archives included.
- Commit granularity · 상황: many batch commits vs one big commit · 선택지 one commit per batch on one topic ★ / one squashed commit · 고른 것 요지: accepted per-batch after explanation · 추천과 같음 · 근거(Claude): revertable per batch, no long-lived conflict.
- Translator model · 상황: A/B on ADR-0021..0030, Sonnet 1H+3L vs Opus 1H+1L, Sonnet 2x faster · 선택지 Sonnet + review ★ / Opus · 고른 것 「ㅇㅇ 몇개 더 비교해봐. 그냥 너가 쭉한다음에 결론 나오면 그 방향으로 쭉하자.」 → Sonnet · 추천과 같음.
- Reviewer after codex quota ran out · 상황: codex blocked until 11/07 20:55 · 선택지 wait for codex ★ / Claude Opus review / buy codex credits · 고른 것 「이건 claude 만으로만 진행해줘.」 · 추천과 갈림.
- (묻지 않음) overlap · 상황: open branches edit 13 translated ADRs · 선택지 없음 · 고른 것 「현재 다른 작업하고 겹치지 않는것 위주로 최대한 하면됨.」 · 추천과 —.
- (묻지 않음) stop point · 고른 것 「코드 단계에서는 한번 멈춰줘.」.
- (묻지 않음) handoff timing · 고른 것 「이번 뭉탱이작업 다 끝나면 핸드오프 하는게 좋아보임.」.

Claude decisions:
- Keep in Korean: text in 「」, app UI strings, user quotes, ADR lint tokens (`- 상태:` + vocab, `- 관련:`, `## 맥락/결정/거부한 대안/근거/영향 / 불변식`), CLAUDE.md/.claude section names, backtick spans, code blocks; ADR file names stay Korean (rename = separate script-only batch at the end).
- Review-and-fix in one agent (Opus medium) instead of reviewer + fixer, to save tokens.
- Deferred 13 ADRs restored to master Korean: preset-library edits 0061 0070 0092 0109 0264 0279 0291; transport-attach edits 0129 0177 0178 0180 0181 0272. Reviewed translations kept in `s33-translation-kit/deferred-adr/` — after those branches merge, re-apply on top of their edits (their changes will be Korean and need translating too).
- `.git-blame-ignore-revs` to be added at integration with all S33 translation commit hashes.

Open (not decided):
- Push the branch for backup? (asked, no answer).
- Gloss pass for 「」 quotes left without English.
- Code comments plan (must stop and ask).

## In progress

- Nothing running at save.

## Stop conditions

- Before touching any code comment (`*.rs`, `*.ts`, `*.tsx`, …): stop and present a plan.
- Before translating a file that an open branch edits (check `git diff --name-only master...<branch>`): skip it and list it as deferred.
- Before `step-log.md`, `tracking*.md`, `testing-strategy.md`, `todo*`, `docs/process/S21-crate-boundaries/`, `docs/process/S25-preset-library/`: skip (high churn / open branch).

## References

- `.claude/handoff/attachments/s33-translation-kit/` — `progress.md` (pipeline notes), `brief-batch.md` (translator brief), `research-note.md` (non-ADR override), `brief-review-fix.md` (review-and-fix brief), `resume-note.md`, `check_doc.py`, `stage.sh` / `finalize.sh` (ADR) / `finalize_dir.sh` (other dirs), `otherdocs-*.txt`, `deferred-adr/`.
- `docs/process/S33-english-translation/glossary.md` (renderings and traps), `plan.md`.
- `docs/process/step-log.md` § S33.
