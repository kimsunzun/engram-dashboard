# 핸드오프: S14 T6b 출력 평면 구현 완료(미커밋)·/review code deep 3인 → FIX 4건, 굵은 결정 3개 사용자 대기 (dashboard2/master)

> ⚠️ 멀티트랙/저장 위생: `.claude/continue`는 **gitignored(로컬 전용)**. db2=worktree `engram-dashboard`(master), db1=worktree `engram-dashboard-a1`(wip/a1) 분리. 이건 db2 관점.

## 한 줄 상태 + 다음 첫 액션
S14 모듈① **T6b(출력 평면) 구현 완료·테스트 green(미커밋)** + **`/review code deep` 3인 적대 끝 → FIX 4건**. 핵심 fan-out 메커니즘은 3인 모두 "정상" 확인, **결함은 connect/재연결 재동기 시퀀스 + 자원 정리에 집중**. 커밋 안 함 — **굵은 결정 3개(C1 범위·C2 안·C4 seq 시점)가 사용자 결정 대기**(F7, 결정권=사용자).
**다음 첫 액션:** 아래 "굵은 결정 3개"를 사용자가 고른다 → 코더(opus)에게 4건 수정 일괄 지시 → **재리뷰(동시성이라 `/review code deep` 다시)** → `/qa full`(GUI 실측 G2: connect→subscribe_output→assign→출력 도달, `cdp.mjs`) → 커밋. ★C1 미해결이면 GUI 실측은 "connect 먼저→assign" 수동 순서로만 출력 도달(부팅 순서 의존 결함 우회).

## repo 상태 (★미커밋★)
- HEAD = master `723f39b`(T6a), **working tree에 T6b 변경 7파일 미커밋**(유실 주의 — 디스크엔 있음).
- 변경: `connection.rs`(+236, main_loop Binary/SubscribeAck/Subscribe/Unsubscribe/Fire arm·재연결 resubscribe·fan_out/send_fire 헬퍼·ConnectionCommand 재정의) · `mod.rs`(+63, DaemonClient router/registry 필드+생성자·subscribe/unsubscribe/send_fire_and_forget/try_enqueue) · `agent.rs`(subscribe_output·resize 배선) · `layout.rs`(6 mutation rebuild 락안+delta 송신 락밖) · `lib.rs`(router/registry manage·subscribe_output 등록) · `tests.rs`(+155, T6b 4테스트+기존 Subscribe 수정) · `output_channel.rs`(신규, WindowChannelRegistry 타입).

