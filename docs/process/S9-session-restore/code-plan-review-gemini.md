# Gemini 응답

제시된 ① 코드 기획(session-restore-code-plan.md)을 ② 설계 근거(session-restore-lld.md) 및 실제 구현 시 마주할 edge case 관점에서 하드코어하게 검토한 결과입니다.

결론부터 말씀드리면, 전반적인 구조는 탄탄하나 설계대로 구현할 경우 무조건 터지는 '치명적인 레이스 컨디션 및 예외 처리 결함'이 4가지 발견되었습니다. 이를 보완해야 구현에 진입할 수 있습니다.

1. 결함 및 취약점 분석 (Adversarial Critique)
① [치명적] self_test 타임아웃 레이스 컨디션

기획 내용: spawn 직후 1회 self_test(child_pid, sid) 수행하여 <pid>.json 존재 및 매칭 확인.

결함: spawn 직후(몇 밀리초 이내)에는 Claude CLI가 구동되어 sessions/<pid>.json 파일을 채 생성하기도 전일 확률이 99%입니다. 즉, 정상적인 상황임에도 self_test가 파일 미발견으로 실패하고 "가정 붕괴 조기 감지" 로그를 무수히 뿜어내게 됩니다.

해결책: self_test는 즉시 1회 실행이 아니라, 최대 1~2초간 지수 백오프(Exponential Backoff)를 갖는 폴링(Polling) 루프로 구현하거나, session_tracker 비동기 태스크에서 파일 생성 이벤트가 감지될 때까지 대기하는 구조여야 합니다.

② [치명적] spawn_session_watcher 시그니처의 스레드 안정성 결여
Rust
pub fn spawn_session_watcher(agent_id: AgentId, expected: Uuid, on_change: impl Fn(Uuid));


결함: Rust에서 디렉토리 watch(notify 크레이트 등)는 별도의 OS 스레드 또는 백그라운드 런타임 태스크에서 동작합니다. 따라서 내부에서 on_change 콜백을 실행하려면 이 클로저가 반드시 Send + Sync + 'static을 만족해야 합니다.

또한, 이 콜백 안에서 기획서대로 "즉시 atomic persist"를 하려면 PtyManager 내부의 프로필 맵을 수정하고 파일로 저장해야 하므로, 복잡한 소유권(Lifetimes) 충돌이 발생합니다.

해결책: 클로저 주입 방식 대신 크로스비ーム 채널(crossbeam_channel::Sender)이나 Tokio MPSC 채널을 넘겨 watcher가 변경 이벤트를 중앙 루프로 던지게 하고, PtyManager가 이벤트를 수신하여 스레드 안전하게 수정 및 저장(persist)을 전담해야 합니다.

③ [치명적] 잘못된 Fallback 조건으로 인한 '대화 강제 유실' 위험

기획 내용: --resume 실패 시 새 UUID로 Fresh fallback.

결함: --resume이 실패하는 원인은 "대화가 서버에서 사라짐(Exit 1 / No conversation found)" 외에도 인터넷 단절, Claude 서버 일시 장애, 자격증명(Token) 만료 등이 있습니다. 만약 네트워크가 잠깐 끊겨서 실패한 것인데도 시스템이 무조건 Fresh fallback을 감행해 새 UUID를 발급하고 저장해 버리면, 사용자의 기존 대화 컨텍스트가 영구히 분리(유실)되는 대참사가 발생합니다.

해결책: Exit 1 이면서 stderr 스트림에 구체적인 오류 문자열(No conversation found)이 찍힌 경우만 명시적 Fresh fallback을 트리거해야 합니다. 네트워크 오류나 기타 exit code인 경우 복원을 보류하고 ExitedWithError 상태로 남겨두어 사용자가 재시도할 기회를 주어야 합니다.

④ [잠재적] Atomic Persist의 std::fs::rename 크로스 파일시스템 오류

기획 내용: agents.json.tmp 쓰기 → flush → rename(atomic).

결함: 만약 임시 파일(agents.json.tmp)이 생성되는 OS의 시스템 Temp 디렉토리와 실제 agents.json이 저장되는 앱 설정 디렉토리가 서로 다른 파티션/드라이브(크로스 파일시스템)에 위치할 경우, OS 레벨의 rename은 EXDEV 에러를 내며 실패합니다.

해결책: agents.json.tmp를 반드시 agents.json과 동일한 부모 디렉토리 안에 생성한 뒤 rename 하도록 강제해야 합니다.

2. 미래 확장성 검토 (YAGNI vs Prepare Now)

