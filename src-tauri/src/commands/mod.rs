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

pub use autostart::*;
pub use discovery::*;
pub use layout::*;
pub use tray::*;
