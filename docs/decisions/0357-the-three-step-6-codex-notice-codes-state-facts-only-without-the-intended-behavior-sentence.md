# ADR-0357: The three step-6 codex notice codes state facts only, without the intended-behavior sentence

- 상태: 확정 (2026-11-10, 근거: user decision U3 2026-11-09 (「그렇게 하던지」 after rewording) — TRD `docs/process/S25-preset-library/trd-codex-translation.md` §4-9 · §9-3 U3 · `(으)로` user 2026-11-10 · landed `a3891273` · `abee6ebe`) · 부분 폐기 by ADR-0364 (decision 3 and the 영향 wording arms, the codes with their own wording)
- 관련: Amends ADR-0337 (Decision 3, the intended-behavior sentence) · ADR-0341 (folder entries — `codex-terminal-folder-omitted`) · ADR-0355 (same-name codex skills — `codex-same-name-skills`) · PRD `docs/process/S25-preset-library/prd.md` R37 addendum · R57 · TRD `trd-codex-translation.md` §8-1 (the open amendment this ADR closes) · Amended by ADR-0364 (decision 3 and the 영향 wording arms, the codes with their own wording)

## 맥락
ADR-0337 decision 3 says the codex-terminal notice wording "is facts only · it states that this is intended behavior and does not recommend a fix", with the final wording to be shown to the user at implementation (G28). TRD step 6 draft 1 (`21bc0063`, §9-3 U3) proposed wording that followed it, with an intended-behavior marker: 「알림: codex 터미널 모드는 프리셋 지시서와 스킬을 싣지 않습니다(의도된 동작). 빠진 것 — 지시서 <이름> · 스킬 <이름>」 and 「알림: codex 터미널 모드는 폴더 항목을 읽지 않습니다(의도된 동작). 빠진 것 — <폴더>: <파일 · 스킬 폴더>」. The user reworded it and settled the wording of the three step-6 codes (U3, 2026-11-09); that wording states facts only and has no intended-behavior sentence (PRD R37: it "no longer says intended behavior"). It landed in `a3891273` (step 6 P2, "three notice codes with the user's wording"); `abee6ebe` changed the same-name particle to `(으)로`.

The generic wording in `crates/engram-dashboard-agent/src/launch.rs` `notice_text` still says 「의도된 동작입니다」, and the three codes fall back to it when their `args` are malformed. So ADR-0337 decision 3's intended-behavior sentence now holds for some notices and not for others. TRD §8-1 left the ADR-0337 amendment open; this ADR records where the sentence stops holding.

## 결정
1. The three notice codes `codex-terminal-omitted`, `codex-terminal-folder-omitted` and `codex-same-name-skills` state facts only. Their wording does not say that the omission is intended behavior. The exact wording is TRD §4-9 (PRD R37 addendum for the two terminal codes · PRD R57 for the same-name code):
   - `codex-terminal-omitted` — 「알림: codex 터미널 모드는 프리셋 지시서와 스킬을 지원하지 않습니다. 적용되지 않은 항목: 지시서 <이름>, 스킬 <이름>」 (an empty list leaves its part out; with both empty only the first sentence remains).
   - `codex-terminal-folder-omitted` — 「알림: codex 터미널 모드는 폴더 항목을 지원하지 않습니다. 적용되지 않은 폴더: <폴더> (<파일, 스킬 폴더>)」 (an empty `items` list ends at the folder, with no parentheses).
   - `codex-same-name-skills` — 「알림: 같은 이름의 스킬이 여러 곳에 있습니다. codex에서 $<이름>(으)로 부르면 어느 쪽도 선택되지 않습니다. <이름>: <경로>, <경로>」. This is a codex JSON notice (ADR-0355), not a terminal one; it follows the same facts-only rule.
2. Everything else in ADR-0337 decision 3 stays: facts only, no fix recommended (G28), the dropped items named, the notice only to the caller, and nothing sent to the started agent.
3. Every other notice code takes the generic wording 「알림(<code>): 의도된 동작입니다.」 (with 「 대상 — <values>」 when `args` lists values), and so do the three codes above when their `args` do not have the shape their constant documents (not an object, a missing `folder` or `name`, a list that is not of strings, a same-name notice with no paths). For those, ADR-0337 decision 3's intended-behavior sentence still applies unchanged.

## 거부한 대안
- **Keep the intended-behavior marker in the three codes** (TRD draft 1 `21bc0063` §9-3 U3 — 「…싣지 않습니다(의도된 동작). 빠진 것 — …」, written to ADR-0337 decision 3) — the user was shown this wording and reworded it; the reworded set was decided (U3, 2026-11-09, 「그렇게 하던지」) and PRD R37 records that it replaces the draft and no longer says "intended behavior". The record gives no reason for dropping the marker beyond the user's decision (the user's style rule against translationese — no 「싣다 / 실리다」 — explains the verb change, not the marker). 약함 — 사용자 직접 기각.

## 근거
- User decision U3 (2026-11-09) — TRD `trd-codex-translation.md` §9-3 U3 and §4-9 (exact wording); the `(으)로` particle — user 2026-11-10 (TRD "As landed", commit `abee6ebe`).
- PRD `prd.md` R37 (the final wording "no longer says intended behavior") · R37 addendum · R57.
- Code: `crates/engram-dashboard-agent/src/launch.rs` `own_wording` gives the three codes their own sentence; `notice_text` falls back to the generic 「의도된 동작입니다」 wording for every other code and for malformed `args`. The golden tests in `launch.rs` pin both (the three exact wordings, and the generic wording for other codes and for the fallbacks).

## 영향 / 불변식
- Wording arms: `crates/engram-dashboard-agent/src/launch.rs` `own_wording` (`CODEX_TERMINAL_OMITTED` · `CODEX_TERMINAL_FOLDER_OMITTED` · `CODEX_SAME_NAME_SKILLS`) and the generic branch of `notice_text`. A new wording arm for one of the three codes must not add an intended-behavior sentence or a fix.
- The generic wording keeps saying 「의도된 동작입니다」; a malformed-`args` fallback of the three codes therefore still says it. Changing the generic wording is a change to ADR-0337 decision 3 as it applies to the other codes, and needs its own decision.
- The three codes are produced by step 7 (`check` — TRD §8-1 "Step 7"); this ADR fixes only their wording.
