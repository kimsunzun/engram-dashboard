# 핸드오프: S14 모듈① T6a(cmd 평면) 완료·push — 다음 = T6b(출력 평면, 동시성-치명) (dashboard2/master)

> ⚠️ 멀티트랙/저장 위생: `.claude/continue`는 **gitignored(로컬 전용)** — git에 안 올라간다(정정: 옛 핸드오프의 "git-tracked·머지로 덮임"은 사실 아님). db2=worktree `engram-dashboard`(master), db1=worktree `engram-dashboard-a1`(wip/a1)로 **작업 디렉토리가 분리**돼 각자 `.claude/continue`를 가지므로 서로 안 덮인다. 이건 db2 관점. 진짜 기록 = 이 history 파일(로컬 append-only) + 코드/문서의 git 커밋 이력.

## 한 줄 상태 + 다음 첫 액션
S14 모듈① 전송 재배치: **T5(OutputRouter)·T6a(cmd 요청/응답 평면) 완료·push** (HEAD `723f39b`, origin 동기, tree clean). 다음 = **T6b(출력 평면, 동시성-치명)** — 데몬 출력 바이트가 실제로 창 슬롯까지 닿게 배선. 이게 끝나면 목표 데모 ③(슬롯에 에이전트 화면)이 화면으로 완성된다.
**다음 첫 액션:** `docs/process/S14-multi-page-layout/module1-transport-spike.md` **§9 "T6 시퀀스"** 정독 → T6b = (a) `connection.rs:668` Binary arm route(`decode_frame→decide_output→Deliver면 router.targets()→창 Channel fan-out`, raw `Response`) (b) window Channel registry(`Arc<Mutex<HashMap<label, Channel<Response>>>>` in AppState) (c) `subscribe_output` invoke(창 mount 등록) (d) `commands/layout.rs` 전 mutation에 **rebuild-under-lock**(D3/FIX-1) + unlock 후 delta→`Subscribe`/`Unsubscribe`(fire-and-forget) (e) AppHandle emit 배선(broadcast 이벤트). **미해결 G1/G2/G3 먼저 해소(아래).** 코더(opus)→`/review code deep`→`/qa`(GUI 실측 G2 포함).

## repo 상태
- HEAD = master `723f39b`, **origin/master 동기(push 완료)**, **working tree 깨끗**.
- 이번 세션 커밋(전부 push): `b4e286e`(tauri 2.11.2→2.11.3 데드락수정) · `3a7c7d6`(carrier 리서치+spike §7) · `f81a098`(T5 TRD §8) · `a14ba59`(T5 OutputRouter) · `66ac66f`(T6 TRD §9) · `723f39b`(T6a cmd 평면).

## 검증 상태 (쌍)
- **green·커밋됨:** 전체 workspace `cargo test`(src-tauri lib **141** = T5 17 + T6a + 기존 / 전 멤버 0 failed) · `cargo fmt --check` 0 · core `rg "use tauri" crates/engram-dashboard-core/src/`(주석 1줄 외 실 import 0) · tauri 2.11.3 build green. 재실행 = `cargo test`(워크스페이스 루트).
- **검증 안 됨(중요):** ① **T6a invoke(agent_spawn 등)가 실제 프론트에서 동작하는지** — 프론트는 아직 `wsTransport`로 데몬에 직결(lib.rs 주석), invoke 경로 미연결. T7 cutover 때 실측. ② **G2: `Channel::send`를 tokio connection task에서 호출해 출력이 실제 창에 도달**하는지 GUI 미실측(T6b의 `cdp.mjs` 게이트). ③ `agent_resize`는 현재 **의도적 no-op**(warn 로그) — Resize가 fire-and-forget(request_id 없음)이라 T6b의 fire-and-forget 송신 경로 대기.

