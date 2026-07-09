# 핸드오프: 리로드 두절 = ADR-0040/0043 내 '버그'로 재규정 (데몬 재-hydration은 기각 대안) + A(구조화 저장)·B(리로드) 묶음 PRD 계획

## 한 줄 상태 · 다음 첫 액션
이 세션은 **전부 설계 논의(코드 변경·커밋·테스트 0)**. 리로드 두절 픽스 방향을 OSS 리서치+실제 ADR 정독으로 **재규정**했다 — 핵심 성과이자 정정: **지난 핸드오프가 밀던 "데몬 재-hydration/재요청"은 ADR-0040이 이미 명시적으로 기각한 대안**이고, 리로드 두절은 **확정 아키텍처(ADR-0040/0043) 안의 구현 버그**다(재설계 아님). 사용자가 A(구조화 저장)+B(리로드 버그) **묶어서** 진행 승인 + "구조 결정만 올리고 나머지 자율" 위임.
**다음 첫 액션 = 승인된 플랜 ①부터 재개:** JSON 모드 배선 상태(ADR-0044 + 파싱/저장 코드) 수집 → A 스코프 확정. (이번에 ① Explore 스폰하려다 사용자가 세션 종료 선택해서 중단됨.)

## ★ 지난 핸드오프(latest 02:36) 정정 — 꼭 읽을 것 ★
- 지난 핸드오프는 "정공 픽스 = 데몬 재-hydration(`Subscribe{after_seq:None}` from-oldest 재요청) → ADR-0040 개정"이라 했다. **이건 틀렸다.**
- **ADR-0040 원문이 "새 창마다 데몬에 재요청"을 거부한 대안으로 명시**한다 — 사유: *"사용자 지적 '구조가 이상하다'. 2계층 중계 모델에서 불필요한 네트워크 왕복이고, 재구독 min 합산이 곧 유실 결함의 원인."* 즉 **사용자가 예전에 직접 기각한 방향**을 지난 핸드오프가 되살린 것.
- **왜 재요청이 redundant인가:** src-tauri 공유 버퍼가 **데몬의 bounded 범위(min ~2MB, ~4096건)를 그대로 미러**한다. 데몬에 다시 물어도 같은 범위라 **더 줄 게 없다.**
- **불변식 개정도 불필요:** ADR-0002가 이미 `OutputEvent`를 확장 enum으로 정의(구조화 variant 예약). "파싱해서 구조화 저장"은 **이미 확정 설계**다. 세션 중 내가 "§2 불변식이 파싱을 막는다"고 한 건 내 오독이었음 — 그런 제약 없음.

## 이번 세션이 확정한 것 (두 직교 축)
| 축 | 질문 | 답 (확정) | 근거 |
|---|---|---|---|
| **A — 저장 형태** | raw냐 파싱된 구조화냐 | **파싱된 구조화** `OutputEvent`. 백엔드 어댑터가 SSE/JSON 파싱→타입된 이벤트(TextDelta/ToolCall/Usage)→코어 버퍼 저장→엣지가 capability로 렌더러 선택 | ADR-0002 |
| **B — 재접속 메커니즘** | 리로드 시 어디서 replay | **src-tauri 공유 버퍼에서** 전체 replay (데몬 재요청 X). 리로드=fresh resubscribe→cursor None 리셋→전체 replay | ADR-0040/0043 |

- 스트리밍 저장 세부: 데몬은 payload(JSON 조각) 파싱값이 아니라 **프레임된 이벤트 단위**(seq+타입)로 보관. JSON "끝"은 파싱이 아니라 `content_block_stop` 같은 **명시 이벤트**로 안다. 터미널(바이트)↔API(구조화 이벤트)가 **같은 저장 모델**(seq 달린 `OutputEvent`), 종류만 다름 = ADR-0002 그대로.

## B (리로드 두절) 픽스 — 어떻게 (승인된 접근)
**추측 금지, 계측 먼저(ADR-0038).** 지난 세션의 코드↔실측 충돌(코드는 "fresh resubscribe가 전체 replay해야"인데 실측은 두절)이 아직 미해결이라 **근본원인부터 못박는다.**
1. **재현+계측:** cdp(`scripts/cdp.mjs`)로 재현(에이전트 출력 쌓고 웹뷰 리로드) + `RUST_LOG=debug`로 **4후보 손실지점** 특정:
   - ① teardown에 버퍼 드롭 (old 창 슬롯 0→`unsubscribe`/`drop_agent`) — `output_view_store.rs`
   - ② fresh 전체 replay 미트리거 / 그때 버퍼 빔 — `agent.rs:141 subscribe_output`→`output_view_store.rs:341 resubscribe_slot`
   - ③ replay 계산됐는데 새 Channel 미등록이라 배달 드롭 (deliverable 게이트 race) — `connection.rs:1175 ReplaySlots arm` + ADR-0043
   - ④ 배달됐는데 프론트 seq dedup/epoch가 버림 — 프론트 `[agentId,epoch]` 재구독 + seq dedup
