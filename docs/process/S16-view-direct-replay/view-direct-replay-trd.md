# S16 TRD — PC 미러 버퍼 제거: 뷰 직결 replay (view-direct replay) — rev4

> **왜/무엇 = ADR-0046(ADR-0040 supersede + 부분 supersede 목록 §7)** + 리서치(2026-07-05 medium). 이 문서 = **어떻게**. 리뷰 이력: rev2 = `/review trd deep`(BLOCK 2·FIX 1) → gen 펜스. rev3 = 재검증(BLOCK) → **single-flight 1:1 결합**(셈 기반 결합 금지 — 마지막값 각인·FIFO 카운팅 둘 다 유실/desync 실증). rev4 = 3차(FIX) → acked 게이트·진행 기반 deadline·단절 시 마커 미발행·full-replay 전제 명문화.
> **확정 결정(사용자, 2026-07-05):** ① 뷰별 직구독(각 뷰가 데몬 ring에서 직접 replay 수신, src-tauri = 무상태 통과) · ② 갭 정책 = 보관 하한부터 replay(하한 너머 복원 비목표 — 이력 정본 = claude 세션 `--resume`) · ③ 모바일/원격 캐싱 = transport seam 자리만 · ④ 게이트 통과분 단계별 커밋(push 없음).
> **내부 결정(메인 — 아침 보고 대상):** ⓐ wire 다중화 = 에이전트당 wire 구독 1개 + 뷰 mount마다 전량 재replay + 뷰별 dedup(frame sub_id 프로토콜 확장 거부 — 체감 동일, 파급 큼). ⓑ **재연결 = 전량 재replay 수용**(기존 tail-resume 대비 회귀 맞음 — PC loopback·재연결 희소라 수용, 원격은 seam에서 after_seq resume으로 해결. §4). ⓒ replay 경계 = Channel 내부 마커 + gen 펜스(리뷰 3인 수렴 지시).

## 1. 목적·범위

src-tauri의 데몬 버퍼 미러(`AgentBufferStore`/`OutputViewStore`/`BoundedSeqLog`/per-view cursor)를 **전부 제거**, remount/리로드/새 창 = **데몬 ring 전량 재replay**로 대체. 진도 상태의 거처가 "웹뷰 뷰 단위" 한 곳이 되어 미러↔데몬↔화면 3자 동기화가 소멸한다. `resync_output`(23a8c47 증상 대응)도 흡수·폐기.

**범위 밖:** 데몬측 화면 스냅샷(거부, ADR 기록). 모바일 캐싱 구현. JSON 렌더 잔여(S15 DEFER — 별도 스텝). mount 폭주 coalescing(§4 — 선택 최적화로만 기록, v1 미구현).

## 2. 데이터 흐름·프로토콜 (재설계 후)

```
데몬 ring(ReplayBuffer, 유일 보관자) ── R1: Ack→replay→ReplayComplete 단일 FIFO
   │  (리뷰 실측 확인: ws.rs:519 단일 mpsc, text/binary 동일 큐 → 상대 순서 보존.
   │   재구독은 dispatch 직렬 + subscribers lock 안 동기 replay → 동시 replay 절단 없음)
   ▼ WS 단일 연결 (에이전트당 wire 구독 1개)
src-tauri: 무상태 라우터 + replay gen 채번자
   request_replay(agent) command: gen+=1 채번 → wire Subscribe{agent, epoch:known, after_seq:None} → gen 반환
   binary arm: frame 헤더(agentId·epoch)만 읽고 → epoch 필터(★재배선, §3 T5) → targets∩registered 창 Channel로 원본 bytes 통과
   text arm:   SubscribeAck → apply_subscribe_ack(epoch 갱신) + truncated 기억
               ReplayComplete{agent,epoch} → 마커 frame(tag=255) 합성: {agentId, epoch, gen, truncated} → 같은 Channel로 송신
   ▼ Tauri Channel (창당 1개, wire frame 그대로 + 마커) — ★Channel 순서 보존 = 명시 가정, 테스트 고정(§5)
웹뷰: ProtocolClient 뷰(slot) 단위 구독 + gen 펜스
   subscribeOutput(viewId, agentId, cb) → SubState 등록(동기) → requestReplay(agentId) → myGen 회수
   phase: buffering(frame 축적) → 마커(gen ≥ myGen && epoch 일치)에만 sort+dedup flush → live(직행+뷰별 dedup)
```

