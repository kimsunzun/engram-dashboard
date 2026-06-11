# wezterm 세션 복원 실패 분석 — 대시보드 설계 전제

세션 저장/복원 설계 **전에** 깔고 가는 선행 분석. 기존 wezterm 기반 복원 시스템(`~/.config/wezterm/`)이 왜 불안정했는지 코드로 분석하고, 대시보드가 같은 함정을 피할 방어 원칙을 도출한다.
**결론 먼저: claude `--resume` 세션 복원은 100% 신뢰 불가. 우리 메타데이터는 독립 저장하고, claude 복원은 best-effort로 다룬다.**

## 1. 기존 wezterm 복원 시스템 (현장)

| 파일 | 역할 |
|------|------|
| `layout_capture.lua` | Ctrl+Shift+S로 레이아웃 저장. `is_claude_pane`로 claude 패인 감지 |
| `sid-map.json` | `cwd:win:tab:pane` → claude 세션 uuid 매핑 |
| `workspaces.lua` | `claude --resume <sid>` (powershell)로 복원. `OUR_SID` 하드코딩 |
| `split-debug.log` / `save-debug.log` | 5분 주기 dump 로그 |

복원 방식: `powershell -NoExit -Command "Set-Location <cwd>; claude --resume <sid>"` — sid는 sid-map에서 위치 키로 조회.

## 2. 실패 메커니즘 (왜 못 믿나)

1. **위치 기반 세션 매핑** — `sid-map.json` 키가 `cwd:win:tab:pane`(인덱스). 패인을 **추가/이동/삭제하면 인덱스가 밀려** 엉뚱한 세션이 복원됨. (실제로 "패인 옮기니 ID가 바뀜" 증상의 원인)
2. **프로세스명 기반 claude 감지** — `is_claude_pane`이 foreground 프로세스명 `node`/`claude`로 판별. claude가 node 기반이라 **다른 node 프로세스(vite dev server 등)를 claude로 오탐**.
3. **세션 ID 비결정성** — sid를 claude가 생성하고 그걸 캡처하는 구조라 캡처 타이밍/정확도 의존. `OUR_SID` 하드코딩은 이 비결정성을 못 견뎌 특정 세션을 코드에 박은 흔적.
4. **claude --resume 자체의 취약성** — TUI를 새 PTY에서 재개 → redraw/cols·rows 의존, 세션 손상 시 동작 불명.

## 3. 대시보드 방어 설계 원칙 (도출)

1. **세션 식별 = 안정적 고유 ID(AgentId/uuid), 위치 무관.** 슬롯 인덱스에 세션을 묶지 않는다. (위치 기반 매핑 함정 회피)
2. **claude 프로세스 추적 = 우리가 spawn → child PID/AgentId 직접 보유.** 프로세스명(node) 감지에 절대 의존하지 않는다. (오탐 회피)
3. **claude 세션 ID는 우리가 통제** — spawn 시 우리가 세션 ID를 지정할 수 있으면(`--session-id` 등, claude-code-guide로 확인) 그렇게 해서 비결정성 제거. 불가하면 spawn 직후 결정적으로 1회 캡처.
4. **레이아웃 ↔ 세션 분리 저장.** 슬롯 레이아웃(프론트)과 에이전트 프로필/세션(백엔드)을 별도 저장. wezterm은 둘을 위치로 묶어 깨졌다.
5. **복원 실패 graceful.** `claude --resume`이 실패(세션 없음/손상)해도 **fresh로 fallback**하고, 우리 메타데이터(프로필/AgentId)는 독립 보존되어 안 깨진다. claude 복원은 "되면 좋은" best-effort.

## 4. 설계/검토에 전달할 핵심 질문

- claude CLI가 **spawn 시 세션 ID 지정**(`--session-id`)을 지원하나? 결정적 재개 가능한가?
- `--resume <id>`를 **새 PTY에서 비대화형 spawn** 시 TUI/렌더 문제는?
- 세션 손상/부재 시 `--resume`의 동작(에러? 빈 세션? 행?) — fallback 트리거를 뭘로?

→ 이 분석 + "100% 신뢰 불가" 전제를 claude-code-guide 조사 / 웹(Gemini·GPT) / fable 검토에 동일하게 전달한다.
