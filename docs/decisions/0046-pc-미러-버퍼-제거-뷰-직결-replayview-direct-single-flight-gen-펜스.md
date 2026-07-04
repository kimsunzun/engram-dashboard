# ADR-0046: PC 미러 버퍼 제거 — 뷰 직결 replay(view-direct) + single-flight gen 펜스

- 상태: 확정 (2026-07-05, 근거: medium 리서치(grounding+Codex 적대) + `/review trd deep` 3라운드 + 사용자 결정)
- 관련: Supersedes ADR-0040 · Amends ADR-0037 (dedup/진도 거처 조항: Rust 단독 → 웹뷰 뷰 단위) · Amends ADR-0007 (프론트 epoch 권위 조항: SubscribeAck 단독 → src-tauri decide_epoch 1차 필터 + 필터된 frame/마커 채택) · Amends ADR-0043 (deliverable gate 메커니즘: 미러 cursor 게이트 폐기 → 뷰 buffering phase + gen 펜스로 대체) · ADR-0029(데몬 데이터 소유) · ADR-0041/0042(BLOCK-1·axis A 계승) · TRD `docs/process/S16-view-direct-replay/view-direct-replay-trd.md`(rev4) · 리서치 2026-07-05

## 맥락
src-tauri 중계 허브가 데몬 ring을 미러(`AgentBufferStore`/`OutputViewStore`, ADR-0040)하고 per-view cursor로 fan-out하는 구조에서 동기화 버그가 3건 연속 발생했다: 리로드 replay 유실(ca3f325), split/remount 고착(23a8c47 resync_output 증상 대응), 다중 slot 공유 유실(버그 B — lastDeliveredSeq가 agentId 단위). 셋 다 "데몬이 이미 가진 데이터를 클라가 한 벌 더 미러하고 그 둘을 맞추려다" 생기는 한 계열이다. ADR-0040은 미러 유지를 "메모리 중복 ~2MB라 실익 작음"으로 정당화했으나 실제 비용은 메모리가 아니라 **동기화 복잡도**였음이 실증됐다. 사용자 결정(2026-07-05): PC(loopback)는 미러 제거·데몬 직수신, 모바일/원격만 캐싱(transport seam 분기).

## 결정
1. **미러 전면 제거.** src-tauri = 무상태 라우터(frame 헤더만 읽고 targets∩registered 창 Channel로 통과). `AgentBufferStore`/`OutputViewStore`/`BoundedSeqLog`/per-view cursor/`resync_output` 폐기.
2. **remount/리로드/재연결 = 데몬 ring 전량 재replay.** wire는 에이전트당 구독 1개 유지, 뷰 mount마다 `Subscribe{after_seq:None}` 재발행(데몬 R1: Ack→replay→ReplayComplete FIFO 무변경). 이미 라이브인 뷰는 뷰별 seq dedup로 중복 흡수.
3. **진도 상태의 유일한 거처 = 웹뷰 뷰(slot) 단위 `lastDeliveredSeq`.** src-tauri에 출력 진도(seq/cursor/버퍼) 금지 — 허용 상태는 요청 부기(epoch·single-flight)뿐.
4. **replay 경계 = Channel 내부 마커(tag=255) + single-flight gen 펜스.** src-tauri가 에이전트당 in-flight 1개(sent→acked→completed 수명, 진행 기반 deadline, 대기열 병합)로 Subscribe↔ReplayComplete를 1:1 대응시켜 gen을 각인. 뷰는 자기 requestReplay가 반환한 gen 이상의 성공 마커에만 sort+dedup flush. 상세 상태기계 = TRD rev4 §2.
5. **갭 정책: 보관 하한부터 replay(현행 의미 유지).** 하한 너머 복원은 비목표 — 대화 이력의 정본은 claude 세션(`--session-id`/`--resume`, ADR-0008)이 이미 보존하고, 에이전트 재시작 시 claude가 이력을 재출력해 새 epoch ring에 다시 쌓인다.
6. **모바일/원격 캐싱 = transport 구현 내부에만 허용(자리만).** ProtocolClient 인터페이스 불변. **주의: gen 펜스의 정합 전제 = 모든 replay가 full-from-oldest.** 원격 seam에서 after_seq 부분 resume을 도입하는 순간 이 전제가 깨지므로, 그때 뷰별 마커 상관(정확 gen 일치 등)을 먼저 강화해야 한다.