**gen 펜스 + single-flight 결합 (rev3 — 결합 방식 확정):** 뷰는 자기 `requestReplay`가 반환한 `myGen` **이상**의 성공 마커에만 flush — 남의(이전) replay 마커 조기 flush → 자기 replay 머리 dedup 유실(버그 B 재유입 경로)을 차단. 늦게 mount한 뷰의 버퍼 = 이전 replay 꼬리 + 자기 replay 전체 → sort+dedup flush = 완전. gen은 **Channel 내부 계약**(데몬 protocol 무변경).

**gen↔ReplayComplete 결합은 셈(카운팅)으로 추론하지 않는다** — 재검증에서 두 소박한 구현이 모두 격파됨: ⓐ 마지막값 각인 = 남의 Complete에 새 gen이 찍혀 조기 flush 유실 재발, ⓑ FIFO 카운팅 = Complete가 안 오는 실패 경로(agent 소멸 시 send_error로 Ack/Complete 자체 생략 — connection_core.rs:1041-1051·1080-1088)에서 한 칸 영구 desync → 모든 후속 뷰 flush 불가 → 무한 재요청 폭주. 대신 **에이전트당 single-flight**로 1:1 대응을 강제한다:

- src-tauri per-agent 상태: `{gen_counter(단조, 리셋 안 함), in_flight: Option<{gen, deadline}>, waiters(대기열)}`.
- `request_replay(agent)`: in_flight 없음 → gen 채번·Subscribe(None) 송신·in_flight 설정, gen 반환. in_flight 있음 → **다음 1회 Subscribe에 병합**(대기열의 모든 요청 = 같은 다음 gen 반환 — 요청은 항상 뷰 SubState 등록 후 도착하므로 "그 뒤에 송신되는 Sub"에 병합해도 안전. rev2의 coalescing 안전조건이 구조로 충족됨).
- **in_flight 수명 = sent → (SubscribeAck) → acked → (ReplayComplete) → 마커.** Complete는 **acked 상태의 in_flight에만** 각인한다 — wire 순서가 [Ack_k]…[Complete_k][Ack_k+1]이므로, Ack 전에 도착한 Complete는 증명 가능하게 전대(前代)의 고아 → 무시(오귀속 원천 차단, rev4). 각인 시 **성공 마커(gen)** 송신·in_flight 해제 → 대기열 있으면 다음 Subscribe 송신.
- **deadline = 진행 기반(rev4):** 절대 시간이 아니라 **그 에이전트의 frame/Ack 수신마다 리셋** — 건강한 느린 replay(디버그 빌드·경합)가 스트리밍 중인 동안은 절대 트립되지 않는다. 진행 없이 deadline 초과(agent 소멸·subscribe 실패로 Ack/Complete 자체가 안 옴): **실패 마커(gen, flags.failed)** 송신·in_flight 해제·다음 진행. (절대 deadline은 healthy-slow replay에서 오탐 → 후속 Complete가 다음 gen에 오각인되는 desync를 만든다 — 재검증 NEW-A로 기각.)
- **연결 단절 시(rev4):** in_flight·대기열 **내부 클리어만** — 실패 마커를 웹뷰로 보내지 않는다(재요청 구동자는 connected 전이 단독 — 이중 구동·outage 중 사다리 소진 방지). gen_counter는 단조 유지(구세대 마커 오인 방지).

**replay 트리거 = 뷰 주도 일원화:** wire Subscribe(None)를 보내는 곳은 `request_replay`가 유일. ① 뷰 mount/remount → requestReplay ② 창 리로드 → 각 뷰가 mount하며 requestReplay(subscribe_output은 Channel 등록만) ③ WS 재연결 → 프론트가 `daemon-connection-state` connected 전이에서 mounted 뷰 전원 buffering 리셋 + requestReplay(= 마커 재장전, buffering 고착 자가 복구). src-tauri 재연결 훅의 eager 재구독·seq 읽기 삭제. 라우터는 **Unsubscribe(더 안 보이는 agent 정리)만** wire에 보낸다.

