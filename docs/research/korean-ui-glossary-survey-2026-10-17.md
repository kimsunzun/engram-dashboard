# ★ Korean wording for the library screen — glossary cross-checked against the Korean editions of Unreal · Unity · Windows — light research

| Item | Value |
|---|---|
| Status | Research complete · applied 2026-10-17 — convention first (user) · a second round of research moved our own words to peer words too: 지시서→지침 (instruction document → guideline) · 쓰는 곳→참조 (where used → reference) · 묶음→컬렉션 (bundle → collection) · 내장→기본 제공 (built-in → provided by default) · 쉼→사용할 수 없음 (resting → unavailable) · 다시 잇기→다시 연결 (re-link → reconnect) · 미리보기→미리 보기 (preview, spacing fix) · 담기→추가 (put in → add) (basis = the VS Code Korean language pack vscode-loc main.i18n.json — 지침 87 · 기본 제공 71 · 미리 보기 87 · 다시 연결 20 · 참조 130 · 사용 안 함 84 occurrences) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (fetch · about 22 searches · 5 official Korean doc pages) · 1 main grounding spot check (the Unreal 「필터와 컬렉션」 Korean doc via WebFetch) · no adversarial review |
| Question | If the on-screen wording of the briefing · PRD (mixed with phrases coined by the session — flagged: 「3개 고름」 · 「항목을 고르면 여기 보입니다」) is chosen to match the words of the Korean editions of Unreal · Unity · Windows |

## Limits (read first)

- The Unreal · Unity side is **Korean translated documentation**, not the editor's on-screen strings — the screen may differ (e.g. the doc says 「이름변경」).
- The Windows empty-screen · status wording (「N개 항목 선택함」 · the empty text of the details pane, etc.) is **from memory** — the source pages could not be fetched. The Microsoft terminology portal could not be reached.

## Principles

1. **Menu · button = a noun or an action noun** (삭제 · 이름 바꾸기 · 새로 만들기 · 새로 고침) — no 「하세요」 form. Only confirmation-dialog buttons use the action itself (휴지통으로 이동 · 취소).
2. **Counts = the Windows form** — 「3개 항목 선택함」 · 「항목 2개」 · 「프리셋 2개」. The per-kind breakdown goes after a middle dot.
3. **Guidance · empty screen = a 「~습니다」 declarative sentence or a short noun phrase** — do not write sentences that tell the user what to do.
4. **Words the peer has are used as the peer has them** — library concepts from Unreal (즐겨찾기 · 필터 · 검색), file operations from Windows (새로 만들기 · 이름 바꾸기 · 삭제 · 새로 고침 · 이동).
5. **One concept = one word, on every screen** — 고름 → 선택 · 지우기 → 삭제 · 옮기기 → 이동.
6. **Even what we thought of as our own words becomes the peer word if a peer word exists** (user 2026-10-17 「관용이 먼저야 글자는」 — reverses 2026-10-16 「컬렉션이라고 하면 뭔 소린지 모른다」) — 지시서 → 지침 · 쓰는 곳 → 참조 · 묶음 → 컬렉션 · 내장 → 기본 제공 · 쉼 → 사용할 수 없음 · 다시 잇기 → 다시 연결 · 다시 이어짐 → 다시 연결됨 · 미리보기 → 미리 보기 · 담기 → 추가. Only concepts for which no peer word really exists (presets, etc.) keep our word, and it is defined just once.

## Glossary (now → change)

