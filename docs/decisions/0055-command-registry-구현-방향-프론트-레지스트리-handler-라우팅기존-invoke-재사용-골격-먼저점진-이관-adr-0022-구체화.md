# ADR-0055: command registry 구현 방향 — 프론트 레지스트리 + handler 라우팅(기존 invoke 재사용), 골격 먼저·점진 이관 (ADR-0022 구체화)

- 상태: 확정 (2026-07-09, 근거: 사용자와 설계 합의 — 이 세션 대화에서 forks 해소)
- 관련: ADR-0022(통합 command registry 방향 — 이 ADR이 그 미해결 forks를 구체화) · CLAUDE.md §5(LLM-우선 제어) · ADR-0020(단일 dispatch) · ADR-0011(agentClient 제어 표면) · `docs/process/B-wezterm-tabs/PRD.md`(탭 = 첫 어댑터) · step-log 2026-07-09

## 맥락
ADR-0022가 "모든 동작 = id로 등록된 command, palette·키바인딩·LLM·메뉴는 소비자(파생)" 방향만 고정하고 구현 forks(레지스트리 위치·백엔드 미러·키바인딩 저장·when-context·enum vs registry)를 **미해결**로 남겼다. 이제 구현 착수 — 최소 골격을 확정해야 한다. 사용자 방침: **"전부 한꺼번에 커맨드로 갈아엎지 말고, 시스템(골격)을 갖추고 하나씩 이관."**

## 결정
- **command = `{ id, title, category?, keybinding?, when?, run(args) }`** 를 **프론트 Map 레지스트리**에 등록(`register`/`run(id,args)`/`list`).
- **실행 = handler가 기존 진입점으로 라우팅한다** — 백엔드 동작은 `invoke('cmd', args)`(기존 타입 dispatch → `ViewManager`/`AgentManager` 싱글톤), 순수 프론트 동작은 store 호출. **새 싱글톤·새 arg 파싱 로직을 만들지 않는다**(상태 권위는 기존 그대로, ADR-0035 유지).
- **소비자 = 전부 `run(id, args)` 하나로 통일:** 사람 클릭 · 전역 `keydown`(★포커스 가드 필수 — `input`/`textarea`/`.xterm` 타이핑 중엔 단축키 가로채기 금지★) · `window.__engramCmd.{list, run}`(LLM/cdp §5 진입점). 팔레트는 `list()`를 먹는 별도 UI로 **후속**.
- **골격 먼저 + 점진 이관:** 기존 ad-hoc 표면(`window.__engramLayout`·agent `invoke`)은 **그대로 두고**, **새 기능부터** 레지스트리에 등록한다. big-bang 전체 이관 금지.
- **인자 = 객체 하나(가방)**; 가변인자 안 씀. 각 handler가 필요한 키만 destructure. TS 타입 안전은 `id → 인자타입` 룩업으로 호출부 컴파일 체크(옵션, enum-exhaustive 안전성을 여기서 얻음).
- **레지스트리 위치 = 프론트 단독**(LLM은 cdp로 `__engramCmd.list()` introspect). 백엔드 미러 = 후속.

## 거부한 대안
- **enum-데이터 단일 커맨드(variant + 중앙 dispatch):** 타입·exhaustive는 강하나 **새 커맨드마다 중앙 enum + match + 메타맵을 수정** → ADR-0022의 "추가 여파 0 · 기능이 자기 command를 스스로 등록(탈중앙)" 목표에 위배. (백엔드 `AgentCommand` enum은 **wire 계약이자 실행 계층**이라 그대로 유지 — 이 ADR의 프론트 레지스트리와 별개 층으로 공존.)
- **백엔드 미러 레지스트리(지금):** LLM introspection엔 이상적이나 현재 cdp `list()`로 충분 → 후속(브리지·중복 비용 회피).
- **big-bang 전체 이관:** 기존 표면을 즉시 전부 command화 = 위험·큼 → 점진(사용자 방침).
- **커스텀 키맵 저장(localStorage) · when-context 풀 모델 · 팔레트 UI:** 골격 밖 → 후속(최소부터 쌓고 레이어로 얹음).

## 근거
- ADR-0022의 VS Code 검증 모델(registry → palette/keybinding/menu 파생). 10년+ 확장성 패턴.
- **기존 invoke/`AgentCommand` 타입 dispatch를 실행 계층으로 재사용** → 신규 싱글톤 0, 레지스트리는 얇은 발견/라우팅/메타 계층.
- hot path(에이전트 출력)는 binary raw 통과라, cold path인 command에 JSON을 써도 비용 무시가능(사람 손 속도).
- 골격이 작고 저위험(순수 프론트 additive)이라 seam을 지금 까는 게 유리(CLAUDE.md §0 — 저위험+장기 → over-engineering 허용).

## 영향 / 불변식
- **새 UI/레이아웃 기능은 command로 등록한다**(§5 — LLM·키바인딩·팔레트가 자동 흡수). **탭(PRD B-tabs, Phase 2)이 첫 어댑터** — `tab.switch`/`tab.create`/`tab.close` 등.
- **레지스트리는 상태 권위가 아니다** — 발견/라우팅/메타만. 실행은 기존 invoke → 싱글톤(ADR-0035 레이아웃 권위 유지).
- **키바인딩 포커스 가드 불변식:** 안 하면 터미널/입력창 타이핑을 단축키가 가로채는 회귀(load-bearing).
- ADR-0022의 supersede 아님 — 그 방향의 **구체화(Amends)**. 후속 확장(백엔드 미러·팔레트·커스텀 키맵·when)이 이 골격 위에 레이어로 얹힌다.
