# ADR-0072: 에이전트 트리 = 계층 구조(백엔드 parent_id + ReparentProfile) + react-arborist 부활 — 1단 중첩·부모삭제=루트승격·상태 글리프

- 상태: 확정 (2026-07-12, 근거: 사용자 인터뷰 결정 — 구현 후속)
- 관련: CLAUDE.md §5(LLM 제어) · ADR-0014(오케스트레이션 — 이 계층이 시각·데이터 기반) · ADR-0018(reserved = 프론트 머지) · ADR-0062(상태 글리프) · ADR-0070(display_name override — 동형 additive 패턴) · ADR-0071(persistence 락) · `crates/engram-dashboard-core/src/agent/profile.rs`(`AgentProfile`) · `src/components/agent/mergeTreeNodes.ts` · `src/components/agent/AgentList.tsx`

## 맥락

에이전트 트리(`AgentList`)는 이름과 달리 **평면 목록**이다 — MVP에 계층이 없어 `react-arborist`를 빼고 flat list로 갔다(`AgentList.tsx` 헤더 주석). 사용자의 다음 목표는 오케스트레이션이다: **A가 메인, B·C·D가 그 하위**, A가 B에 메시지를 쏘고 A에 보고하는 구성. 그 시각·데이터 기반으로 트리에 부모-자식 계층이 필요하다. "일단 겉으로만" 보여주더라도 재시작 후 배치가 흩어지면 안 되고(사용자), LLM이 트리 구성을 제어할 수 있어야 한다(§5).

## 결정

1. **부모-자식 = 백엔드 저장.** `AgentProfile.parent_id: Option<AgentId>`(`#[serde(default)]`) 신설 + `ReparentProfile { child_id, parent_id: Option<AgentId> }` command(rename 슬라이스 ADR-0070과 동형 additive 패턴). §5로 LLM/사용자가 같은 command로 부모를 지정. protocol/bindings 미러.
2. **렌더 = react-arborist 부활**(이미 설치된 의존성 `^3.9.0`). 들여쓰기·접기/펼치기·드래그 재부모화. `mergeTreeNodes`가 flat concat 대신 parent_id로 자식을 부모 밑에 묶어 `children` 트리를 반환.
3. **중첩 깊이 = 1단**(A > B·C·D). 자식은 다시 부모가 될 수 없다(cycle 처리 단순).
4. **부모 삭제 = 자식 루트 승격**(orphan-to-root). A 삭제 시 B·C·D의 parent_id를 None으로 풀어 최상위로. cascade 삭제 아님(데이터 보존).
5. **표시 = 계층 주축 + 상태 글리프.** 실행/저장 구분은 섹션이 아니라 노드 글리프로(● running / ○ reserved / ◐ spawning) — 계층이 조직 축이라 섹션과 충돌 방지. **Spawning = 프론트 합성**(백엔드에 별도 Spawning 상태 없음 — spawn 요청~첫 Running 사이 프론트가 표시).

## 거부한 대안

- **프론트 전용 cosmetic parent 맵** — §5 위반(LLM이 트리 구성 제어 불가)·비영속(재시작 시 배치 흩어짐)·오케스트레이션 라우팅 붙일 때 버려짐. 사용자가 "껏다 켜면 흩어지면 안 됨"으로 백엔드 저장 선택.
- **평면 목록 유지 + indent만** — 접기/펼치기·드래그 재부모화 없음. `react-arborist`가 이미 의존성인데 트리 기능을 수동 재구현하는 셈.
- **다단(임의 깊이)** — cycle 방지·재부모화 제약이 복잡. 현 A>BCD 모델엔 과함 → 1단으로 시작, 필요 시 확장.
- **섹션(실행/저장) 유지 + 섹션 안에서 계층** — 부모와 자식이 다른 섹션(A=running, B=reserved)에 걸리면 분리돼 보여 오케스트레이션 직관이 깨짐. 계층 주축 + 글리프로 대체.
- **부모 삭제 cascade** — 실수로 그룹 전체를 날릴 위험. 루트 승격이 안전.
- **백엔드 새 Spawning 상태 신설** — lifecycle 상태머신(ADR-0016/0019) 변경은 비용·위험 큼. 표시용 과도기라 프론트 합성으로 충분(실측 필요 시 나중 백엔드화).

## 근거

- **§5(손발/두뇌 분리):** 트리 구성(누가 누구 밑)도 LLM이 제어하는 핸들이어야 한다 → 백엔드 `parent_id` + command. 사람 드래그는 같은 핸들의 보조 입력.
- **ADR-0014 기반:** 이 계층이 오케스트레이션(A→B 라우팅·보고)의 시각·데이터 토대. cosmetic으로 짜면 라우팅 붙일 때 버려지므로 처음부터 백엔드.
- **판단기준 §0(저위험+장기 → 지금):** parent_id seam·command는 저위험 additive(ADR-0070 rename과 동형 검증됨)이고 장기(오케스트레이션)에 필요 → 지금 깐다.
- **react-arborist 재활용:** 뺐던 의존성을 되살리는 것이라 신규 도입 아님.

## 영향 / 불변식

- `AgentProfile.parent_id` `#[serde(default)]` 유지 — 옛 `agents.json`(필드 없음) → None 흡수(무마이그레이션, ADR-0070과 동일 규율).
- **`ReparentProfile` cycle 방지:** 1단이라 "자식을 부모로" 금지 검증 필수(child가 이미 누군가의 부모면 거부, self-parent 거부, 존재 안 하는 parent 거부). persistence는 ADR-0071 락 규율 경유(`ProfileRegistry` mutate).
- **부모 삭제 시 자식 parent_id=None 승격** — deleteProfile이 자식들을 훑어 승격(한 임계구역). 고아 참조(존재 안 하는 parent_id) 금지.
- **`mergeTreeNodes`는 ADR-0018 머지(running ∪ reserved)를 계층 안에서 유지** — 부모·자식 각각 running/reserved일 수 있고 글리프로 구분.
- react-arborist idAccessor=id · childrenAccessor=children. 구독·상태 계약(ADR-0046 등)은 무변경(표시 계층만 추가).
- Spawning 글리프는 프론트 파생 — 백엔드 상태머신 불변(ADR-0016/0019).
