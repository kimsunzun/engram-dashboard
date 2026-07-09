# ADR-0060: 슬롯 콘텐츠 모델 = 타입드 유니온(SlotContent enum) — view-type 레지스트리(P2)·URI(P3) 거부

- 상태: 확정 (2026-07-09, 근거: /research medium 설계-결정 모드 OSS 서베이 + Codex cross-family 적대 리뷰 FIX 반영 + 사용자 결정)
- 관련: CLAUDE.md §5(LLM 제어)·§0(저위험·장기 seam) · ADR-0035(레이아웃 권위=src-tauri) · ADR-0057(탭 소유) · ADR-0041/0042/0046(출력 라우팅) · ADR-0002/0030(capability 합성) · `src-tauri/src/layout/types.rs`(LayoutNode::Slot) · TRD `docs/process/C-slot-content/TRD.md` · step-log 2026-07-09

## 맥락
레이아웃 트리의 leaf 슬롯(`LayoutNode::Slot { id, agent_id: Option<Uuid> }`)이 **에이전트 하나만** 담도록 굳어 있다. 사용자가 슬롯에 이질적 콘텐츠(라이브 에이전트 + 파일/엔티티 트리 + 커스텀 제어 버튼셋 등)를 담길 원하는데, 현재 모델엔 "슬롯 점유자의 종류"라는 개념 자체가 없다(agent_id의 Some/None = 점유/비점유뿐). 굵은 설계라 착수 전 OSS가 "칸 안 콘텐츠 종류"를 어떻게 모델링하는지 서베이했다(VS Code·JupyterLab/Lumino·Theia·Obsidian·Tabby·tmux·Zellij).

## 결정
슬롯 점유자를 **타입드 유니온 `SlotContent`**(Rust enum, `#[serde(tag="type")]` 내부태깅)로 모델링한다 — 기존 `LayoutNode` 태깅 규약과 동일. `Agent` variant가 지금의 `agent_id`를 흡수하고, 비-에이전트 콘텐츠는 별도 variant로 추가된다. ts-rs가 이 enum을 TypeScript discriminated union으로 자동 생성 → 프론트(`ViewLayoutRenderer`, 순수 렌더러)가 `switch(content.type)`로 타입안전 분기한다.

```rust
#[serde(tag = "type", rename_all = "snake_case")]
enum SlotContent {
    Empty,
    Agent { agent_id: String },   // 스트림은 OutputRouter가 별도 라우팅(여기엔 바인딩만)
    // 후속: FileTree { .. }, ControlPanel { config } — variant 추가로 확장
}
```

**핵심 통찰(enum 폭발 방지):** 사용자 커스텀(프리셋 버튼셋)은 **새 variant가 아니라 `ControlPanel` variant 내부의 config 데이터**로 표현한다(data-driven). 버튼셋을 수백 개 만들어도 Rust variant·프론트 switch case는 늘지 않는다. 새 케이스는 "구조적으로 완전히 다른 종류"가 생길 때만.

## 거부한 대안
- **P2 — view-type 레지스트리(`{ viewType: string, state: opaque }` + 팩토리 레지스트리).** 대형 GUI 앱의 지배적 관행(VS Code EditorInput typeId·Obsidian registerView·Theia factoryId·JupyterLab command+args). **거부 이유:** 그 관행의 전제 = **플러그인 생태계**(서드파티가 콘텐츠 종류를 등록 → 코어가 타입을 몰라야 함 → 불투명 state로 타입안전 포기). engram은 콘텐츠 종류를 **백엔드가 전부 통제**하고(플러그인 확장 없음) Rust+ts-rs로 **end-to-end 타입안전**을 얻는 게 설계 목표(§5·ADR-0035)다. 전제가 다르니 관행(P2)이 아니라 소수파(P1)가 제약에 맞는다. 필요해지면 P1 안에 `Custom { schema_id, state: JsonValue }` 탈출구를 나중에 얹을 수 있다(지금은 YAGNI).
- **P3 — URI + scheme resolver(`agent://…`, scheme이 핸들러 선택).** VS Code `EditorResolverService`의 파일중심 모델. **거부 이유:** URI 없는 순수 뷰(제어 패널·트리)에 안 맞고, 문자열 파싱이라 런타임까지 타입 오류가 안 잡힌다 — Rust 권위·타입안전 이점 상실. (LLM 명령의 외부 표기로는 부분 유용할 수 있으나 내부 표현으로는 부적합.)

