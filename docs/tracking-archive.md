# 추적 항목 — 해소·종결 기록

tracking.md 에서 해소·종결된 항목 중 다른 문서·코드가 번호로 가리키는 것만 한 줄씩 둔다. 전문 = git 이력(간소화 전 = `git show 8671e5c:docs/tracking.md`).

- **T-50** 덮는 층의 「줄 바깥 빈 영역」 — 종결: 층 높이 = 그려진 알림 줄만큼을 사용자가 확정했다(줄 밖의 클릭은 그 아래로 간다 · 상한 값 1/3 은 여전히 세션 판단) · 2026-10-08 · ADR-0287 결정 2 · ADR-0276 · ADR-0277 · `docs/process/S21-storage/trd.md` §14-16 · §14-18
- **T-48** 다른 연결 사건도 덮는 층에 넣을지 — 종결: 넣는다 — 팝업 아닌 알림(띠)은 전부 덮는 층에 쌓는다(일시적 끊김 · 재시도 중 · 유실 알림 · 명령 하나 실패 포함 · 띠별 세부는 transport crate 부착 TRD 몫) · 2026-10-08 · ADR-0287 결정 1 · ADR-0277 · `docs/process/S21-storage/trd.md` §14-17 · §14-18
- **T-46** 실 claude 의존 테스트 6건 — CI 커버리지 0 — 해소: CI 0 을 받아들이고(ci.yml `--skip` 그대로) 로컬은 관측 실패를 통과가 아니라 실패로 낸다 · 예외 = `c1_park_then_spawn_auto_delivers` 는 `#[ignore]`(입력을 주지 않아 핵심 단언이 매번 건너뛰어졌다 — 그 축은 `c2_busy_recipient_parks_*` 가 덮는다) · claude 부재는 스폰 가드가 못 잡고 뒤의 단언이 실패한다 · `ENGRAM_TEST_REQUIRE_CLAUDE` 스위치 폐지 · 2026-10-02 · `crates/engram-dashboard-daemon/tests/control_send.rs` 머리말 · `.github/workflows/ci.yml` 의 같은 주석 · 옛 번호 = T-16
- **T-24** `ENGRAM_EXE` 미사용 주입 — 해소: 데몬 부팅의 주입을 걷었다(리포 밖 소비자 없음 — 사용자 확인) · 2026-10-02 · step-log 이 번호로 가리킨다 · 형제 `locate_send_exe`(`ENGRAM_CLI_EXE`)는 그대로
- **T-38** 사용량 한도 슬롯 ADR 열 건 코드 앵커 — 해소: 0246–0251·0253–0256 앵커 추가 · 2026-10-02 · step-log 이 번호로 가리킨다 · `v0.3.3/feat/chat-ux` 기록의 「T-38」은 다른 주제다(= T-44)
- **T-40** claude 훅 도중 끊기가 약 300 초 멈춘다 — 해소: 우리 Job 안의 끊기 뒤 훅 잔여물만 끝내고 claude 는 살린다(GUI 실측 G1 11/11 · 사용자 최종 테스트) · 2026-10-01 · ADR-0262(ADR-0257 대체) · `docs/process/S21-chat-ux/trd-t40.md`
- **T-34** 중단(interrupt) 기능이 반쯤 배선된 채 멈춰 있었다 — 해소: 채팅 칸 Esc → `agent.interrupt` · claude JSON 끊기 줄 · 2026-09-28 · ADR-0237/0238/0245 · 트리 노드 `canInterrupt` 는 읽는 화면 없이 둔다
- **T-18** branch protection(초록 아니면 머지 금지) — 종결: 도입 안 함(사용자 결정) · 재론 트리거 = 협업자 증가 또는 CI 초록 없이 들어간 변경의 사고 · 도입하면 ADR-0232 를 먼저 다시 연다 · ADR-0131
- **T-10** discovery crate 통합 — 종결: 안 한다(사용자 결정 2026-08-26) · 데몬도 그 crate 를 의존하고 async 무의존 게이트가 거기 붙으며 `base` 셋째 입주자 문제(ADR-0175)가 걸린다 · 남은 동기(`src/lib.rs` 크기)는 파일 분할로 · 다시 엶: ADR-0271(2026-10-02 — discovery 를 나눠 데몬 · 셸로 · 작업 추적 = T-47 작업 순서 2-2) → 2-2 착지 2026-10-07 — crate 삭제(ADR-0282 · step-log)
- **T-8** shutdown_all 순차 종료 지연 — 해소: scoped thread 로 병렬 kill · 각 kill 은 `join_pump(5s)` 로 유계라 N×5s 누적 없음 · `crates/engram-dashboard-agent/src/manager.rs` 의 `shutdown_all`
- **T-7** get_agent_snapshot wire 포맷 — 종결: 문제의 불일치가 사라졌다 — 명령은 남아 있으나(`AgentCommand::GetSnapshot` · `protocolClient.getSnapshot`) live 출력은 이제 base64 없는 바이너리 프레임(protocol codec)으로 가고, `getSnapshot` 은 테스트 밖 호출자가 없다(`rg getSnapshot src -g '!*.test.*'` → 선언·구현뿐 · 2026-10-02)
- **T-5** monaco TS worker optimizeDeps — 종결: monaco 의존을 걷었다(2026-09-25) · 다시 들이면 `optimizeDeps.exclude` 에 worker 추가 검토
- **T-4** 프론트 terminal 판정(`status_changed` 만으로 종료 판정 금지) — T-16 증상 ① 로 흡수 · 불변식 = CLAUDE.md 「상태 알림 분담」 · ADR-0005
- **T-2** 프론트 seq dedup 확인 — 대체: seq 연속(`마지막+1` 만 배달 · 구멍 뒤는 붙든다) · ADR-0231 · `src/api/protocolClient.ts` 의 `HELD_MAX_*`
- **T-1** 로그 API 키 마스킹 — 구현 · 2026-06-11 · `mask_secrets`(`crates/engram-dashboard-base/src/logging/mod.rs`) · sink 에 배선하지 않고 호출자가 명시 호출 · 정본 = `docs/reference/logging-conventions.md` 「보안」 (= D-6)
- **R-1** Exiting 상태 살림 — 결정·구현: kill 맨 앞에서 `Exiting` 설정 + 알림 · 분담 = CLAUDE.md 「상태 알림 분담」 · ADR-0005
- **D-8** 데몬화 IPC(턴키 라이브러리 없음) — 해소: 데몬 구현 · ADR-0028/0029 · 조사 = `docs/process/S12-daemonization/ipc-library-consult.md`
- **D-7** 레이아웃/창 영속화 = 프론트 localStorage — 종결: 레이아웃 저장 시스템은 새로 설계한다(사용자 결정 2026-10-02 — 이 번호를 가리키는 옛 저장·복원 요구도 함께 닫는다) · 저장 위치 결정은 ADR-0035/0057 이 이미 대체(레이아웃 권위 = 백엔드 · 지금은 인메모리뿐)
- **D-4** LLD §6 drain 시그니처 4인자 — `docs/process/S1-design/backend-lld-stage1.md` 반영 · 2026-06-11
- **D-3** LLD §10 Mutex poison = fail-fast — `backend-lld-stage1.md` 반영 · 2026-06-11
- **D-2** LLD §13 JobObjectHandle 분리 API — `backend-lld-stage1.md` 반영 · 2026-06-11
- **D-1** LLD §14 로깅 명세(RUST_LOG · 기본 warn) — `backend-lld-stage1.md` 반영 · 2026-06-11
