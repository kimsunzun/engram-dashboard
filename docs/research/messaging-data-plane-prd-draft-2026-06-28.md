# 메시징 data-plane PRD 초안 — 옵션셋 (오너 결정 대기)

**상태:** draft (PRD-stage 옵션셋). design-only. **`/review prd` 적대검증 미통과** — 통과 후 정식화·docs/process로 승격. **채택=오너 결정**(임의 확정 금지, 산출은 옵션셋까지).
**작성:** dashboard1 (wip/a1) · db2 작업배정(메시징 data-plane 설계) 이행. 2026-06-28.
**입력:** `agent-messaging-survey-2026-06-28.md`(OSS 서베이, cross-family medium) · `llm-control-surface-message-command-scope-2026-06-28.md`(스코프).
**제약:** docs-only — protocol crate / src-tauri / ViewManager 코드 **무접촉**(메인 소유). 본 문서는 *설계 옵션*이지 구현이 아니다.

## 0. 목적·스코프
에이전트 간(A→B) 메시지 **data-plane** — "한 에이전트가 보내고 다른 에이전트가 받는다". control-plane(커맨드: 레이아웃·spawn·테마 등 LLM 제어)과 **논리 구분**. 1차 목표 = 스코프 시나리오 6번(에이전트 간 통신 확인).

**설계 전제(현실 제약):** 수신자 = PTY 프로세스(claude CLI), I/O = stdin/stdout 텍스트뿐. 구조화 reply 없음 → **"응답 인식"은 별도 seam으로 미룸**(본 PRD 비목표). 전달 = 결국 수신자 stdin 주입 + 오케스트레이터 라우팅. 봉투 타입은 *시스템엔 의미, PTY엔 텍스트로 degrade*.

## 1. ★ 핵심 결정 (오너가 가장 먼저 정할 것)

### D1. 전달 클래스 표면화 여부 — `ephemeral / durable / state_latest / command_rpc`
메인 protocol이 이미 **암묵 구현 중**(broadcast=ephemeral · replay buffer/persistence=durable · watch 최신값=state_latest · request_id reply=command_rpc). 이걸 **타입으로 표면화할지**가 이 PRD의 가장 굵은 결정이다.
- **A (표면화 — 추천):** 봉투에 전달클래스를 명시 enum/필드로. *장점:* 의도 명시·교체성·LLM이 클래스 선택·오배달 방지. *단점:* 스키마 1겹↑.
- **B (암묵 유지):** 현행처럼 채널 종류로 암묵. *장점:* 단순·현상유지. *단점:* 의도가 코드에 안 드러나 다음 세션이 오용/재발명.
- **C (부분 표면화):** durable·command_rpc만 명시, ephemeral·state_latest는 암묵.
- *거부대안 1줄:* "전달보장을 전송로(tokio/NATS)에 위임, 봉투엔 안 둠" → 전송로 swap 때 at-most-once↔at-least-once 의미 불일치로 깨짐.

> 나머지 선택(2절)은 survey에서 양 family가 강수렴해 비교적 정설 — D1만 진짜 갈림길.

## 2. 옵션셋 (각 굵은 선택 + 거부대안 1줄)

### C1. control / data plane 분리
- **A (논리 분리 · 물리 전송로 공유 — 추천):** 스키마·의미는 분리, 전송로(WS/tokio/NATS)는 공유 가능.
- B (완전 단일 버스): 초기 단순하나 auth·취소·chat혼동 리스크.
- *거부:* 단일 버스 — authorization/취소/chat 의미가 섞여 저위험-over-eng 원칙(§0) 위반.

### C2. supervised actor small layer (Ractor식)
- **A (small 개념층 자체구현 — 추천):** 메일박스+supervision 개념만 얇게, **bounded 병용**(PTY 고빈도 출력 백프레셔).
- B (Ractor 직접 의존): 원격·PG 라우팅 이득이나 기본 unbounded 메일박스 주의 + 의존 추가.
- *거부:* Actix — 원격 불가·개발 정체·tokio 마찰.

### C3. agentic 봉투 (`Task / Message / Handoff / Approval / RunTrace`)
- **A (타입드 의미 봉투 — 추천):** transport 위 레이어. correlation·deadline·audit 메타 동반.
- B (단일 opaque message): 단순하나 handoff/approval/trace 메타를 못 실음.
- *거부:* 봉투 없이 raw 텍스트만 — 에이전틱 요구(승인 게이트·라우팅·멱등 replay) 불가.

### C4. peer 자유토론 금지 / 오케스트레이터 필수
- **A (오케스트레이터 경유 강제 — 추천):** 모든 A→B는 supervisor 통과.
- *거부:* peer groupchat — 오케스트레이터 없는 자유토론은 오류 증폭(survey 명시).

### C5. A2A = 외부 어댑터 전용
- **A (외부 boundary 어댑터만 — 추천):** 외부 상호운용만, 내부 PTY 라우팅엔 X.
- *거부:* A2A 내부 버스 — 신생 + 내부 PTY 부적합.

### C6. 전송로 seam (now / future)
- **A (Transport seam 트레이트 뒤 tokio now / NATS JetStream swap future — 추천).** ADR-0020(전송의미론)·0028(single-push) 일관.
- *거부:* Redis(라이선스·설계부적합) · RabbitMQ(Erlang서버 과중) · MQTT(IoT 추상화 어색) · ZeroMQ(영속·보장 X).

### C7. 영속 모델 (이번 세션 대화 결론 반영)
- **A (에이전트=저장항목 수명 종속 · durable · cascade 삭제 · lazy cleanup — 추천):** 메시지 수명 = 수신 에이전트(저장 항목) 수명. 삭제 시 cascade. spawn/삭제 시점 lazy 정리. transient(저장 안 된) 에이전트 = 휘발 mailbox.
- *거부:* 무기한 영구 보존 — 죽은 에이전트 앞 메시지 무한 적체.
- *미결(구현 때 확정):* 키(profile id vs runtime AgentId) · 전달보장(at-least-once+dedup vs at-most-once) — 응답인식 seam과 함께.

## 3. 놓친 대안 / 공백 / 한계
- **만장일치 ≠ 정답:** survey 양 family가 tokio+NATS+actor+에이전틱 분리에 강수렴 — 잘 정립된 패턴이라 저위험이나 공통 학습편향 가능성 인지.
- Ractor 전달보장 at-most-once = 문서 미명시(불확실) — 채택 시 실측 필요.
- **응답인식**(PTY stdout → reply 판정)은 본 PRD 비목표 — 별 seam·별 결정.
- D1의 정확한 클래스 집합(4개가 맞나)은 **메인 protocol 실제 동작 확인 후** 미세조정(읽기 허용, 편집 금지).

## 4. 결정권 / 다음 스텝
- 채택 = **오너**(옵션셋까지만, 임의 확정 금지). 굵은 결정 → ADR.
- 다음: **`/review prd` 적대검증** → 오너 옵션 선택 → **ADR-0014(오케스트레이션 후보) 갱신** + TRD.
- 발견 체인: 입력 survey 2 + scope → 본 PRD → (예정) ADR-0014 / TRD.
