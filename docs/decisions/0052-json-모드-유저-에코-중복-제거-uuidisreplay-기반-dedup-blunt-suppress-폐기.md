# ADR-0052: json 모드 유저 에코 중복 제거 = uuid/isReplay 기반 dedup (blunt suppress 폐기)

- 상태: 확정 (2026-07-06, 근거: uuid 왕복 실측 + OSS 조사 + /review code deep PASS)
- 관련: CLAUDE.md §5(LLM-우선 제어)·S10 backend 추상화 · ADR-0044/0045(claude stream-json decoder + 입력-시점 에코) · ADR-0004(backend 격리)·0006(락)·0045/0046(accumulator 멱등·replay→live) · 코드: `crates/engram-dashboard-core/src/agent/backend/claude.rs`(wrap_user_turn·consume_block)·`backend/mod.rs`(input_echo_event)·`agent/session.rs`(write_input)·`src/components/slot/structuredAccumulator.ts`(extractUserUuid·seenUserUuids) · step-log 2026-07-07

## 맥락
json(StreamJson) 모드는 터미널 PTY 같은 로컬 에코가 없다 — 유저가 친 메시지가 claude의 왕복 replay 전까지 화면에 안 뜬다. 체감 반응성을 위해 `write_input` 성공 즉시 합성 유저 이벤트를 emit(낙관적 에코)하는데, claude가 `--replay-user-messages`로 같은 메시지를 되울리면 중복이 된다. 이 중복을 어떻게 제거하느냐가 문제였다. 초기 구현(ADR-0044/0045 후속)은 decoder가 **user-role `type=="text"` 블록을 무조건 억제**해 dedup했다.

## 결정
낙관적 에코와 replay를 **client가 생성한 uuid로 매칭해 그 짝만 dedup**한다:
- `write_input`이 호출당 `Uuid::new_v4()` 하나를 생성해 (a) stdin으로 나가는 user 메시지(`wrap_user_turn`)와 (b) 합성 에코 이벤트 양쪽에 **같은 값**으로 부착.
- decoder(`consume_block`)는 replay된 user 블록을 **억제하지 않고** line-level uuid를 실어 그대로 통과.
- 프론트 `structuredAccumulator`가 **`type=="text"` user 블록만** uuid로 dedup(합성 uuid == replay uuid → 1개). tool_result·비-text·다른 uuid·과거 이력은 항상 보존.
- seam = **S1(프론트 dedup)**: backend 공유 가변상태 0, wire 무변경(uuid는 `Structured.json` 안에 탑승).

## 거부한 대안
- **A. blunt suppress (user text 무조건 억제) — 폐기.** 지금은 동작하나 오직 json 모드 resume 비활성(fresh session 고정) 전제에서만 정확하다. resume/history replay를 켜는 순간 claude가 되울린 **과거 유저 메시지까지 전부 삭제**돼 대화 이력이 화면에서 소실된다. cross-family 리뷰어(Codex)가 BLOCK(HIGH)로 적출 — 정확성이 암묵적 불변식(resume off)에 묶이고 테스트가 그 억제를 "옳다"고 못박아 미래 회귀를 가리는 잠복 함정.
- **B. 백엔드 pending-set (S2) — 거부.** session이 보낸 uuid 집합을 보관하고 decoder가 `isReplay && uuid∈집합`일 때만 억제. 정확하나 session↔decoder 간 공유 가변상태가 필요해 락·격리(ADR-0006/0004) 부담이 커진다. S1이 같은 정확성을 공유상태 없이 달성.
- **C. 낙관적 에코 제거, replay를 진실원으로만 렌더 — 거부.** dedup 로직이 통째로 사라져 가장 단순하나, 전송~표시 사이 왕복 지연이 화면에 노출된다(에코를 넣은 애초 목적을 되돌림). 체감 반응성 유지를 위해 에코 + uuid dedup을 채택.

## 근거
- **실측(확정):** claude를 `--replay-user-messages`로 띄우고 stdin user 메시지에 top-level `uuid`를 심으면, replay된 stdout user 라인이 그 uuid를 **그대로 보존**하고 `isReplay:true`를 단다(이번 세션 직접 관측: uuid `X` 전송 → `X` + `isReplay:true` 회수). 정밀 id-상관이 가능함이 사실로 확인됨.
- **OSS 조사:** 성숙 채팅 시스템(Discord `nonce`·Matrix `txnId`·XMPP `origin-id`)의 표준 = client 생성 correlation-id + pending 매칭이며, "유저 에코 무조건 버리기"는 resume/history/멀티전송에서 깨지는 문서화된 안티패턴. **공식 VS Code Claude 확장(설치본 코드 실측)도 메시지 `uuid`로 dedup + transcript 저장** — 채택안이 공식 도구와 동일.
- **검증:** `/review code deep` 적대 3인 — 초기 구현의 multi-block(같은 uuid의 text+tool_result) 소실 결함을 3인 일치 적출 → dedup을 `type=="text"` 블록에만 한정하는 FIX 후 재리뷰 2인 PASS. full build · core 144+통합 · protocol 42 · vitest 277 · tsc · fmt · 격리 게이트 PASS. uuid 왕복 실측.

## 영향 / 불변식
- **dedup 키 = `type=="text"` user 블록의 client uuid.** tool_result 등 비-text 블록은 dedup 대상 아님(항상 보존) — `extractUserUuid`가 non-text에 `null` 반환. 이 경계를 어기면(uuid 단독 키로 되돌리면) multi-block 라인에서 tool_result가 소실된다.
- **합성 에코와 stdin 메시지는 반드시 같은 uuid** — `write_input`이 한 값을 양쪽 주입. 어긋나면 dedup 실패(이중 렌더).
- **ADR-0004 격리 유지** — claude json 스키마(uuid/isReplay 위치)는 `backend/claude.rs`에만. session/mod는 불투명 `Uuid` 토큰만 전달.
- **ADR-0006 락** — 새 락 0, `core.emit` 재사용. **ADR-0045 accumulator 멱등** — `reset()`이 `seenUserUuids` 포함 초기화 → replay→live rebuild(ADR-0046)에서 재수렴.
- **미확인(동작):** GUI 실측(실제 화면 dedup)은 이 ADR 시점 미수행 — 로직은 리뷰 + 단위테스트 + uuid 실측 프로브로 검증됐으나 end-to-end 화면 확인은 대기. 커밋 `dbced5a`는 로컬 전용(push 보류).
- 관찰(비차단): `seenUserUuids`는 단일 epoch 수명 동안 무한 증가(reset마다 비움, 실사용 저위험).
