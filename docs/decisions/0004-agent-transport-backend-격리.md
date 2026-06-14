# ADR-0004: AgentTransport seam + backend 지식 격리

- 상태: 확정 (S10)
- 관련: CLAUDE.md §2·§3 · `transport/mod.rs` · `backend/{mod,claude}.rs` · `types.rs::CommandSpec`

## 맥락
멀티 백엔드(claude/codex 콘솔 + API)를 한 인터페이스로 통합해야 한다. 백엔드별 인자 조립(claude `--session-id`/`--resume`)과 전송 방식(PTY/HTTP)이 코어에 새면 결합이 생긴다.

## 결정
- **전송 = `dyn AgentTransport` seam** — start/send_input/resize/interrupt/shutdown/capabilities. PtyTransport(콘솔 공용)/ApiTransport(API)가 같은 trait로 끼워진다.
- **명령 조립 = `backend/`** — `CommandSpec{program,args,env,cwd}`만 산출. transport는 어느 백엔드인지 모른다. claude 전용 지식은 `backend/claude.rs` 한 곳에만.

## 거부한 대안
- **manager가 claude 인자를 직접 조립** — claude에 결합되어 새 백엔드마다 manager 수정. 격리 실패.
- **transport가 백엔드를 알게** — PtyTransport에 claude/codex 분기가 새어 교체성 깨짐.

## 근거
"교체 가능" 대원칙(§0). ApiTransport가 같은 trait로 끼워짐을 S10 fable 게이트에서 확인.

## 영향 / 불변식
- manager는 `backend::needs_session()`/`build_command_spec()`/`backend_for` dispatch만 부른다.
- codex/gemini는 CLI spike 후 `AgentCommand` variant 추가 + `backend_for` 라우팅 연결(현재 stub·미연결).
