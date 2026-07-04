# ADR-0037: 전송 의미론 위치 — Rust 단독 가드, 프론트 ProtocolClient 박막화

- 상태: 확정 (2026-06-27, 근거: S14 모듈① spike D1 사용자 결정 — A안)
- 관련: Amends ADR-0020 (결정3: 프로토콜 의미론 위치 — JS ProtocolClient → Rust(DaemonClient/protocol_state)) · ADR-0036(전송 중계 통일 — 이 ADR이 그 가드 위치를 확정) · ADR-0011(agentClient 제어표면 — 인터페이스 불변) · ADR-0029(daemon-only, embedded carrier 제거) · CLAUDE.md §5 · spike `docs/process/S14-multi-page-layout/module1-transport-spike.md` §5(D1) · Amended by ADR-0046 (seq dedup/진도 거처 조항: Rust 단독 → 웹뷰 뷰 단위 lastDeliveredSeq — epoch 1차 필터는 Rust 존속)

## 맥락
ADR-0036이 전송 중계를 src-tauri 단일 데몬 클라이언트(+`OutputRouter`)로 통일하기로 했으나, **프로토콜 의미론(dedup/epoch/seq 가드 등)을 어디서 돌릴지**는 미결로 남겼다(spike D1). ADR-0020 결정3은 이 의미론을 carrier-무관 JS `ProtocolClient` "한 곳"에 모았는데, 그 뒤 ADR-0029(daemon-only)로 embedded carrier가 사라지고 ADR-0036으로 **단일 Rust 연결이 창 N개로 fan-out**하는 구조가 됐다. 이 구조에서 의미론을 JS에 남기면 가드가 창마다 N회 돌거나(라우팅 후) 의미론이 두 곳에 중복된다 → ADR-0036의 "단일 연결·1회 처리" 이점이 사라진다. 그래서 가드 위치를 못 박아야 한다.

## 결정
프로토콜 의미론 — request_id pending 매칭 · seq high-water dedup · epoch 가드 · resubscribe(resume) · 끊김 시 pending reject — 을 **Rust `DaemonClient` + `protocol_state`가 단독 소유**한다. 데몬 단일 WS 연결에서 `OutputRouter` 라우팅 **전 1회** 적용하고, 창 N개로는 깨끗한 출력 청크만 fan-out한다. 프론트 `ProtocolClient`는 얇은 carrier(`TauriTransport`)로 축소되며, **컴포넌트가 의존하는 `agentClient` 인터페이스(ADR-0011)는 글자 그대로 유지**한다(구현 위치만 JS→Rust 이동, 호출처 무수정).

이로써 ADR-0020 결정3의 "프로토콜 의미론을 한 곳(JS ProtocolClient)에 모은다"에서 *그 한 곳*이 JS→Rust로 이동한다(0020의 나머지 결정 — 단일 프로토콜·ConnectionCore message-level 추출·embedded racing 직렬화·lease/viewport 우회금지·crate 위치 — 은 유효).

## 거부한 대안
- **B (Rust 1차 + JS 방어적 2차)** — 기존 JS 가드를 2차 방어선으로 보존하고 Rust에 1차 추가. 동일 가드 의미론이 Rust·JS **두 곳에 중복**돼 한쪽만 갱신되는 rot 위험. §5 단일 진실원·"두뇌=백엔드" 기조와 충돌.
- **C (JS 단독 + Rust raw relay)** — Rust는 프레임을 그대로 중계하고 JS가 전부 처리. Rust가 어느 창으로 보낼지 알려면 프레임을 **부분 디코드**해야 하고, 가드가 **창마다 N회** 실행된다 → ADR-0036의 "단일 연결·1회 처리" 이점을 반감시켜 통일의 목적과 충돌.

## 근거
§5(두뇌=백엔드, 프론트=순수 I/O 렌더링) + ADR-0036(통일 relay, 라우팅 전 1회 처리). 단일 연결이 모이는 길목에서 1회 거르는 것이 N창 fan-out 모델의 자연스러운 가드 위치다 — 라우팅 후(B/C)면 창 수에 비례해 중복/N회가 된다. ADR-0011의 facade 인터페이스 경계는 손대지 않으므로(인터페이스 ≠ 구현) "ProtocolClient 박막화"는 ADR-0011 위반이 아니라 구현 위치 이동이다.

## 영향 / 불변식
- **ADR-0011 인터페이스 불변** — 컴포넌트·스토어는 `agentClient`만 의존(무수정). 바뀌는 건 메서드 내부 구현 locus(JS→Rust)뿐.
- **ADR-0020 R2(seq dedup/high-water — `lastDeliveredSeq` 기준, replay_from으로 기준 안 건드림)·R3(epoch 가드/Resume — resubscribe epoch=null 금지)를 Rust `protocol_state`가 보존 이식.** TS 테스트 2파일(`src/api/wsTransport.test.ts`·`protocolClient.test.ts`, 40+케이스)이 Rust 이식 명세서.
- **동시성-치명** — ADR-0001(kill 2동사)·0005(finalize 1회)·0006(락 순서)·0007(epoch)의 데몬 소유분은 클라 무관(클라는 Kill 전송+Ack·seq dedup만). `OutputRouter` 갱신은 ViewManager 락 드롭 후(ADR-0006 보존).
- **부속 내부 결정(D2~D5, spike §5):** D2 `ProtocolClient`≈`TauriTransport`(D1=A 종속) · D3 라우팅 carrier=Tauri Channel(emit_to 거부, 멀티윈도우 per-window 안전성 context7 확인 + QA full 실측 게이트) · D4 JS InProc mock 폐기(목적이 Rust headless unit T3로 이전) · D5 `commands/discovery.rs` ensure_lock 일단 보존→T2/T5 단일 DaemonClient 확인 후 dead면 제거.