## 결정된 것 (이번 세션)
- **tauri 2.11.2 → 2.11.3**(채널-data 데드락 수정 흡수). 최신 2.x.
- **carrier(D3 외) = Tauri Channel, raw byte**(`Channel<tauri::ipc::Response>`/`InvokeResponseBody::Raw`) — `Channel<&[u8]>`는 Serialize라 JSON으로 샘(리서치 적대검증). 이벤트(emit)는 레이아웃 control만.
- **T5 내부(D1~D6):** WindowId=window label(String) · rebuild-always · D4 carrier=per-window Channel(태그) · 정리(send Err→sink 제거 / 창 close→unsubscribe+unlisten #15583).
- **F-A=T5 단독 먼저 · F-B=구독 union layout 파생**(별도 ref-count 맵 없음 — View=화면뿐+데몬 출력보관이라 ②명시카운터는 무이점·YAGNI, ADR-0035 정합). 확장(비화면 소비자=메시징)은 "layout집합 ∪ 별도구독자"로 나중에.
- **★D3 수정(T5 리뷰 FIX-1):** rebuild를 **ViewManager 락 보유 중** 호출(직렬화·ABA 차단) — 순수계산+lock-free store라 ADR-0006 위반 아님, delta **송신만** unlock 후. (옛 "emit_after_unlock 후 호출"은 비원자 RMW race로 폐기.)
- **T6 forks A1~E1**(spike §9) + **T6 슬라이스 = T6a(cmd)/T6b(출력)**.

## T6b 미해결 (코더 진입 전 해소 — spike §9 G1~G3)
- **G1:** delta→`Subscribe` 변환 시 `epoch`/`after_seq`는 connection task의 **`SubState`**(protocol_state, T3)에서(`resubscribe_params` 재사용: 신규=None 전체replay/재구독=tail). 변환을 **task 안**에서 SubState 조회로 채운다(layout 커맨드가 아니라).
- **G2:** `Channel::send` from tokio task 안전성 — 문서상 가능성 높음, 미검증 → 첫 출력 도달 `cdp.mjs eval` 실측이 T6b GUI 게이트.
- **G3:** registry `Arc`를 connection task에 주입(`start_connection` 인자 추가). label("main"/"slot-popup") ↔ ViewManager `window_bindings` label 일치 확인.

## 실패한 접근 / do-not (carry-forward + 신규)
- **green ≠ correct (재확인):** T5에서 `rebuild` RMW race가 테스트 green인데 동시성 결함이었음(리뷰가 적출). **동시성 변경은 `/review code deep`(opus+Codex+reviewer-deep 3인) 유지** — deep 적대 검증이 opus 단독 PASS를 두 번 뒤집음(T6a 버퍼-cmd drain·재연결 엣지).
- **Explore/코더 자기보고 불신:** T6 Explore가 protocol `Resize`/`Subscribe`에 `request_id` 있다고 **환각**(실제 없음=fire-and-forget) → 코더가 실제 `protocol/messages.rs` 읽어 정정. **매 라운드 working tree 직접 검증**(이번 세션 전부 verify함).
- **Codex 모델명 생략** — `mcp__codex__codex` 호출 시 `model` 미지정(gpt-5.2-codex 미지원), config는 `model_reasoning_effort`만.
- **cargo 동시 실행 금지**(빌드락). tauri dev도 cargo라 cargo test와 겹치면 안 됨(순차).
- **출력 박스/와이드 아스키 다이어그램 금지** — 사용자 터미널에서 깨짐(한글+박스문자 정렬 붕괴). 단순 텍스트/들여쓰기·`->` 화살표로.
- LF→CRLF git 경고는 정상(무시).

## 참조 (읽을 것만)
- 다음 작업 정본: `docs/process/S14-multi-page-layout/module1-transport-spike.md` **§8(T5 TRD)·§9(T6 TRD·forks·G1~G3)** · `trd.md`(수용기준).
- 코드: `src-tauri/src/output_router.rs`(T5 targets/rebuild) · `src-tauri/src/daemon_client/connection.rs`(main_loop — cmd/Text arm 배선됨, **Binary arm = `:668` TODO(T5/T6b)**) · `daemon_client/mod.rs`(send_command·owned runtime) · `commands/agent.rs`(invoke, resize no-op) · `commands/layout.rs`(`emit_after_unlock` — rebuild 배선 자리, **단 D3=락 안에서 호출**) · `layout/manager.rs`(ViewManager) · `lib.rs`(manage·invoke 등록).
- 리서치: `docs/research/tauri-channel-multiwindow-carrier-research-2026-06-28.md`.
- ADR: 0035(레이아웃권위=src-tauri)·0036(전송중계통일)·0037(전송의미론=Rust)·0006(락순서)·0011(agentClient seam=에이전트 명령 전용).

## a1 트랙 (dashboard1, worktree `engram-dashboard-a1`, 브랜치 wip/a1 — 만지지 말 것)
- a1 = 메시징 data-plane(목표 ⑤), **연기**. 단방향 메일박스로 좁히면 경계 BLOCK 우회 가능(오너 확인 대기).
- **공통 파일(SlotPane 2곳·SlotContextMenu)은 모듈③** — 건드리기 전 db1 핑(합의됨).

## 정리 사항
- 세션 시작 시 떠 있던 tauri dev 앱(포트 9223)·데몬·cargo 종료함 → **현재 떠 있는 앱 없음**(포트 9223 free). T6b GUI 실측 시 새로 띄우면 됨.