## 검증 상태 (쌍)
- **green(미커밋):** 전체 `cargo test --workspace` = src-tauri lib **145**(기존 141+신규 4) / core 67+통합 / daemon 35+ws_e2e 44 / discovery 44 / protocol 26+golden — **0 failed**. 메인이 직접 재실행 verify함. `cargo fmt --check` 0. core `use tauri` 0(격리 유지). 재실행 = `cargo test --workspace`.
- **검증 안 됨(중요):** ① **green ≠ correct 재확인** — 145 green인데 아래 C1/C2/C4를 테스트가 못 잡음(적대 리뷰가 적출). ② **G2(Channel::send from tokio task → 실제 창 도달) GUI 미실측** — 단위 불가, `/qa full` cdp.mjs 영역. ③ **C1(connect 후 재구독) 단위·GUI 양쪽 미검증**(opus#2 표기 지적).

## /review code deep 결과 — FIX 4건 (3인: Codex blind + opus doc-aware ×2)
> 메인 의심(재연결 resubscribe 무필터)이 **3인 모두 확인**. Codex가 C4를 신선 단독 적출. 무해 확인(3인): fan-out 메커니즘·decide_output 가드 순서·fan_out의 registry.lock 보유 중 await 0(ADR-0006)·layout rebuild 락안/송신 락밖·원본 frame fan-out correctness·subscribe wire epoch/after_seq·테스트 비-vacuous.

- **C1 [HIGH] connect 후 재구독 트리거 누락(끊긴 고리).** connect/start_connection 어디에도 연결 후 `router.rebuild`→`subscribe` 트리거 **없음**. 비연결 중 layout 변경→`try_enqueue` no-op→connect 후 `subs` 빈 채 시작→resubscribe no-op→**그 agent 영영 미구독→출력 0**. 주석("다음 connect 시 layout이 rebuild→resubscribe")이 **존재하지 않는 경로를 가정**(거짓). 기존 `protocolClient.ts:72-79`는 connected watch에서 `resubscribeAll()` 직접 호출 — 그 동등 트리거 누락. **현재 프론트=wsTransport 직결이라 당장 안 터짐, T7 cutover 시 출력 전멸 시한폭탄.** opus#1 "BLOCK 직전".
- **C2 [HIGH] 재연결 resubscribe 무필터(메인 의심 확인).** `connection.rs` main_loop 진입 resubscribe가 `subs` **전체** 재구독. Unsubscribe는 `subs`에서 제거 안 함(F-B) + `router.targets().is_empty()` 필터 없음 → 안 보이는 agent도 재연결마다 재구독. 화면 정확성 무해(fan_out이 빈 targets로 막음), but 유령 구독·트래픽·죽은 agent_id Subscribe 폭증.
- **C3 [MED] subs 무한 증가.** Unsubscribe/kill/close 어디서도 `subs` 제거 0 → 메모리 단조 증가(10년 전제 누적). C2와 같은 뿌리. 주석은 "정리는 후속" 인지.
- **C4 [MED] fan_out 실패/registry 미등록 시 last_delivered_seq 전진(Codex 단독).** `decide_output`이 Deliver 판정 시 seq를 이미 전진(`protocol_state.rs:175`). 그 후 fan_out에서 Channel 없거나 send 실패해도 "배달됨" 기록 → 재구독 after_seq가 미전달 frame 건너뜀. SubState 주석("실제 배달한 최고 seq")과 불일치. **registry race**: layout assign(subscribe)이 창 mount(subscribe_output)보다 먼저면 frame 유실+seq 전진.

## ★굵은 결정 3개 (사용자 — 임의 확정 금지)★
1. **C1 connect 후 재동기 seam:** (A) **지금 T6b에서 배선** — connect 성공 직후 LayoutState 잠가 `router.rebuild`→delta subscribe. 단 DaemonClient↔LayoutState seam 설계 필요(connect 완료를 command 레이어가 받아 rebuild). / (B) **T7로 미루고 거짓 주석만 정직화** — T6b는 출력 평면 자체만, connect 재동기는 T7 cutover.
2. **C2/C3 재연결+메모리:** (A) **`router.targets` 필터만**(resubscribe에 `if targets.is_empty(){continue}`) — F-B 무손실 유지, 메모리 누수 잔존. / (B) **Unsubscribe arm에서 `subs.remove`** — 메모리도 해결, 단 재구독=FromOldest 전체 replay(dedup이 거름)라 F-B "tail Resume 무손실" 일부 포기.
3. **C4 seq 전진 시점:** (A) **현행 유지**(가드 통과 시 전진) + 데몬 ring replay 안전망 신뢰 + 주석 정직화. / (B) **fan_out 성공 후로 분리** — ADR-0037 "가드 라우팅 전 1회" 의미 변경 영향 검토 필요. / 또는 **registry race만 별도**(창 mount 순서 보장 = T7 ordering).

## 실패한 접근 / do-not (carry-forward + 신규)
- **green ≠ correct (재재확인):** T5 RMW race·T6a 버퍼 drain에 이어 T6b도 145 green인데 적대 리뷰가 HIGH 2건 적출. **동시성 변경은 `/review code deep` 3인 유지** — Codex blind가 C4를 단독 적출(opus 2인은 놓침), 메인 의심을 opus 2인이 확인. cross-family 다양성 효과 실증.
- **Codex 모델명 생략**(`mcp__codex__codex` model 미지정), config `model_reasoning_effort:high`만. sandbox read-only·approval never로 호출.
- **cargo 동시 실행 금지**(빌드락). test/build 순차. tauri dev도 cargo.
- **출력 박스/와이드 아스키 금지**(사용자 터미널 깨짐). LF→CRLF git 경고 정상.
- **Explore/코더 자기보고 불신** — 매 라운드 working tree 직접 검증(이번에도 메인이 7파일 diff 전부 verify, C2 의심 직접 발견).

## 참조 (읽을 것만)
- 정본: `docs/process/S14-multi-page-layout/module1-transport-spike.md` §8(T5)·§9(T6 G1~G3) · `trd.md`.
- 코드(미커밋): `src-tauri/src/daemon_client/connection.rs`(main_loop arm·resubscribe 루프=C1/C2 자리·fan_out=C4) · `mod.rs`(try_enqueue=C1 뿌리·subscribe/unsubscribe) · `commands/layout.rs`(rebuild+delta) · `output_channel.rs`(registry) · `protocol_state.rs`(decide_output=C4).
- ADR: 0006(락순서)·0035(레이아웃권위)·0036(전송중계)·0037(전송의미론 Rust 단독 가드)·0007(epoch).

## a1 트랙 (dashboard1, worktree `engram-dashboard-a1`, wip/a1 — 만지지 말 것)
- a1=메시징 data-plane(목표 ⑤) 연기. 공통 파일(SlotPane·SlotContextMenu)=모듈③, 건드리기 전 db1 핑.

## 정리 사항
- 떠 있는 앱/데몬 없음(포트 9223 free). G2 GUI 실측 시 새로 띄움.
- 리뷰 서브에이전트(opus reviewer-deep ×2) ID: `ab69797d1626413dd`, `ab4e8125ec739bef1` — 필요 시 SendMessage로 추가 질의 가능. Codex thread: `019f0f77-c799-7c80-9266-29c8d51ed563`.
