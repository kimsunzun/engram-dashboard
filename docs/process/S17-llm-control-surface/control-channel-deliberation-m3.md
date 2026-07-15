# S17 제어 채널 — engram-ctl → in-band 출력 마커(M3) 피벗 (쟁점 정리)

> **상태: 방향 확정 = M3 · 정식 박제 완료 = ADR-0085**(2026-07-15 — ADR-0080 전체 폐기, ADR-0014는 무관해 손대지 않음). **PRD/TRD §6 갱신·Unit 재설계는 아직**(추가 작업). 이 문서 = 결정에 이른 쟁점·옵션·실측의 스냅샷 — 결정 정본은 이제 **ADR-0085**.
> 날짜: 2026-07-15 · 앵커: ADR-0014(CLI-via-Bash)·ADR-0080(제어표면)·ADR-0002(capability matrix)·ADR-0079(resume seed 경계)·ADR-0003(core 격리).

## 1. 문제
에이전트 A→B 메시지 전송(및 향후 UI·관찰 제어)의 **채널**. 원안 = **engram-ctl**(Bash로 부르는 Rust CLI → WS → 데몬, 토큰 인증; ADR-0014/0080, TRD §6 슬라이스 1). 사용자 제기: **in-band면 보안·속도가 월등** — 에이전트에 토큰/포트를 안 줘 공격면↓, per-call 프로세스 스폰+WS 핸드셰이크가 없어 고빈도 메시징에 결정적.

## 2. 검토 옵션 (M1~M5)
- **M1 engram-ctl CLI**(원안): 에이전트 env에 토큰 노출 + 호출마다 프로세스 스폰+WS 핸드셰이크. 고빈도서 손해.
- **M2 MCP**(데몬이 로컬 MCP 엔드포인트 노출): 구조화·양방향 최상. 단 **툴 스키마가 에이전트 컨텍스트에 상주(~0.5~1.5k 토큰/에이전트)** + 엔드포인트 인증·권한 게이트. ADR-0080이 이미 "ceremony 과함"으로 기각한 방향.
- **M3 출력 마커**: 에이전트가 출력 텍스트에 마커 → **데몬이 이미 소유한 stdout에서 줍기** → 데몬이 mail처럼 배달. 토큰0·스폰0·가장 직접적.
- **M4 Bash sentinel + hook**: hook이 툴 실행 길목을 가로챔. 구조화 캡처·출력포맷 무관. 단 **"우리가 콜백받는" 게 아니라** claude가 띄운 스크립트가 밖으로 릴레이하는 **간접** 방식 + **데몬에 인그레스 없음**(WS만) → 신설 필요 + 메시지마다 툴콜 비용.
- **M5 파일 drop**: 견고하나 매 호출 프로세스 스폰.

## 3. 실측 (claude 2.1.170 · `.spike-control/` — throwaway, git 미추적)
- **M4 hook 발화 ✓**: 우리 정확한 헤드리스 호출(`claude -p --input-format stream-json --output-format stream-json --verbose`)에서 PreToolUse hook 발화. `tool_input.command`+`session_id` 캡처. `--settings <path>` 및 cwd `.claude/settings.json` per-agent 주입 O. deny로 실제 실행 차단 O.
- **M3 마커 방출 near-perfect ✓**: 엄격 단일라인 지시 시 마커 라인 **38/38 완벽**(유효 JSON·수신자/본문·이스케이프 정확). 유일 함정 = 포맷을 *설명*할 때 4/5로 ```펜스 예시``` 안에 마커를 찍음 → **파서가 코드펜스 안 라인 스킵**으로 해결. (펜스블록 마커는 이 문제 못 풀어 오히려 나쁨 → **단일라인 센티넬** 채택.)
- **라이브/리플레이 경계 코드 확인**: resume seed는 `output_core.seed()`로 **fanout 없이** 적재, 라이브만 `emit()` 경유 → **마커 파서는 리플레이/스크롤백을 절대 안 봄**(부작용 재실행 없음). ADR-0079 정합.

## 4. 결정 (방향)
- **M3 = 주 경로** (Claude Code CLI 백엔드). 근거: CLI에선 데몬이 출력 스트림을 **원래 소유**하므로 마커 줍기가 최단 경로. hook은 인프로세스 콜백이 아니라 간접 릴레이 → 무겁고 어색. 운영이 **항상 json 모드 + 고빈도**라 M3의 **제로 오버헤드**가 결정적.
- **진짜 인프로세스 콜백 = API/SDK 백엔드에서만** (우리가 추론 루프 소유 → tool_use 직접 수신). capability matrix(ADR-0002): 같은 "에이전트가 제어 신호를 낸다" 능력의 백엔드별 구현 — **API=직접 콜백 · CLI=M3**.
- **M4 = 문서화된 폴백** (터미널 모드·json 포맷 붕괴 헤지). 지금 구현 X — 필요 시 착수.
- **engram-ctl(ADR-0014/0080) 폐기 방향** — 정식 supersede + PRD/TRD 갱신 + Unit 재설계는 추가 논의 후.

## 5. 거부 대안 + 근거
- **engram-ctl(M1)**: 토큰 노출 + per-call 스폰/WS 비용(고빈도 손해).
- **MCP(M2)**: 컨텍스트 토큰 상주 비용 + 엔드포인트/권한 ceremony (ADR-0080 기존 기각 유지).
- **hook-as-primary(M4)**: 간접 릴레이(콜백 아님) + 데몬 인그레스 신설 + 툴콜 비용 → 백업으로만.
- **파일 drop(M5)**: 매 호출 스폰.

## 6. 열린 항목 (추가 논의)
- **정식 ADR 박제** — engram-ctl(ADR-0014/0080) supersede + M3 결정.
- **PRD/TRD(§6 engram-ctl 표) 갱신**, Unit #1~4 재설계(engram-ctl 전제 폐기).
- **★병렬 세션 충돌★**: 워킹트리에 병렬 세션이 만든 **engram-ctl 크레이트**(`crates/engram-dashboard-ctl/`: catalog/envelope/error/v1.rs + Cargo.toml 멤버 추가, **미커밋**)가 있음 — 우리가 폐기하기로 한 그것. **조율 필요**(폐기/보존 사용자 결정).
- **M3 고도화 설계**: 마커 프로토콜(센티넬·JSON 스키마)·펜스스킵 파서·`ControlSignal` OutputEvent·표시에서 마커 억제·한 턴 다중 메시지·데몬 mail(보관/배달/ACK 범위).
- Codex 중립 설계 리뷰 여부.

## 7. 구현 seam (실측 매핑 — 참고, 확정 아님)
- **공통 배달**: `AgentManager::write_stdin(agent_id, data)` (`manager.rs:638`) → `session.write_input` → 대상 stdin. 데몬 호출부 `connection_core.rs:714`.
- **M3**: `OutputEvent::ControlSignal{target, message}` 신설(`types.rs`) + 디코더 마커검출(`backend/claude.rs` — 라이브 TextDelta) + 데몬이 ControlSignal 가로채 `write_stdin` (`ws.rs`/`connection_core.rs`). core 격리(ADR-0003) 유지 — 마커 검출은 core에서 이벤트만 방출, 라우팅은 데몬.
