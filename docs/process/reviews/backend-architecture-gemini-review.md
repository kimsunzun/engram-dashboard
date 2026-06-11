# Gemini 검토 결과: backend-architecture-final

**검토 일시:** 2026-06-08
**대화 URL:** https://gemini.google.com/app/34e13952777bff45
**모델:** Gemini 3.5 Thinking

---

Engram Dashboard의 Rust 백엔드 설계안은 Tauri v2의 최신 기능(Channel)을 적극적으로 활용하고, 멀티 창 구조에서 발생할 수 있는 후발 창 attach(Replay Buffer) 문제까지 고려한 매우 탄탄하고 성숙한 구조입니다.

---

## 1. portable-pty가 Tauri v2 + Windows 환경에서 실전 사용 가능한가? 알려진 이슈?

결론: 현재 Rust 생태계에서 크로스플랫폼 PTY를 구현할 때 가장 믿을 만한 선택지입니다. WezTerm의 기반 크레이트이므로 신뢰성이 높습니다.

**비동기(Async)와의 상성 문제:**
portable-pty는 동기(Sync) I/O 기반입니다. Tauri의 Command나 Tokio 비동기 런타임 내에서 blocking 방식으로 호출하면 전체 런타임 스레드가 고갈될 수 있습니다.
- **보완책:** std::thread::spawn 또는 tokio::task::spawn_blocking으로 완전히 독립된 OS 스레드에서 I/O 처리 필수. 설계안의 "별도 thread에서 drain loop 운영" 명시는 훌륭한 선택.

**Windows 자식 프로세스 좀비화 (Zombie Processes):**
Windows ConPTY에서 상위 PTY 프로세스를 kill 하더라도, Claude Code나 하위 서브 프로세스(Node.js 등)들이 죽지 않고 백그라운드에 살아남는 고질적인 문제가 있습니다.
- **보완책:** Windows 환경에 한해서 PTY 생성 시 자식 프로세스를 Windows의 **Job Object**에 등록하고 JOBOBJECT_EXTENDED_LIMIT_INFORMATION 설정(KillOnJobClose)을 켜주어야 부모 프로세스가 죽을 때 하위 프로세스들이 깨끗하게 정리됩니다.

---

## 2. Tauri v2 Channel 기반 PTY 스트리밍 — 이 설계에서 놓친 pitfall?

Tauri v2 Channel은 최선의 선택이지만, **백프레셔(Backpressure)**와 **청크 쪼개짐** 문제를 고려해야 합니다.

**Rust 단에서의 배칭(Throttling/Batching) 누락:**
설계안은 프론트엔드에서 8~16ms 배칭을 하지만, 대량 출력 시(cat으로 거대 파일 출력 또는 Claude가 코드를 길게 뽑을 때) Rust drain thread가 수 바이트 단위로 channel.send를 무차별 호출하면 IPC 파이프가 터지거나 프론트엔드가 얼어버릴 수 있습니다.
- **보완책:** Rust drain thread 단에서 최소 5~10ms 버퍼링하거나 4KB가 모이면 channel.send를 호출하도록 **Rust 단의 배칭 메커니즘** 반드시 추가.

**UTF-8 및 ANSI Sequence 쪼개짐 에러:**
4KB~32KB 단위로 스트림을 임의로 자르면 멀티바이트 UTF-8 문자 중간이나 ANSI 이스케이프 시퀀스([31m 등) 중간이 잘려 xterm.js에서 화면 깨짐/사각형 노출될 수 있습니다.
- **보완책:** 가장 안전한 방법은 바이너리(Vec<u8>) 스트리밍 채택. xterm.js는 불완전한 UTF-8을 어느 정도 버퍼링하나, Replay Buffer 저장 시 바이트 경계선 구분 필요.

---

## 3. drain thread + replay buffer + subscriber 관리 구조의 개선 포인트?

현재 구조는 Arc<Mutex<PtySession>> 내부에 모든 상태가 모여 있어 **락 경합(Lock Contention)**이 발생하기 쉽습니다.

**락 범위(Lock Granularity) 최소화:**
drain thread가 subscribers 순회 + replay_buffer 추가 중에 프론트엔드가 write_stdin/resize_pty로 PtySession 전체 MutexGuard를 쥐면 drain thread가 블로킹되어 스트리밍이 밀립니다.

**개선안 — PtySession 내부 분리:**



**Replay Buffer 용량 산정 규칙:**
단순 VecDeque<PtyChunk> 대신 **최대 바이트 크기(1MB~2MB)** 또는 **최대 라인 수(5,000라인)** 제한을 명확히 두어 메모리 누수 방지.

---

## 4. xterm.js <-> Tauri v2 Channel 연동 주의사항?

**바이너리(Vec<u8> / Uint8Array) 전송 추천:**
PTY 출력은 본질적으로 바이너리 시퀀스. Rust에서 String::from_utf8_lossy 변환을 거치지 않고 Vec<u8> 형태로 쏘는 것이 CPU 연산과 인코딩 버그 방지 측면에서 훨씬 유리. xterm.js의 terminal.write()는 Uint8Array를 네이티브로 지원.

**후발 창 Attach 시점의 크기(Resize) 불일치:**
팝업 창이 뒤늦게 열리면서 Replay Buffer를 들이부을 때, 팝업 창의 xterm.js 크기(cols, rows)가 최초 에이전트를 생성했던 메인 창 크기와 다르면 줄바꿈/깨짐 발생.
- **주의사항:** subscribe_agent_output 호출 전에 팝업 창이 자신의 terminal 크기를 먼저 resize_pty로 전송하거나, Rust가 기억하는 세션의 기존 cols/rows를 받아와 xterm.js 크기를 먼저 맞춘 뒤 Replay Buffer 렌더링 필요.

---

## 5. 전체 구조에서 놓친 것, 더 개선할 설계 포인트?

### 앱 강제 종료/크래시 시 찌꺼기 프로세스 청소 (Orphan Process)

Tauri 앱이 정상 종료 시엔 kill_agent가 돌지만, 작업 관리자 강제 종료나 Rust 패닉 크래시 시 Claude Code 백그라운드 프로세스가 좀비로 잔존합니다.
- **개선안:** Tauri setup 단이나 plugin 수명 주기에서 시스템 시그널 처리하거나, Windows **Job Object** / Unix prctl(PR_SET_PDEATHSIG, SIGKILL) 같은 OS 레벨 부모-자식 생명주기 결속 장치 연동.

### 사용자 환경 변수(PATH) 상속 문제

Claude Code는 내부적으로 node, npm 등 시스템 도구를 호출합니다. 개발 환경에서 Tauri 실행 시 PATH가 잘 잡히지만, **프로덕션 패키지(dmg, exe) 더블 클릭 실행 시 시스템 환경 변수가 온전히 상속되지 않아** spawn_agent가 Claude Code 내부 진입 후 실패할 확률이 높습니다.
- **개선안:** spawn_agent 시 사용자의 실제 셸 환경 변수(.zshrc, .bash_profile, Windows 사용자 환경변수)를 로드하여 PTY 프로세스 env에 명시적 주입하는 **fix-path** 기법 필요.

---

### 요약: 피드백 반영 아키텍처 제안 구조



이 정도의 엣지 케이스들만 보완하신다면, 상용 서비스 수준의 극도로 안정적이고 빠른 멀티 윈도우 터미널 대시보드를 완성하실 수 있을 것입니다. 구조가 매우 탄탄하여 큰 수정 없이 디테일업만 하시면 되겠습니다!