**뷰 상태전이표 (F1 — 코딩 전 고정):**

| 상태 | 이벤트 | 전이·동작 |
|---|---|---|
| buffering | frame(epoch=대기 epoch 또는 미정) | buffer에 push(상한 §4, 초과 시 drop-oldest) |
| buffering | frame(epoch 더 높음) | buffer 폐기 → 새 epoch로 buffering + **requestReplay 재발행(새 myGen)** — 구 epoch 대상이던 기존 myGen 마커는 무효(재검증 NEW-5) |
| buffering | 마커(gen<myGen 또는 epoch 불일치) | 무시(남의/구세대 replay) |
| buffering | 마커(gen≥myGen, epoch 일치, flags.failed) | flush 금지 — **buffer는 유지한 채**(sort+dedup가 중복 흡수, 폐기 불필요) 재요청 사다리(아래 bounded 규칙) |
| buffering | 마커(gen≥myGen, epoch 일치, 성공, **token 현재**) | sort+seq dedup flush → live(epoch 채택, lastDeliveredSeq=꼬리) · truncated면 경고 표면화 |
| buffering | 마커(token 불일치 — StrictMode 사망 구독) | 무시(생존 구독의 마커만 유효) |
| buffering | **마커 도착 시 myGen 미확정**(invoke 응답이 Channel보다 늦는 파이프 교차 — 재검증 NEW-3) | 마커를 버리지 않고 **최고 gen 마커 1개 보관** → myGen 확정 시 재평가 |
| buffering | watchdog 만료(예: 10s) | **flush 금지** — requestReplay 재발행(새 myGen). 부분 flush 금지 |
| buffering | buffer 상한 초과 | buffer 폐기 + requestReplay 재발행(부분 flush 금지) |
| buffering | connected 전이(재연결) | buffer 폐기 → requestReplay 재발행 |
| live | frame(epoch 일치, seq>lastDeliveredSeq) | 전달 + 진도 갱신 |
| live | frame(epoch 일치, seq≤) | dedup drop(중복 replay 흡수) |
| live | frame(epoch 더 높음) | drop — epoch 전환은 기존 `[agentId, epoch]` remount 흐름이 처리(remount→새 구독→replay가 전량 재전달) |
| live | 마커(어떤 gen이든) | 무시(fan-out으로 도달하는 남의 replay 경계 — live 뷰는 dedup만으로 충분) |
| live | connected 전이(재연결) | buffering 리셋 + requestReplay |
| any | unsubscribe(viewId, token 일치) | SubState 제거 |

**재요청 사다리(bounded — 재검증 NEW-4):** watchdog/실패 마커/상한 초과에 의한 requestReplay 재발행은 **시도 상한(3회) + 지수 백오프**. 소진 시 뷰를 명시적 에러 상태로 전이(무한 폭주 금지)하고 슬롯에 표면화 — 이 에러 상태는 §5 LLM 제어 표면에서도 조회 가능해야 한다. `[agentId, epoch]` remount·connected 전이는 사다리를 리셋한다.

## 3. Seam별 설계

### 데몬·protocol (무변경)
`connection_core.rs` handle_subscribe(sink 교체+원자 스냅+R1)·frame 포맷·codec 전부 그대로. 데몬 불변식(에이전트당 단일 무손실 스트림, viewer 불가지) 보존.

