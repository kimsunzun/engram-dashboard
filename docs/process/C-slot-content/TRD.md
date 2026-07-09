# TRD: 슬롯 콘텐츠 seam — `Slot.agent_id` → `Slot.content: SlotContent`

> 결정 정본 = **ADR-0060**(타입드 유니온 채택, P2/P3 거부). 이 문서는 *어떻게*(구현 기계)만 담는다.
> 이번 슬라이스 = **seam만**(behavior-identical). 실제 비-에이전트 콘텐츠(Tree/ControlPanel) UI는 **follow-up**(UX = 사용자 결정).

## 1. 스코프
- **In:** `LayoutNode::Slot`의 `agent_id: Option<String>`을 `content: SlotContent`(`Empty | Agent { agent_id: String }`)로 교체. 백엔드·wire·프론트 렌더 분기까지 end-to-end. 동작은 **완전 동일**(빈=Empty, 배정=Agent).
- **Out(follow-up):** 실제 `FileTree`/`ControlPanel` variant + 렌더러 + 배치 command, capability 표, 영속화 시 version/Unknown/migration. (ADR-0060 "영향/불변식"에 요건 박제.)

## 2. 변경 지점 (Explore 맵 기반 — file:line은 착수 시 재확인)

### 백엔드 (필수)
- **`src-tauri/src/layout/types.rs`** — (a) `SlotContent` enum 신규(`#[serde(tag="type", rename_all="snake_case")]`, `#[ts(export)]`, `Empty` + `Agent { agent_id: String }`) + `//! `/`///` 불변식 주석 + `// ADR-0060` 앵커. (b) `LayoutNode::Slot { id, agent_id }` → `{ id, content: SlotContent }`. (c) `new_empty_slot()` → `content: SlotContent::Empty`. 편의 메서드 `SlotContent::is_empty()` / `agent_id() -> Option<&str>` 권장(호출부 단순화).
- **`src-tauri/src/layout/tree.rs`** — `find_slot`(반환 `Option<&SlotContent>`), `first_empty_slot_id`(`matches!(content, SlotContent::Empty)`), `assign_in_tree`(파라미터 `SlotContent` 또는 assign/clear 분리), 구조분해 패턴 2곳(`{ id, content }`).
- **`src-tauri/src/layout/manager.rs`** — `slot_agent`(SlotContent→Option<String> 유도), `assign_agent`(`SlotContent::Agent{agent_id}` 생성), `resolve_spawn_slot`(3-way: `Some(Empty)=Ok` / `Some(Agent{..})=SlotOccupied` / `None=SlotNotFound` — ADR-0059 불변 유지), `prepare_detached_view`, test 패턴 2곳(`content: SlotContent::Empty`).
- **`src-tauri/src/output_router.rs`** — `collect_agents`: `LayoutNode::Slot { content, .. }` 구조분해 + `if let SlotContent::Agent { agent_id } = content { agent_id.parse::<AgentKey>() … }`, `Empty` 명시 무시. (ADR-0041/0042/0046 라우팅 불변 — 배정 슬롯만 수신.)
- **`src-tauri/src/commands/layout.rs`** — `spawn_into`/`assign_agent` command: **IPC wire 파라미터 `agent_id: String` 유지**(프론트 계약 불변), 내부에서 manager가 `SlotContent::Agent` 조립. 최소 변경.

### Wire (ts-rs — 자동)
- `src-tauri/bindings/LayoutNode.ts` + 신규 `SlotContent.ts`는 **ts-rs 재생성으로 자동 갱신**(수동 편집 금지). `#[ts(export)]` 필수. 재생성 = 바인딩 export 테스트/빌드.

### 프론트 (필수 1파일)
- **`src/components/layout/ViewLayoutRenderer.tsx`** — `node.agent_id != null` 4곳을 `node.content.type === 'agent'`로 전환(권장: `switch(node.content.type)` 형태). agent lookup = `node.content.type === 'agent' ? node.content.agent_id : null`. DomSlot/RichSlot/TerminalSlot·SlotContextMenu에 넘기는 `agentId` prop은 분기 안에서 non-null.
- **무변경 확인:** `viewStore.ts`(agent_id 직접 미참조), `layoutTypes.ts`(re-export), `TerminalSlot.tsx`(prop 수신), `renderMode.ts`, `AgentTree.tsx`(슬롯 콘텐츠 모델 밖 — 고정 사이드패널).

## 3. 테스트 (TDD — 누적)
- **신규:** `SlotContent` serde round-trip(`{"type":"empty"}` / `{"type":"agent","agent_id":"…"}`) golden · `is_empty`/`agent_id()` · ts-rs export shape 단언(가능하면).
- **갱신:** 기존 layout 단위/throwaway verbatim-mount 하네스(resolve_spawn_slot 3-way·first_empty_slot·assign·collect_agents 라우팅)를 새 타입으로 — **동작 불변 회귀**가 목표.
- **격리:** src-tauri 순수 로직은 기존 throwaway verbatim-mount 방식 유지(0xc0000139 회피 — 핸드오프 do-not 참조).

## 4. 수용 기준
1. `LayoutNode::Slot`이 `content: SlotContent`를 갖고, `Empty`/`Agent` round-trip이 내부태깅 JSON으로 정확.
2. 라우팅(collect_agents)·spawn_into·resolve_spawn_slot·assign이 **동작 동일**(회귀 테스트 통과).
3. ts-rs 바인딩 재생성 → 프론트 `switch(content.type)` 타입체크 통과.
4. GUI 실측: spawn_into → 슬롯에 에이전트 Running(=Agent variant 경유), 빈 슬롯=Empty. 기존 탭/분할 동작 무회귀.
5. 코어 격리(`rg "use tauri" core` 0), fmt, 전 회귀(cargo member-scoped + vitest + tsc) green.

## 5. 리스크
- **load-bearing 라우팅**(`collect_agents`) 오변환 시 출력 오라우팅 → 회귀 테스트로 방어, /review code에서 2-family 검증.
- ts-rs 재생성 누락 시 wire drift → 빌드/바인딩 게이트.
- 전체 `cargo test`는 0xc0000139(환경) — member-scoped + throwaway-mount 우회(핸드오프 do-not).
