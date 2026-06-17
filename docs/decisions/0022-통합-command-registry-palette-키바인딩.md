# ADR-0022: 통합 command registry — palette + 키바인딩 + LLM + 메뉴/트레이 단일 출처

- 상태: **제안(Proposed)** (2026-06-17, 방향만 고정 — 구현은 나중. 다음 세션들이 command를 이 형태로 쌓게 하기 위한 기록.)
- 관련: CLAUDE.md §5(LLM-우선 제어, 모든 메뉴 프로그래밍 가능) · ADR-0011(agentClient 제어 표면) · ADR-0020(단일 dispatch) · ADR-0021(데몬 lifecycle command) · 백로그 "data-driven 우클릭 메뉴"
- 범위: 사용자 동작·LLM 제어·키바인딩·메뉴/트레이/palette를 **하나의 command registry**에서 파생시키는 아키텍처 방향. 미래 기능 추가의 blast-radius를 0에 수렴시키는 게 목적.

## 맥락

세 요구가 사실 **같은 한 가지**로 수렴한다:
1. **§5 LLM-우선 제어** — 모든 기능(백엔드 + UI/레이아웃)을 LLM이 호출 가능해야.
2. **VS Code식 Command Palette** — 검색하면 수많은 command가 뜨고 실행.
3. **커스텀 키바인딩** — 모든 동작에 단축키를 사용자가 매핑(keybindings.json식).

셋 다 "모든 동작이 ID로 등록된 command이고, 여러 표면이 그걸 호출만 한다"는 동일 구조를 전제한다. 현재: 백엔드는 command화됨(invoke/agent_command/daemon_*), 그러나 **UI/레이아웃 동작(분할·슬롯 배치·레이아웃 저장·테마·트리 조작)은 프론트(Zustand) 전용** — LLM·palette·키·트레이가 닿을 단일 진입점이 없다(§5 갭). 이대로 기능을 추가하면 "UI 먼저, 제어 나중"이 반복돼 매번 여파가 생긴다.

## 결정 (제안)

**모든 동작을 `id + 메타 + handler`로 command registry에 등록하고, palette·키바인딩·메뉴·트레이·LLM은 전부 그 registry의 소비자(파생)로 둔다.**

- command = `{ id, title, category, handler, when?(context), defaultKeybinding? }`.
- **단일 진입점:** 사람 클릭 = command 호출, LLM 호출 = 같은 command 호출, palette 검색실행 = registry 조회 후 호출, 키 = `키→command id` 맵, 트레이/메뉴 = command id 참조.
- **새 기능 추가 = command 등록 1개** → 자동으로 palette에 뜨고, 키바인딩 가능, LLM 호출 가능, 메뉴/트레이에 얹힘. **추가 여파 0에 수렴**(blast-radius 목표의 완성형).

## 거부한 대안
- **기능마다 UI + 제어를 따로 구현(현 갭):** 매 기능이 프론트 직접 수정 + 제어 경로 별도 → 여파 큼, §5 위반, palette/키바인딩이 기능마다 수동 배선.
- **백엔드만 command화, UI는 프론트 전용 유지:** LLM이 UI/레이아웃을 못 만짐(§5 미달). palette에 UI 동작이 안 뜸.
- **하드코딩 메뉴/키맵:** 새 기능마다 메뉴·키맵을 손으로 수정 → rot·여파. (CLAUDE.md ADR rot 방지 정신과 동일하게 거부.)

## 근거
- VS Code 검증 모델: `contributes.commands`(registry) → Command Palette + `keybindings.json`(키→id) + 메뉴(`when` 절) 전부 registry 파생. 10년+ 운영된 확장성 패턴.
- ADR-0020/0021에서 이미 "단일 진입점이 추가 여파를 줄인다"를 백엔드에 적용 — 이 ADR은 그 원칙을 **UI 포함 전 동작**으로 확장.

## 영향 / 방향 (구현 전이라도 지금부터 지킬 것)
- 앞으로 만드는 command(데몬 lifecycle·UI/레이아웃 등)는 **registry 호환 형태(안정적 id + 메타)** 로 쌓는다 — 나중에 palette/키바인딩 얹을 때 기존 것 안 흔들리게.
- **§5 UI/레이아웃 제어 표면 갭**을 이 registry로 흡수하는 게 전제(프론트 Zustand 액션을 command로 승격).
- **#2 트레이**는 이 방향의 첫 사례 — 트레이 항목 = registry command 참조(별 로직 0).

## 미해결 (구현 단계에서 결정 — 임의 확정 금지)
- registry 위치: 프론트(JS) 단독 vs 백엔드 미러 vs 양쪽(LLM은 백엔드측에서 introspect). LLM이 command 목록을 발견하는 경로.
- command id 네임스페이스 규약, `when`-context 모델, 키바인딩 저장 위치(프론트 localStorage vs 백엔드 config).
- 백엔드 command(invoke/agent_command)와 프론트 UI command를 한 registry로 어떻게 통합 표현하나(브리지).
- → 구현 착수 시 prior-art(VS Code command/keybinding/when) 재조사 + 선택지를 사용자 결정으로.
