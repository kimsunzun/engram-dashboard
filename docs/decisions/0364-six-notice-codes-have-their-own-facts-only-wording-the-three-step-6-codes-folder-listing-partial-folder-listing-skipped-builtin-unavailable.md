# ADR-0364: Six notice codes have their own facts-only wording: the three step-6 codes, folder-listing-partial, folder-listing-skipped, builtin-unavailable

- 상태: 확정 (2026-11-13, 근거: S25 step 7 — main's wording for the three new codes (B22 · P6 · P8 hold 2); user decisions D2 (2026-11-11) and B29 (2026-11-12) on the codes not added · landed `72effa1d` (P6) · `f42d8538` (P8))
- 관련: Amends ADR-0357 (decision 3 and the 영향 wording arms, the codes with their own wording) · ADR-0337 (decision 3 — facts only, no fix) · ADR-0341 (decision 5 — notices once per agent) · ADR-0361 (`builtin-unavailable`) · ADR-0363 (`folder-listing-skipped`) · child TRD `docs/process/S25-preset-library/trd-asset-library.md` B22 · D2 · B29

## 맥락
ADR-0357 decision 3 gives three codes their own wording (`codex-terminal-omitted`, `codex-terminal-folder-omitted`, `codex-same-name-skills`) and sends every other code to the generic wording 「알림(<code>): 의도된 동작입니다.」 (with 「 대상 — <values>」). Step 7 added three notice codes:
- `folder-listing-partial` — a folder entry's listing stopped at the 10,000-entry cap (child TRD U2 · B22);
- `folder-listing-skipped` — folders or entries below a folder entry could not be read and were left out (ADR-0363);
- `builtin-unavailable` — a built-in could not be used and an empty stand-in was applied (ADR-0361).
Two of them report a failure, and all three need a value (the folder, the skipped paths, the built-in and why) to be useful. The generic wording says the notice is intended behavior and lists bare values; for a failure that sentence is false. ADR-0357 「영향」 names the wording arms of `own_wording` as the three step-6 constants only.
Step 7 also dropped notices that earlier drafts planned: D2 removed `settings-backend-field-absent` and `item-other-backend`, and the user withdrew `working-folder-unchecked` (B29), which had departed from ADR-0357 decision 3 with a fallback of its own.

## 결정
1. **Six codes have their own wording** in agent `launch.rs` `own_wording`: the three of ADR-0357 decision 1 (unchanged) and
   - `folder-listing-partial` (`args` `{folder}`) — 「알림: 폴더 <폴더>의 항목이 10,000개를 넘어 일부만 확인했습니다.」 (the number from `FOLDER_LISTING_CAP`);
   - `folder-listing-skipped` (`args` `{folder, paths}`) — 「알림: 폴더 <폴더>에서 읽을 수 없는 항목을 건너뛰었습니다: <경로>, <경로>, <경로>」, the first three paths named and 「 외 n개」 for the rest;
   - `builtin-unavailable` (`args` `{path, why}`) — 「알림: 기본 제공 <path>을(를) 읽지 못해 빈 내용으로 적용했습니다. 이유: <why>」.
   They follow ADR-0337 decision 3 as ADR-0357 left it: facts only, no intended-behavior sentence, no fix recommended, the caller only. The three new sentences name no backend.
2. **The fallback stays as ADR-0357 decision 3 has it**: each of the six falls back to the generic wording when its `args` do not have the shape its constant documents (not an object, a missing `folder`, `name`, `path` or `why`, a value that is not a string, no `paths` for `codex-same-name-skills` or `folder-listing-skipped`). Every other code takes the generic wording 「알림(<code>): 의도된 동작입니다.」.
3. **No other step-7 notice code exists.** `settings-backend-field-absent` and `item-other-backend` were not added (D2); `working-folder-unchecked` was withdrawn — an unanswered working-folder canonicalize is logged only (B29).

## 거부한 대안
- **The generic wording for the three new codes** (ADR-0357 decision 3 as written) — it says 「의도된 동작입니다」 about a built-in that could not be read or a folder that could not be read, and lists bare values without saying what they are (code: agent `launch.rs` `notice_text`, generic branch).
- **Notices for a settings asset without this backend's field and for an item tagged for another backend** (`settings-backend-field-absent`, `item-other-backend` — drafts before D2) — per-backend fields and rows exist only when the user adds them, so their absence is the normal state, and these notices would multiply with more backends. User 2026-11-11: 「음? codedx칸이 없을수도 있지 왜 알림을해?」 and 「지금 노출 안된상태에서 Add 하면 보여주는 걸로 해쌎ㄶ아. 왜냐하면 모델이 너무 많아질수 있으니.」 약함 — 사용자 직접 기각.
- **A `working-folder-unchecked` notice when the working-folder canonicalize does not answer** (drafts 9–11, with its own fallback for malformed `args`) — it told the caller nothing actionable; its one consequence (a resumed claude session may miss an earlier conversation when the cwd spelling differs) is claude-only and rare. User 2026-11-12: 「뭔지 이해안된다. 나 자야되니 A로 너가 해」, taking main's recommendation to log only. 약함 — 사용자 직접 기각.

## 근거
- Wording: `folder-listing-partial` — main (child TRD B22, "wording = main's call (as U3)"); `folder-listing-skipped` — main with the user's skip-and-tell decision (ADR-0363; P6 `72effa1d`); `builtin-unavailable` — main with the user's run-and-tell decision (ADR-0361; P8 `f42d8538`).
- User decisions D2 (2026-11-11) and B29 (2026-11-12) — child TRD §10.
- Code: agent `launch.rs` `own_wording` (six arms), `notice_text` (the generic branch and the fallback), the constants `FOLDER_LISTING_PARTIAL` · `FOLDER_LISTING_SKIPPED` · `BUILTIN_UNAVAILABLE` and their wording and fallback tests.

## 영향 / 불변식
- Wording arms: agent `launch.rs` `own_wording` — `CODEX_TERMINAL_OMITTED` · `CODEX_TERMINAL_FOLDER_OMITTED` · `CODEX_SAME_NAME_SKILLS` · `FOLDER_LISTING_PARTIAL` · `FOLDER_LISTING_SKIPPED` · `BUILTIN_UNAVAILABLE` — and the generic branch of `notice_text`. A wording arm must not add an intended-behavior sentence or a fix.
- A new notice code that reports a failure, or whose values need naming, gets its own arm; otherwise it says 「의도된 동작입니다」. A malformed-`args` fallback of a failure code still says it — accepted, as in ADR-0357.
- Changing the generic wording is still a change to ADR-0337 decision 3 as it applies to the other codes, and needs its own decision (ADR-0357 「영향」).
- Values in the six sentences come from outside input (folder and file names) and are escaped (`escape_controls`), so a notice stays one line.