현 구조(OutputSink/StatusSink trait 기반 코어 분리)가 다음 요구사항을 유연하게 흡수할 수 있는지에 대한 검증입니다.

(a) Codex CLI 지원 (AgentCommand 확장)

판정: 흡수 가능 (Prepare Now 완료)

AgentCommand가 enum으로 설계되어 있어 Codex { ... } 변형을 추가하는 것은 매우 쉽습니다.

주의점: session_tracker 내부 로직은 ~/.claude/sessions/라는 고도로 Claude 종속적인 경로를 탐색합니다. Codex가 도입될 때를 대비해 session_tracker를 PtyManager가 직접 꽉 쥐게 하지 말고, AgentCommand 타입에 따라 워처 활성화 여부를 결정하는 분기 코드를 spawn_agent에 미리 열어두어야 합니다.

(b) 모바일 원격 제어 (PtyManager 위 WebSocket 레이어)

판정: 흡수 가능 (YAGNI 경계)

StatusSink를 통해 agent-restore-result와 epoch를 외부로 방출(emit)하므로, Tauri 이벤트뿐만 아니라 WebSocket Broadcaster 스레드를 StatusSink 구현체로 붙이기만 하면 프론트엔드와 모바일이 동일한 상태를 구독할 수 있습니다.

YAGNI로 미룰 것: 모바일에서의 '동시 제어 및 입력 충돌(Race)' 레이어까지 지금 설계하는 것은 과합니다. 지금은 epoch 기반 재구독 설계가 추상화(Sink)를 잘 따르고 있는지만 확인하면 됩니다.

(c) 종량제 비-PTY API (터미널 없이 API 직접 호출)

판정: 구조적 충돌 위험 (지금 준비해야 함!)

현재 기획은 모든 Agent가 PTY 프로세스(child_pid)를 가진다는 강한 가정 하에 pty/manager.rs와 pty/session_tracker.rs가 묶여 있습니다. API 방식은 child_pid가 존재하지 않습니다.

지금 준비할 것: AgentProfile 구조체에서 PTY 종속적인 필드를 분리하거나, PtyManager라는 이름 대신 차후 AgentRuntimeManager로 추상화할 수 있도록 여지를 두어야 합니다. 최소한 child_pid를 다루는 resolve_session_file 등의 함수가 Option<u32>를 받거나 PTY 에이전트 전용 서브 모듈로 격리되어야 합니다.

3. 구현 순서상의 리스크

스파이크와 구현의 역전 위험 (단계 4):
기획서 상 단계 4에서 session_tracker + spike 추가를 진행하며 PtyManager 경로 spawn을 엮으려 합니다. 하지만 단계 5(manager: spawn_agent)가 완성되지 않은 상태에서 tracker의 self_test나 watcher를 정교하게 테스트하기 어렵습니다.

조정 권고: 단계 4에서는 완전한 매니저 연동이 아니라 Mock 프로세스(예: 5초 뒤 json 파일을 생성하는 더미 스크립트)를 띄워 session_tracker 고유의 파일 감지/한계 상황(공유위반 재시도 등)만 순수하게 검증해야 합니다.

4. 최종 판정 및 제언

"지정된 치명적 결함 4가지를 수정한 후, 구현 착수 GO"

🛠 즉시 반영해야 할 코드 수정 가이드
Rust
// 1. session_tracker.rs 의 워처 시그니처 변경 (클로저 -> 채널)
pub fn spawn_session_watcher(
    agent_id: AgentId, 
    expected: Uuid, 
    tx: tokio::sync::mpsc::Sender<(AgentId, Uuid)> // 스레드 안전한 채널 사용
);

// 2. manager.rs 의 self_test 리턴 타입 및 내부 로직에 폴링 도입
pub async fn self_test_with_retry(child_pid: u32, expected_sid: Uuid) -> SelfTestResult {
    // 즉시 실행 금지! 최대 5회 백오프 폴링으로 sessions/<pid>.json 대기
}

// 3. manager.rs 의 fallback 조건 강화
// 단순히 "실패 시" 가 아니라, 에러 텍스트 명시적 파싱 가드 필수
if exit_code == 1 && stderr.contains("No conversation found") { 
    // Fresh Fallback 진행
} else {
    // 일반 네트워크/인증 에러 처리 (상태 유지)
}


이 세 가지만 코드 기획에 명시적으로 추가 보완하시면, 현 기획은 3자 검증(fable/Gemini/GPT)의 방어 원칙을 매우 충실히 이행한 훌륭한 코어 GO 범위 구현 계획이 됩니다. 진행하셔도 좋습니다.