### src-tauri
| # | 파일 | 변경 |
|---|---|---|
| T1 | `output_channel.rs` | `AgentBufferStore`·`build_deliverable`·`flush_snapshot` 제거. `WindowChannelRegistry`(창→Channel) 존속, 죽은 Channel = send 실패 시 제거 유지 |
| T2 | `daemon_client/connection.rs` | binary arm: 버퍼·cursor 제거 → **epoch 필터(T5) 통과분만** targets∩registered 창에 원본 frame 통과. text arm: SubscribeAck→epoch 갱신+truncated 기억, ReplayComplete→**gen 각인 마커** 합성을 **binary frame과 같은 Channel::send 경로로** 송신(★app.emit 경유 금지 — 순서 붕괴, 리뷰 finding). `ReplaySlots`/`DropSlots` arm·`resubscribe_and_sweep`의 seq 읽기·eager 재구독 삭제 |
| T3 | `commands/agent.rs` | `subscribe_output` = Channel 등록만. `resync_output` → **`request_replay(agent_id) -> gen`**(§2 single-flight: 즉시 송신 또는 다음 Sub에 병합, gen 반환). `forward_daemon_command`의 Subscribe/Unsubscribe 차단 존속(BLOCK-1) |
| T4 | `output_router.rs` | `SubscriptionDelta` axis B 제거. wire엔 **Unsubscribe만** 라우터가 발행(prune). targets() 라우팅 존속 |
| T5 | `daemon_client/protocol_state.rs` | epoch 추적 존속하되 **"존속"이 아니라 재배선**: 현 binary 핫패스는 decide_epoch를 안 부름(epoch 필터가 미러 on_frame에 접혀 있었음 — 리뷰 실측). 새 binary arm이 `decide_epoch`(SubState.epoch, Ack로 갱신)를 직접 호출해 stale epoch frame을 통과 전 drop. per-agent **single-flight 상태**(`gen_counter`·`in_flight{gen,deadline}`·대기열·`truncated`) 추가 — **단절 시 in_flight·대기열 실패 처리 후 클리어, gen_counter는 단조 유지**(재검증 NEW-2). (§7 상태 허용 범위) |
| T6 | `crates/engram-dashboard-core` | `output_view_store.rs`·`output_view_buffer.rs` 삭제(+테스트) — 미러 전용, 타 사용처 없음(배선 지도 + 리뷰어 grep 재확인) |