2. **확정 지점서 수정 (ADR-0040/0043 안, 새 아키텍처 없음):** ①→teardown을 진짜 unsubscribe와 구분해 버퍼 유지 / ③→Channel 등록이 replay보다 먼저 순서보장 / ④→프론트 dedup·epoch 수정. 코더(opus)→`/review code`→`/qa`.
3. **검증:** cdp 리로드→스크롤백 복원 + `cargo test`/`npm test` 회귀.
- (원인이 ADR-0043의 진짜 설계 공백으로 판명되면 → 그때만 0043 부분 개정 = **구조 결정이니 사용자에게 올릴 것**. 일단은 구현 버그 가정.)

## A (구조화 저장) — 스코프 미확정 (① 수집 필요)
- ADR-0002가 **설계는 확정**했으나 "지금 무엇을 구현하나"는 **현재 JSON 모드 배선 상태**에 달림 — 최신 **ADR-0044("JSON 모드 배선·StdioTransport 신설·바이트 통로·공용 지속 프로세스")**가 어디까지 왔나를 봐야 함.
- **① 다음 세션 첫 수집(Explore) 프롬프트 골자:** ADR-0044 전문(결정+거부대안), 현재 JSON 모드 출력이 raw 바이트로 흐르나 구조화 파싱이 있나(transport→OutputEvent 경로), `OutputEvent` variant 구현/예약 상태, `capabilities.output` 값·설정 위치, 프론트 RenderMode/DomSlot 렌더러 선택, **A의 실제 gap(있는 것 vs 없는 것)**. "지금 파싱할 라이브 구조화 스트림이 있나, 아니면 JSON 모드가 바이트 통과 중인가"를 명확히.

## 승인된 진행 플랜 (사용자 위임: 구조 결정만 올림, 나머지 자율)
```
① JSON 모드 상태 수집(A 스코프) → ② A+B 묶은 PRD 초안(docs/process/SN-<slug>/<slug>-prd.md)
→ ③ /review prd 적대검증(놓친 대안·race·confident-wrong) → ④ 최종 보고 → (사용자 OK)
→ ⑤ 구현(코더 opus → /review code → /qa)
```
- 사용자 원문 위임: **"전체적인 구조 결정할때 말고는 너가 알아서 해."** → 진행 중 굵은 설계 갈림길만 사용자에게, 수집·계측·구현·검증은 서브에이전트로 자율.

## 검증 상태 (쌍)
**돌린 것:** 없음 — 이 세션은 순수 설계/리서치. 코드 변경·테스트·빌드·커밋 **0**.
**검증 안 된 것 (오신뢰 금지):**
- 리로드 버그 **근본원인 미확정** (4후보 중 어느 것인지 미계측; 코드↔실측 충돌 지속).
- **A 스코프 미확정** (① JSON 모드 수집 안 됨 — Explore 스폰 직전 세션 종료).
- OSS 리서치 결론(아래)은 medium+Codex 검증됨이나, **engram 코드 적용 결정은 위 미계측에 의존**.

## OSS 리서치 결과 (이번 세션 완료 — medium, cross-family Codex 검증. 재조사 금지, 참고만)
- **서버 authoritative bounded 버퍼 + 재연결 replay가 규범** (tmux grid·Zellij Grid·VS Code ptyHost·Eternal Terminal). → ADR-0040의 "src-tauri가 데몬 bounded 범위 미러하고 로컬 서브" 모델을 뒷받침.
- **빈 뷰어 재접속(리로드)=bounded 스냅샷**(tmux/VS Code), delta 아님. delta(mosh/Kafka/ET)는 위치 보유한 consumer용.
- **rendered vs raw 저장:** 터미널-전용(tmux·Zellij·mosh·VS Code 1.60+)은 서버가 파싱해 **렌더 그리드/상태** 보관. ET·Kafka·Redis는 **raw+seq**. engram은 ADR-0002대로 **파싱된 구조화 OutputEvent**(백엔드 어댑터가 파싱) — 저장은 파싱된 형태가 맞다. (VS Code "10MB TerminalRecorder"는 1.60 이전 서술 = Codex가 강등.)
- Codex 적대리뷰 판정 = FIX (적출: Eternal Terminal 누락·VS Code 세부 과장·"리로드=cold캐시" 논리공백). 이 대화 히스토리에 findings 전문.
- research 스킬 feedback: 없음(잘 돌았음).