| Place | Now | Unreal ko | Unity ko | Windows ko | Change | Basis · confidence |
|---|---|---|---|---|---|---|
| Menu | 지우기 | 삭제 | 삭제 | 삭제 | **삭제** | All three — certain (Unreal checked by main) |
| Menu | 이름 바꾸기 | 이름변경 | 이름 바꾸기 | 이름 바꾸기 | 이름 바꾸기 (unchanged) | Unity · Windows — certain |
| Menu | 새로 만들기 ▸ … | 추가(+) · 새 폴더 | 생성 | 새로 만들기 | 새로 만들기 (unchanged) | Windows — likely |
| Menu | 새로 고침 | — | 새로 고침 (from memory) | 새로 고침 | Unchanged | Windows doc — certain |
| Gesture · menu | 옮기기 · 여기 놓기 | 이동 · 여기로 이동 | — | 이동 · 여기로 이동 | **이동 · 여기로 이동** | Unreal · Windows — likely |
| Filter | 필터 리셋 | 모든 필터 비활성화 · 모든 필터 제거 | — | — | **모든 필터 제거** | Unreal source text (checked by main) — clearing every checkbox back to the start means 「제거」 (remove) fits better · likely |
| Confirmation dialog button | 휴지통으로 | — | — | 휴지통으로 이동 (from memory) | **휴지통으로 이동** | Likely |
| Confirmation dialog title | ‘review’ 지우기 | 삭제 | — | 삭제 | **‘review’ 삭제** | Principle 5 |
| Confirmation dialog text | 프리셋 2개가 씁니다 | — | — | — | **프리셋 2개에서 쓰는 중입니다** | Principle 3 · keeps the word 「쓰는 곳」 — our own word |
| Confirmation dialog text | 휴지통으로 갑니다 | — | — | — | **휴지통으로 이동합니다** | Principle 5 |
| Status | N개 고름 | — | — | N개 항목 선택함 (from memory) | **3개 항목 선택함 · 스킬 2 · 지시서 1** | Principle 2 — likely |
| Empty screen | 항목을 고르면 여기 보입니다 | — | — | (details pane text — from memory · source text not checked) | **선택한 항목이 없습니다** | Principle 3 — uncertain |
| Count | 항목 2 · 프리셋 2 | 에셋 수 | — | 항목 N개 | **항목 2개 · 프리셋 2개** | Principle 2 |
| Search box | 검색 — 스킬 › 리뷰 (하위 폴더까지) | 검색 | 검색 | 검색 | **검색: 스킬 › 리뷰 (하위 폴더 포함)** | Windows 「포함」 — likely |
| Column | 종류 | 유형 (unchecked) | 타입 | 종류 | Unchanged | Windows column name |
| Column · detail | 위치 · 설명 · 쓰는 곳 | — | — | 위치 · 설명 | Unchanged | 쓰는 곳 = our own word |
| Sort | ▲ ▼ | — | — | 정렬 기준 · 오름차순 · 내림차순 | Tooltip 「오름차순 · 내림차순」 | Windows doc — certain |
| Left pane | 즐겨찾기 · ＋ 새 묶음 | 즐겨찾기 · 컬렉션 추가 | 즐겨찾기 | 즐겨찾기 | Unchanged (keeps 묶음 — principle 6) | User decision 2026-10-16 |
| Edit pane | + 담기 · ← 선택한 것 담기 | 추가 | — | — | **Awaiting user confirmation** — whether to align to 「추가 · ← 선택 항목 추가」 | Unreal = 추가 · 「담기」 has no peer |
| Status | 오류 · 경고 · 찾을 수 없음 | — | — | 오류 · 경고 · 찾을 수 없음 | Unchanged | Certain · likely |
| Status | 다시 이어짐 · 다시 잇기 · 쉼 · 내장 | — | — | — | Unchanged (our concepts) | — |
| Kind name | 지시서 · 스킬 · 설정 · 프리셋 · 폴더 | 에셋 (generic term) | 에셋 | — | Unchanged · if a generic term is needed, 「항목」 (the selection count) | — |

## What could not be matched

- Korean labels like 「탐색기에서 표시」 · search-popover placeholder text · tooltip text · the Windows empty-search-result wording — no source text.
- 「어디서 일할까요?」 (the working-folder question) is within the scope where working-folder cleanup was deferred, so it is not handled this time.

## Sources

- https://dev.epicgames.com/documentation/unreal-engine/filters-and-collections-in-unreal-engine?lang=ko (checked by main — filters · 모든 필터 비활성화 · 모든 필터 제거 · 추가 · 이름변경 · 삭제)
- https://dev.epicgames.com/documentation/unreal-engine/working-with-assets-in-unreal-engine?lang=ko
- https://dev.epicgames.com/documentation/ko-kr/unreal-engine/content-browser-interface-in-unreal-engine
- https://docs.unity3d.com/kr/2021.3/Manual/ProjectView.html
- https://support.microsoft.com/ko-kr/accessibility/windows/use-a-screen-reader-to-explore-and-navigate-file-explorer-in-windows
