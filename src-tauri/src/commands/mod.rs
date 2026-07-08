// ADR-0029: embedded(in-process 호스팅) 제거 → daemon-only. 앱은 데몬 클라이언트 셸이라
// 에이전트 관련 command(옛 agent_command/agent_connect)는 없다 — 프론트가 WS 로 데몬에 직접 붙는다.
// 남은 command: discovery(데몬 발견/lifecycle), tray(창 show/hide/완전종료), autostart(부팅 자동 시작).
pub mod discovery;
// ADR-0026 2단계: 트레이 동작(창 show/hide/완전종료)의 §5 LLM 제어 표면(트레이 핸들러와 같은 함수).
pub mod tray;
// ADR-0027 §53~55: 부팅 자동 시작 토글(set/get_autostart). ADR-0029: set_mode(모드 전환) 제거 — 모드 없음.
pub mod autostart;
// ADR-0035: 레이아웃 권위 = src-tauri. ViewManager 상태변경 invoke + emit(§5 LLM 제어 표면).
pub mod layout;
// S14 모듈①(ADR-0036) T6a: 에이전트 명령 request/reply 평면(spawn/kill/interrupt/write/resize) →
// DaemonClient::send_command. §5 LLM 제어 표면(프론트 클릭·LLM 동일 진입점).
pub mod agent;
// 슬롯 팝업 분리(pop-out): 슬롯 agent 를 런타임 생성 OS 창으로 MOVE(ADR-0035/0046 라우팅 재사용). §5 표면.
pub mod popout;

pub use agent::*;
pub use autostart::*;
pub use discovery::*;
pub use layout::*;
pub use popout::*;
pub use tray::*;