## 근거
- **Zellij(Rust production 선례):** pane 콘텐츠 종류를 `Run` enum(`Plugin | Command | EditFile | Cwd`)으로 표현하고, `PaneLayoutManifest.run: Option<Run>`이 세션 직렬화에도 discriminator를 보존한다(`zellij-utils/src/input/layout.rs`, `session_serialization.rs`). 단 Zellij `Run`은 "실행 지시" enum이라 범용 이질 콘텐츠 모델을 **약하게만** 지지(과대 유추 주의 — Codex 지적).
- **태깅 정합 실측:** 현 `LayoutNode`가 이미 `#[serde(tag="type", rename_all="snake_case")]` 내부태깅(`types.rs:26`) → `SlotContent` 내부태깅이 완전 일관, ts-rs가 동일하게 discriminated union 생성. (Codex의 ts-rs 태깅 함정 지적 해소 — tuple variant는 내부태깅 불가하나 우린 struct/unit variant만 씀.)
- **영속화 부재:** 레이아웃 트리는 현재 디스크 영속화가 없다(메모리 전용, 재시작 시 `new()`). 따라서 이 리팩터는 serde 스키마 마이그레이션을 요구하지 않는다 — Codex의 버전관리 우려는 **영속화 도입 시점의 요건**으로 이연(아래 불변식).
- **관행 대비 판단:** 관행(P2, 플러그인 앱)의 전제가 우리와 다름을 명시적으로 확인 후 소수파(P1) 채택 — "관행 무시"가 아니라 "전제 불일치".

## 영향 / 불변식
- **콘텐츠 종류와 출력 라우팅은 대체로 직교하되 완전 직교는 아니다(Codex 지적):** `OutputRouter`의 바이트 라우팅은 `SlotContent::Agent { agent_id }`에서 agent_id만 뽑으면 되므로 직교(ADR-0041/0042/0046 불변). **그러나 수명·의미는 콘텐츠 종류가 좌우**한다 — 슬롯 닫기 vs 에이전트 kill/detach, 재배정, 렌더 디폴트(Agent→xterm/rich·비-에이전트→dom), 타이틀/포커스. "완전 직교" 주장은 폐기. 라이브 스트림은 `SlotContent`에 담지 않고 바인딩(agent_id)만 담는다(epoch는 layout 트리 밖, agentStore 소유 — ADR-0007/0046).
- **capability 합성(후속 요건):** closable/killable/serializable/commandable 등 variant 횡단 동작은 exhaustive match로 흩지 말고 **capability 표로 국소화**한다(engram 기존 backend capability 매트릭스 ADR-0002/0030과 idiom 일치). 이번 seam 슬라이스 범위 밖 — variant가 늘 때 도입.
- **영속화 도입 시 요건(deferred):** 레이아웃을 디스크 영속화할 때는 `SlotContent`에 **version 봉투 + `Unknown` variant(구버전 forward-compat) + migration 패스**를 함께 넣는다(Theia 선례). 지금은 영속화가 없어 미적용.
- **중첩 레이아웃 여지(미결):** 미래 "대시보드형" 콘텐츠가 자체 탭/분할을 원하면 `SlotContent`가 leaf-only인지 compositional인지 재검토 필요 — 이번엔 leaf-only(Agent/Empty).
- **불변 유지:** `resolve_spawn_slot` 점유 판정(ADR-0059)은 `SlotContent::Empty` = 빈 / `Agent{..}` = 점유로 이관. `assign_agent`/`assign_in_tree` 덮어쓰기 시맨틱(ADR-0058 관련 불변)은 유지 — 점유 방어는 resolve 층.
