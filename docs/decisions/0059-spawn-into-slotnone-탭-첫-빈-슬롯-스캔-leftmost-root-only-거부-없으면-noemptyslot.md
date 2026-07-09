# ADR-0059: spawn_into slot=None = 탭 첫 빈 슬롯 스캔 (leftmost-root-only 거부 — 없으면 NoEmptySlot)

- 상태: 확정 (2026-07-09, 근거: 스테이지 5 커밋 88134e2 + /review code full 폐쇄 PASS)
- 관련: TRD `docs/process/B-wezterm-tabs/TRD.md` §6 G9(spawn_into 슬롯 정책) · `src-tauri/src/layout/manager.rs:498-532`(resolve_spawn_slot·SpawnSlotError) · `src-tauri/src/layout/tree.rs:52`(first_empty_slot_id) · ADR-0057(탭 소유 모델) · step-log Phase 2 스테이지 5

## 맥락
`spawn_into`에서 `slot`을 지정하지 않으면 대상 탭의 어느 슬롯에 에이전트를 배정할지 정해야 한다. TRD §6 G9는 "대상 탭의 **빈 root 슬롯**"이라 적었는데, 이 문구가 두 해석으로 갈린다: (a) root(최상위/좌측) 슬롯 한 칸만 보고 그게 점유면 실패 — 새로 만든 탭(빈 슬롯 1개)엔 맞지만 split된 탭에선 좌측이 차 있으면 우측이 비어도 실패, (b) 트리를 훑어 첫 번째 빈 슬롯을 찾음. G9는 코더 추측을 막으려 넣은 항목인데 이 지점이 여전히 모호했다.

## 결정
`slot=None`은 **트리를 전위(pre-order, 좌-우선) 순회해 첫 번째 빈 슬롯**을 고른다(신규 `tree::first_empty_slot_id`). 빈 슬롯이 하나도 없으면 `SpawnSlotError::NoEmptySlot`으로 실패한다 — **자동 split이나 덮어쓰기를 하지 않는다**. `slot=Some(s)`는 별개: `s`가 트리에 없으면 `SlotNotFound`, 비어 있으면 `s`, 점유면 `SlotOccupied`(자동 split/replace 안 함). 이 해석으로 TRD §6 G9의 "빈 root 슬롯" 문구를 확정한다.

## 거부한 대안
- **leftmost-root-only (a) — root/좌측 슬롯 한 칸만 검사.** split된 탭에서 좌측이 점유면 우측이 비어 있어도 `SlotOccupied`로 실패한다 — 사용자·LLM이 "빈 자리가 있는데 왜 실패하냐"로 혼란한다. 배치 지정 스폰의 의도(빈 자리에 얹기)와 어긋나 거부.
- **빈 슬롯 없을 때 자동 split/덮어쓰기.** 호출자가 명시하지 않은 레이아웃 변경(분할)이나 기존 에이전트 파괴(덮어쓰기)를 조용히 일으킨다 — 되돌리기 어렵고 의도 밖. `NoEmptySlot`로 fail-loud해 호출자가 split을 명시하게 함.

## 근거
- `/review code full` 2-family 적대 리뷰 1R에서 슬롯 정책 모호·테스트 부재 지적 → `first_empty_slot_id` 도입 + 단위 테스트(single-empty/single-occupied/skips-occupied-leftmost/all-occupied) 추가 후 폐쇄 PASS.
- throwaway verbatim-mount 하네스에서 `resolve_spawn_slot`/`first_empty_slot_id` 경로 테스트 통과(스테이지 5 qa).
- 사용자 결정: 이번 세션 대화에서 "2-b"(첫 빈 슬롯 스캔) 확정.

## 영향 / 불변식
- `resolve_spawn_slot`(순수, `manager.rs:498-532`)이 slot 정책의 단일 출처다 — spawn_into는 배정 전 이걸로 점유 검사를 자체 수행한다.
- **`assign_agent`/`tree::assign_in_tree`의 덮어쓰기 시맨틱을 바꾸지 않는다** — `move_slot_to_window` 등이 의존한다. 점유 방어는 resolve_spawn_slot 층에서만.
- `NoEmptySlot`/`SlotOccupied`는 스폰이 **이미 성공한 뒤** 배정 단계에서 나므로, command는 이 에러에서 에이전트를 kill하지 않고 생존시킨 채 보고한다(`list_agents`로 재부착 가능 — layout.rs `alive_err`).
