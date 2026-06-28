# 리서치 — LLM 제어 표면(intent command 버스) OSS 서베이

**상태:** 설계-결정 모드 서베이(착수 전 OSS 참조). cross-family 교차(Claude Sonnet 팬아웃 + Codex blind) + opus 적대검증. **medium.**
**날짜:** 2026-06-28 · 작성: dashboard1(wip/a1) · **레퍼런스**(채택=오너 PRD 결정).
**확신도 범례:** 확실 / 가능성높음 / 불확실.

> 목적: §5 LLM-우선 제어 — 레이아웃 분할·슬롯 배치·팝업·에이전트 spawn/배치·테마 등 UI+백엔드 전 기능을 LLM(주)·사람(보조)이 **같은 핸들**로 부르는 단일 intent 버스. "이 기능을 어떻게 만들까"의 참조.

## 후보 × 핵심 (요약)

| 후보 | 핵심 메커니즘 | engram 적합도 | 라이선스/성숙도 |
|---|---|---|---|
| **VS Code Commands API** | `registerCommand(id, handler)` → 팔레트·키바인딩·`executeCommand`가 **모두 같은 핸들러** | ★ 단일 control surface의 교과서. "사람클릭=LLM호출=키바인딩 동일 핸들러" 패턴 차용 | MIT(Code-OSS)/매우높음 |
| **Zellij actions/CLI/plugin** | CLI `zellij action`·플러그인이 **같은 Action enum** 공유, `--json`·`subscribe` | ★ "모든 UI 조작 = 이름붙은 action + target 핸들" + 권한체계. Rust 네이티브 | MIT/높음 |
| **tmux control mode `-C`** | stdin/stdout 텍스트 프로토콜, `%begin/%end` + `%`비동기 알림, target ID | 비동기 알림·요청ID 구조는 데몬 WS 이벤트 참조용. 텍스트 프로토콜·Windows미지원은 직접차용 X | ISC/표준급 |
| **WezTerm cli** | 실행 인스턴스 외부 제어, spawn시 pane-id 반환 | 앱 자체 버스와 무관 — 개발/검증 도구 수준(cdp.mjs 병행) | MIT/높음 |
| **CQRS/Redux dispatch (Rust)** | 타입드 command 봉투 → handler/registry dispatch, replay/audit | ★ 내부 아키텍처. 단 `cqrs-es`/`qonduit`는 이벤트소싱 과함 → **트레이트 레지스트리 패턴만 자체구현** | MIT/Apache·중간 |
| Google A2UI | 에이전트가 선언형 JSON 컴포넌트 디스크립터 전송, 카탈로그 매핑 | "승인된 커맨드 ID 집합" 보안경계 개념만 차용. Tauri 네이티브엔 JSON왕복 불필요 | Apache-2.0/초기 |

## 교차검증표 (Claude ↔ Codex)

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| 단일 핸들러 레지스트리(사람=LLM 동일경로) = VS Code 모델 | ✓ | ✓ | **수렴·확실** |
| 모든 기능 = 타입드 action/intent enum (Zellij식) | ✓ `EngramCommand` enum | ✓ `Intent{id,version,actor,target,payload,correlation_id}` | **수렴·확실** |
| dispatch는 CQRS/Redux 패턴 차용, 단 heavy crate 배제 | ✓ (트레이트 자체구현) | ✓ (cqrs-es는 옵션, 트레이트 권장) | **수렴·확실** |
| 프론트 = 순수 I/O(invoke 한 줄), store는 이벤트 수신만 | ✓ | ✓ | **수렴·확실** |
| command ID ↔ LLM tool schema 1:1 | ✓ | (함의) | 수렴·가능성높음 |
| 교체성 = 컨트롤러 트레이트로 분리 | (함의) | ✓ `IntentHandler/AgentSpawner/LayoutController/...` | 수렴 |

**불일치 없음.** Codex가 더 granular(correlation_id·versioning·`IntentResult{events,handles}`·명시 컨트롤러 트레이트). Claude는 "command ID = LLM tool 1:1" 강조.

## 적대검증 (핵심 주장)

- **"command ID = LLM tool 1:1" 과결합 위험?** → tool 폭발·진화 경직 우려. 완화: Codex의 `Intent{id, **version**}` + 그룹핑으로 진화 흡수. **유지(가능성높음).**
- **stringly-typed ID 함정?** → VS Code가 겪는 문자열 ID 약점. engram은 **Rust 타입드 스키마**(enum/struct)로 박고 ID는 표면만 → 완화. **유지.**

## 권고 (수렴안)

Rust 코어 데몬에 **타입드 intent 레지스트리**: `Intent{ id, version, actor, target, payload, correlation_id } → IntentResult{ events, handles }`.
- **단일 핸들러**(VS Code): 사람 클릭·LLM invoke가 같은 핸들러 경로.
- **타입드 action enum**(Zellij): 레이아웃·spawn·배치·테마 전부 한 enum, protocol crate wire 타입으로 직렬화.
- **dispatch**(CQRS/Redux): 트레이트 레지스트리 자체구현(이벤트소싱 crate 배제). 커맨드 로그=audit/replay.
- **프론트=invoke 한 줄**, store는 이벤트 수신(ADR-0011 agentClient 확장).
- **교체성**: `IntentHandler`·`LayoutController`·`AgentSpawner`·`WindowController`·`ThemeController` 트레이트.
- command ID 목록 ≈ LLM tool 목록(신규기능 = 커맨드 등록 + tool 정의 동시).

## 거부 후보 → ADR 거부 대안 후보

- `cqrs-es`/`qonduit` 직접 채용 — 이벤트소싱·saga 과설계, 데스크탑 앱엔 과함.
- WezTerm cli / tmux 텍스트 프로토콜 — 앱 내부 버스로 부적합(외부 제어 도구·Windows·타입안전 X).
- A2UI 직접 채용 — Tauri 네이티브에 JSON 왕복 불필요(개념만 차용).

## 공백·한계

- 만장일치 ≠ 정답: 두 family가 "타입드 command 버스 + 트레이트" 패턴에 공통 수렴 — 잘 정립된 패턴이라 저위험이나, 공통 학습편향 가능성 인지.
- Rust CQRS crate 실활성도(cqrs-es 등) 미검증 — 어차피 자체구현 권고라 영향 적음.
- S14 ViewManager 실제 API와의 인터페이스(레이아웃 커맨드가 ViewManager를 구동)는 **별도 확인 필요**(dashboard2 도메인). 이 보고서는 패턴까지.

## 출처

VS Code Commands https://code.visualstudio.com/api/extension-guides/command · Zellij https://zellij.dev/documentation/cli-actions.html · https://zellij.dev/documentation/plugin-api-commands.html · tmux control mode https://github.com/tmux/tmux/wiki/Control-Mode · WezTerm cli https://wezterm.org/cli/cli/index.html · CQRS https://learn.microsoft.com/en-us/azure/architecture/patterns/cqrs · cqrs-es https://docs.rs/cqrs-es/ · mediator https://docs.rs/mediator/ · Redux https://redux.js.org/tutorials/fundamentals/part-2-concepts-data-flow · A2UI https://developers.googleblog.com/introducing-a2ui-an-open-project-for-agent-driven-interfaces/ · Tauri invoke https://v2.tauri.app/develop/calling-rust/