## 실패한 접근 (do-not)
- **"데몬 재-hydration/재요청" 재추적 금지** — ADR-0040 기각 대안(redundant: src-tauri가 같은 bounded 범위 미러). 지난 핸드오프가 틀림.
- **§2 불변식 "파싱 금지" 오독 재발 금지** — ADR-0002가 구조화 파싱을 이미 의도.
- **OSS 리서치 재실행 금지** — 이번에 medium+Codex로 끝냄.
- **리로드 버그 솔로 추측 금지** — 계측 먼저(ADR-0038).
- **SendMessage 툴 이 하네스에 없음** — 서브에이전트 이어받기 불가, 매번 새로 스폰.
- **`/implement` 스킬 디스크에 없음** — 수동 코더→review→qa.

## 수집한 핵심 ADR 사실 (재수집 불필요)
- **ADR-0002** (output-event-seam): `OutputEvent` 확장 enum, `TerminalBytes` 유일 구현 variant, API=`TextDelta`/`Usage`/`ToolCall`(예약·주석만). `capabilities.output`=terminal_bytes/markdown/tool_events/usage. 불변식 "출력은 종류를 가정하지 않는다(터미널 강제 금지)"=**다형적**(type-blind 아님).
- **ADR-0030**: capability = `compose(TransportCaps, BackendCaps)`. 소유권 타입 강제.
- **ADR-0040** (출력 관리 단위): src-tauri 공유 버퍼가 데몬 bounded 범위 `min(~2MB,~4096건)` 미러, 각 View는 read-index만 독립, 에이전트당 데몬 **한 번만** 구독, 새 창은 **데몬 재요청 없이** 버퍼에서 채움. **거부: "새 창마다 데몬 재요청"(사용자 '구조 이상').**
- **ADR-0041**: 데몬 `Subscribe`/`Unsubscribe`는 src-tauri **layout 델타 단독 소유**. 프론트 subscribeOutput=렌더러 등록만. `forward_daemon_command`가 프론트발 Subscribe 차단.
- **ADR-0043** (mount-replay): actor 경유 직렬. **배정 트리거**(fresh:false)=subscribe(cursor 없을 때만 신설+replay). **등록 트리거**(webview reload, fresh:true)=`resubscribe_slot`(cursor None 리셋→전체 replay). **deliverable 게이트**: cursor advance ⟺ Channel delivery.
- **ADR-0044** (최신, next=0045): JSON 모드 배선·StdioTransport 신설 — **전문 미수집**(① 대상).
- 코드: `OutputEvent` = `crates/engram-dashboard-core/src/agent/types.rs:18`. 백엔드 어댑터 = `agent/backend/mod.rs`(backend_for/build_command_spec/capabilities). 버퍼 = `src-tauri/src/daemon_client/mod.rs:147`(buffer_store)+`output_view_store.rs`. 리로드 경로 = `agent.rs:141`→`connection.rs:1175 ReplaySlots`→`output_view_store.rs:341 resubscribe_slot`.
- PRD/spec 경로 패턴: `docs/process/SN-<slug>/<slug>-{prd,trd}.md`. step-log 포맷: `## SN — 제목 (날짜)` + 무엇/어떻게/결과/문서/후속. ADR 다음 번호 = **0045**.

## repo 상태
- **HEAD = `1a4e76a` (master), 미푸쉬. 작업트리 clean.** 세션 시작과 동일 — 이 세션 커밋 0.
- 선재 fmt drift: `crates/engram-dashboard-daemon/tests/ws_e2e.rs:23` (내 밖, CRLF 아티팩트 의심) → `cargo fmt --check` FAIL. 안 건드림.

## 참조 (읽을 것만)
- ADR: `docs/decisions/` 0002·0030·0040·0041·0043·0044·0038(결함 OSS 선조사).
- 코드: 위 "수집한 ADR 사실"의 file:line.
- 지난 핸드오프 `latest.md`(02:36) = **이 문서가 픽스 방향에서 정정함**(데몬 재-hydration 부분 폐기).