## 거부한 대안
- **미러 유지 + 동기화 정교화** — 버그 3건의 원인 구조에 한 겹 더 얹는 것(resync_output이 그 사례). 동기화 버그의 답은 동기화 제거지 정교화가 아님(사용자 명시 do-not).
- **wire per-view 구독(frame에 sub_id 프로토콜 확장)** — 사용자 체감 동일한데 frame 헤더·codec·데몬·양단 재작업 파급이 큼. loopback 중복 전송은 뷰별 dedup가 공짜로 흡수.
- **단일 구독 + 뷰별 one-shot catch-up 병합** — replay→live 경계 병합이 뷰마다 필요해 현 버그 계열 복잡도가 잔존(사용자가 O1 선택, 2026-07-05).
- **데몬측 화면 스냅샷(VS Code serialize 방식)** — 데몬에 터미널 에뮬레이터급 기능 추가 = 범위 폭증. 잘린 과거의 실체(대화 이력)는 claude 세션이 이미 정본으로 보존해 이중 투자(사용자 결정).
- **마커 gen의 셈 기반 결합(마지막값 각인/FIFO 카운팅)** — 리뷰 실증 격파: 마지막값은 남의 Complete에 새 gen 오각인(조기 flush 유실), 카운팅은 Complete 미도착 실패 경로에서 영구 desync→무한 재요청. single-flight 1:1 대응으로 대체.
- **절대시간 deadline** — healthy-slow replay(디버그 빌드·경합)에서 오탐→후속 Complete가 다음 gen에 오각인. 진행 기반(frame/Ack마다 리셋) + acked 게이트로 대체.
- **src-tauri에 after_seq 재개용 seq 보관(재연결 최적화)** — 결정 3 위반(진도 상태 재도입). 재연결 전량 재replay 회귀는 PC loopback·재연결 희소로 수용, 원격은 seam에서 해결.

## 근거
- **리서치(2026-07-05 medium, grounding+Codex 적대 리뷰):** 서버(데몬) 정본 + 얇은 클라가 업계 일반(tmux/zellij/VS Code — VS Code는 동형 3계층에서 중간 계층에 5ms 배칭 송신 버퍼만, 미러 캐시 없음·소스 확인). 클라 캐시는 원격/오프라인 사유(wezterm mux 예측 캐시·Matrix 모바일 DB)로만 두는 게 일반. 단 최종 논거는 선례가 아니라 **자체 비용 비교: 미러 동기화 복잡도(버그 3건 실증) > loopback 중복 전송(~2MB 상한)**.
- **`/review trd deep` 3라운드(opus doc-aware ×3 + Codex blind):** 데몬 outbound 단일 FIFO(text/binary 동일 큐, ws.rs:519)·재구독 직렬화(dispatch 직렬+락 안 동기 replay) 코드 실측 확인. 1~2라운드 BLOCK(마커 무상관→조기 flush 유실, 셈 결합 desync)을 gen 펜스→single-flight로 해소, 3라운드 FIX 반영(acked 게이트·진행 deadline·단절 시 마커 미발행·full-replay 전제 명문화).

## 영향 / 불변식
- **삭제:** `output_channel.rs` 미러부·`output_view_store.rs`·`output_view_buffer.rs`·`resync_output`·`ReplaySlots`/`DropSlots`·프론트 `pendingBuffers`/`resubscribeAll`. `ViewLayoutRenderer` 중복 워크어라운드 제거(버그 B 근본 해소).
- **BLOCK-1 전면화(ADR-0041 강화):** 프론트는 wire Subscribe/Unsubscribe를 어떤 경로로도 안 보냄(기존 재연결 예외 삭제). wire 구독 형성 = request_replay 단독, 정리 = 라우터 Unsubscribe 단독.
- **데몬 무변경** — R1·frame 포맷·codec 그대로(에이전트당 단일 무손실 스트림·viewer 불가지 보존).
- **재연결 = 전량 재replay(회귀 수용)** — 기존 tail-resume 대비 대역 증가. PC 수용 근거는 위. 원격 carrier 도입 시 seam에서 재설계.
- **mid-replay drop(데몬 outbound 포화 Error) = 현행 동등 한계** — 현행도 미소비. 데몬 Error 귀속화 후 실패 마커 승격은 백로그.
- 구현 게이트: 코더 high · `/review code deep` · `/qa full`(cdp: 버그 B 재현 케이스 포함). 신규 코드에 `// ADR-0046` 앵커.