### 프론트 (TS)
| # | 파일 | 변경 |
|---|---|---|
| F1 | `api/protocolClient.ts` | `subs` 키 viewId. `SubState{agentId, phase, buffer[], myGen, lastDeliveredSeq, epoch, token}` + §2 전이표 구현. **token 가드를 마커 도착 시점에 재평가**(등록 시점 아님 — 리뷰 finding). frame→같은 agent 뷰들 fan-out. `pendingBuffers`·`resubscribeAll`(wire Subscribe 송신 경로) 삭제 — 유일 호출처 onConnectionStateChange뿐임을 리뷰어 확인. connected 전이 = 뷰 buffering 리셋+재요청으로 대체 |
| F2 | `api/transport.ts`·`tauriTransport.ts` | `requestReplay(agentId) -> Promise<gen>`. 마커(tag=255)는 **transport가 정규화해 제어 이벤트로** ProtocolClient에 전달(공개 agentClient API에 마커 개념 노출 금지 — Designer finding). frame decode에 **미지 tag 관용 skip** 가드(M0) |
| F3 | `components/slot/*` 구독 effect | `subscribeOutput(viewId, agentId, ...)` 시그니처. `[agentId, epoch]` effect·`terminal.reset()` 구독 전·기존 micro-rules 유지 — epoch 전환 재장전이 이 effect에 걸림(race 렌즈 #4: 재시작 복구의 유일한 re-arm이므로 vitest로 고정) |
| F4 | `ViewLayoutRenderer.tsx` | 61-63 dedup workaround 제거 |

**마커 frame 규격(Channel 내부 계약):** `[tag=255][agentId:16][epoch:4][gen:8][flags:1(bit0=truncated, bit1=failed)]`. 데몬 codec 미정의 — src-tauri↔웹뷰 계층 문서화. 미지 tag는 웹뷰가 조용히 skip(전방 호환). 모든 `request_replay`는 **최소 1개의 마커(성공 또는 실패)로 종결**된다 — Complete 수신(성공) / deadline 초과·단절(실패). "정확히 1개"는 아니다: 좀비 late-Complete에서 같은 gen의 실패 마커(deadline) 뒤에 성공 마커(늦은 Complete)가 뒤따르는 failed→success 쌍이 가능하며, gen 펜스가 이를 흡수한다(뷰는 실패 마커를 사다리로 넘겼다가 뒤이은 성공 마커에 flush). 이 결정성이 프론트 상태기계의 전제다.

## 4. 볼륨·트레이드오프·리스크

- **순 감소:** 미러+테스트 −600 LOC급(core/src-tauri), 프론트 ±0~+150(전이표 기계 추가 vs pendingBuffers/resubscribeAll 삭제).
- **재연결 회귀(명시 수용 — 아침 보고):** 기존 = tail-resume(after_seq), 신규 = 뷰당 전량 재replay. PC loopback·재연결 희소로 수용. 원격/모바일 carrier는 transport seam에서 after_seq resume+캐싱으로 해결(이 TRD 비범위). **src-tauri에 seq를 남기는 절충은 §7 위반이라 거부.**
- **mount 폭주:** single-flight가 구조적으로 coalescing한다(in-flight 중 도착한 요청 전부 → 다음 Sub 1회에 병합) — N뷰 동시 remount ≤ 2회 replay.
- **뷰 buffering 상한:** ring 상한의 2배(4MB/8192) per view — 버퍼가 "이전 replay 꼬리 + 자기 replay 전체"를 담을 수 있어야 하므로(Codex 재리뷰 #5). 초과 시 **부분 유지(drop-oldest) 금지** → buffer 폐기 + 재요청(사다리 적용). 병리 케이스 방어용이며 정상 도달 불가.
- **mid-replay drop(데몬 outbound 포화로 replay 일부 유실 — connection_core.rs:1105 Error) = 현행 동등 한계:** 현행도 이 Error를 소비하지 않는다(emit_broadcast가 버림 — 리뷰 실측). 신설계도 v1 동일(성공 마커 후 gap 가능, 발생률 극저). 데몬 Error 귀속화(요청 상관) 후 실패 마커로 승격은 백로그로 기록.
- **고위험 seam:** T2 main_loop(락 규율 — 락이 줄어드는 방향) · 마커 동일 Channel 경로 강제 · F1 전이표(StrictMode·epoch 회전·재연결 교차) → 코더 high · `/review code deep` · `/qa full`.

## 5. 테스트 (TDD, ADR-0012)

- **rust(src-tauri):** 라우팅 순수부(targets∩registered) · **single-flight 상태기계**(in-flight 1개 강제·대기열 병합·**acked 게이트: Ack 전 도착 Complete=고아 무시**·Complete→성공 마커 1:1·**진행 기반 deadline**(frame/Ack마다 리셋, healthy-slow replay 무오탐)→실패 마커·**Complete 누락(agent 소멸)에도 desync 없음**·단절 시 내부 클리어(마커 미발행)+gen 단조 유지) · 마커가 replay 꼬리 뒤 + **binary와 동일 Channel 경로**(R1 상대 순서 fixture) · epoch 필터 재배선(stale epoch frame drop) · truncated/failed 플래그 전파.
- **vitest(F1) — 리뷰 지적 시나리오 전부 고정:** ★**엇갈린 mount(진행 중 replay 꼬리만 받은 뷰가 남의 마커 무시 → 자기 gen 마커에 완전 flush)** · 같은 agent 2뷰 fan-out(버그 B 회귀) · live frame이 replay보다 먼저 와도 sort+dedup 복원 · 마커 token 불일치 무시(StrictMode) · **마커가 myGen 확정보다 먼저 도착(보관→재평가)** · epoch 회전 중 buffering(폐기+재요청, 구 epoch 마커 무시) · 재연결 중 buffering(폐기+재요청) · 실패 마커→사다리(상한 도달 시 에러 상태) · watchdog = 재요청이지 flush 아님 · 뷰별 dedup 독립 · unsubscribe 청소.
- **Channel 순서 가정:** tauriTransport 단위(모의 Channel 순서) + cdp 실측으로 이중 고정.
- **게이트:** `cargo build`·`cargo test --workspace --exclude engram-dashboard`·`cargo test -p engram-dashboard --no-run`·`cargo fmt --check`·코어격리 rg 0줄·`npx tsc --noEmit`·`npm test`.
- **cdp 실측(완료 기준):** 단일 slot remount 3종(23a8c47 케이스) · **동일 agent 2슬롯 공유+리로드(버그 B 재현) 복원** · split 직후 즉시 재split(엇갈린 mount 실촉발) · tag1 리로드 복원 · 재연결(클라 재시작) 복원 · 에이전트 재시작(epoch 전환) 복원.

## 6. 구현 순서 (모듈별 커밋)

0. **M0 전방 호환 가드:** 프론트 frame decode 미지 tag 관용 skip(+test) — M1을 프론트 교체 전에 올릴 수 있게.
1. **M1 src-tauri 전환(T1~T5):** 미러 제거 + request_replay/gen + 마커 + epoch 필터 재배선 + rust 테스트.
2. **M2 프론트 per-view(F1~F4):** 전이표 + vitest.
3. **M3 잔재 정리(T6):** core 미러 파일 삭제 · 구 anchor 갱신(protocolClient.ts:479 epoch 권위 주석, connection.rs ADR-0037 dedup 주석 등 — §7 supersede와 짝) · `// ADR-0046` 부착.
4. **M4 cdp 실측** → step-log·문서 정리.

## 7. 불변식·ADR 파급

- **BLOCK-1 전면화:** 프론트는 wire Subscribe/Unsubscribe를 어떤 경로로도 안 보낸다(resubscribeAll 예외 삭제). wire 구독 형성 = request_replay 단독, 정리 = 라우터 Unsubscribe 단독. **단, BLOCK-1(프론트 wire Subscribe 금지)은 src-tauri 허브 경로(TauriTransport) 한정** — direct-daemon carrier(WsTransport, legacy/test 전용)는 1연결=1구독이라 storm 전제가 없어 자체 wire Subscribe로 replay를 형성한다(운영 carrier 아님, clientFactory.ts:24).
- **src-tauri 상태 허용 범위(재도입 방지선):** per-agent `epoch`(Ack 권위) + single-flight 부기(`gen_counter`·`in_flight`·대기열·`truncated`) = **요청 부기(bookkeeping)**로 허용 — 전부 "진행 중 요청"의 수명만 갖고 단절 시 클리어(gen_counter만 단조 유지). **출력 진도 상태(seq/cursor/버퍼) 금지** — 진도의 유일한 거처 = 웹뷰 뷰 단위 `lastDeliveredSeq`.
- **R1(데몬) 무변경.**
- **★gen 펜스의 정합 전제 = "모든 replay는 보관 하한부터 전량"(full-from-oldest).** "gen≥myGen 마커에 flush"가 안전한 이유는 같은 에이전트의 후속 replay가 항상 이전의 누적 상위집합이기 때문이다. **원격 seam에서 after_seq 부분 resume을 도입하는 순간 이 전제가 깨진다** — 그때는 뷰별 마커 상관(정확한 gen 일치 등)을 먼저 강화해야 한다(재검증 NEW-C, ADR에 명시).
- **ADR 파급(신규 ADR에 명시 — 리뷰 지적):** ADR-0040 **전체 supersede** · ADR-0037 "dedup/전송 의미론 Rust 단독 진실원" 조항 **부분 supersede**(dedup 거처 = 웹뷰 뷰 단위로 이동) · ADR-0007 "프론트 epoch 권위 = SubscribeAck 단독" 조항 **부분 supersede**(src-tauri decide_epoch 1차 필터 + 프론트는 필터된 frame/마커 epoch 채택) · ADR-0043 deliverable gate 메커니즘 **폐기 반영** · ADR-0041/0042 axis A는 계승(형태 변경: eager→view-driven).
- 모바일/원격 캐싱 = transport 구현 내부만 허용(ProtocolClient 인터페이스 불변) — 자리만.
