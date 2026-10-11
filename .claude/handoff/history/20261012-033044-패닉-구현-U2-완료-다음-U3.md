# 핸드오프: 패닉 정책 구현 — U1 · U2 완료(push) · 다음 = U3 훅 설치

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.3.3/refactor/crate-boundaries` · HEAD = 이 핸드오프 커밋(push 완료) · 워킹트리 깨끗. U1 `744c060b`(CI 초록) · U2 `76c8031e`(CI = 이 push 로 뜸 — 확인 안 함).
★다음 첫 액션 = ① 이 push 의 CI 결과 확인(`gh run list --branch v0.3.3/refactor/crate-boundaries --workflow ci.yml --limit 1`) ② U3 착수 — `/implement` standard 이상(훅 설치는 프로세스 수명이 걸려 critical 검토): 데몬 기존 훅(`daemon/src/lib.rs:129-151` · 설치 `:458`)을 base `install_hook` 으로 **교체**(겹치지 말 것 — 사유 줄 두 번) · 셸 `run()` 머리(`src-tauri/src/lib.rs:28`)에 설치 · TRD §5 U3 확인 거리(debug 데몬이 tokio 가 격리하던 패닉에 끝난다 — `usage_service/mod.rs:320-322` · `agent_conn.rs:108` → `/qa full` · `stop_smoke` 먼저) · 자식 프로세스 시험(TRD §6 — 데몬 crate · 선례 없는 새 꼴).★
사용자 지시(2026-10-12): 「쭉쭉 진행」(자율 모드) · ★단위마다 로컬 커밋, push 는 적당한 때 몰아서★ · master 머지 · 태그는 사용자 확인.

## U2 결과
- `76c8031e` — agent 펌프 셋(PTY · stdio · codex) · 라이터 drain = `contain`(finish 는 밖 · 펌프별 실프로세스 시험으로 「finish 표시 없음」 확인) · `uncontained` 넷(codex 첫 턴 sink · report_usage · on_written · claude decoder 문) · 데몬 허용 목록 `PANIC_CONTAINED`(agent.list · agent.listQueuedInputs) — 표 호출 + 본문(`call_and_drive`)을 가둠(리뷰어 대립 → 안전 쪽 잠정 채택: codex = 넓혀라 / Claude = 인자 변환은 효과 없어 수용 가능 · 사용자 재판정 거리).
- 게이트: `/review code full` 2 라운드 PASS · `/qa standard` PASS(59 · 4360 · 0 · 30 · 실 claude 6 제외 · lib_unit 826 · 격리 전부 · tsc · npm 2021).
- 결정(메인): 라이터 drain 바깥 `tracing::error!` 유지(라이터 스레드 이름에 agent id 가 없어 훅 줄만으론 귀속이 안 선다 · contain doc 에 「귀속은 남긴다」 단서).
- ★사고 1건★: U2 데몬 코더의 1 라운드 시험이 필터 없이 돌아 실 claude 시험 5개가 실행됐다(코더 자진 보고 · 영향 미확인). 이후 코더 · QA 지시서에 CI `--skip` 6건 명시. ★다음 코더 · QA 지시서에도 반드시 명시★(U1 QA 의 워크스페이스 회귀도 `--skip` 없이 돌았을 수 있다 — 무시 31 vs U2 30). CLAUDE.md 는 「실 claude 시험 = 로컬 몫」이라 적지만 이전 핸드오프 주의는 「금지」 — 다음 세션이 사용자에게 확인할 것.
- U4 낡은 주석 목록(코더 · 리뷰어 수집): `pty.rs` 펌프 주석 「(b) tokio 격리」(U3 뒤 거짓) · `input_queue.rs` OnWritten doc · drain doc 「unwind 빌드에서만」 · 시험 doc · `codex/transport.rs:~84` · `~2380` · `command/src/route.rs:17-19` · `command_delivery.rs:1562` · `:1581` 의 없는 `run_locally`(→ `spawn_local`) · `reaper.rs:65-70` · base `sync.rs` 계약 3 · 페이로드 사본(command crate 는 의존 0 이라 유지).

## U1 결과
- `744c060b` — base `panic`(contain · uncontained · is_contained · Caught · payload_text · hook · install_hook(Once · 미호출)) · logging 재진입 표지 + 패닉 중 파일 락 상한 대기(try_lock 100 ms — codex 리뷰 지적) · 게이트 ③ 정규식 `panic`.
- 게이트: `/review code full` 2 라운드 PASS · `/qa standard` PASS(59 · 4352 · 0 · 31 · base 91 · lib_unit 826 · 격리 전부 · tsc · npm 2021).
- U3 볼 거리(리뷰): 데몬 기존 훅은 base 훅 아래 겹치지 말고 **교체**(겹치면 사유 줄 두 번) · install_hook 은 첫 이전 훅을 영구히 잡는다. 페이로드 사본 교체 대상 = agent pty.rs:174 · stdio.rs:175 · codex/transport.rs:1476 · input_queue.rs:336 · reaper.rs:213 · leftover.rs:897 + daemon lib.rs:139(command crate 사본은 의존 0 이라 못 바꾼다).

## 확정된 결정 (정본 = `docs/decisions/0292-*.md` · TRD `docs/process/S21-panic-policy/trd.md` 11판 개정 4 + 리뷰 3 라운드 PASS)
- 가두지 못한 패닉 = 사유 로그 → `std::process::abort`. 정상 종료 꼬리 · 락 오염 정책 없음.
- 가두는 곳 = 에이전트 펌프 셋(PTY · stdio · codex 리더) + 입력 라이터 drain + **명시 허용 목록** 명령 둘 `agent.list` · `agent.listQueuedInputs`(`usage.get` 은 제외 — 조회를 띄우고 in_flight 를 세운다).
- 가둠 안 표시 끔(`uncontained`) 자리 넷: codex 첫 턴 sink · `report_usage` · 라이터 `on_written` · claude decoder 문 조작.
- 불변식: 표시 범위 ⊆ catch 클로저(`finish` 는 표시 밖). 셸 = 훅만. 고지 = 지금 표면(`Failed` · 입력 거절 · `INTERNAL`) + 로그.
- 펌프 패닉 뒤 CLI = 기존 정리 경로(사용자 결정 A) — U4 에서 펌프 종류별 실프로세스 시험, 실패하면 멈추고 사용자에게(B안 = platform `GroupRef::terminate` + ADR-0199 예외 — 혼자 정하지 말 것).
- 조사 = `docs/research/panic-practice-peers-2026-10-12.md`(우리 「즉시 종료」는 과민하지 않다 · 넓게 가두기가 복잡도의 원인).

## 이 세션에 한 것
- A 단계(데이터 파일 통일)를 master 에 머지 · push(`783a4738` · CI 초록) — 패닉 문서는 안 실음.
- 패닉 TRD 11판 새로 씀 → 사용자 결정 ②(좁게) · (마) N2a · (사) abort · A안 → `/review trd full` 3 라운드(1 FIX·FIX → 2 PASS·FIX → 3 PASS).
- master · `origin/v0.3.3/feat/panic-policy`(ADR-0288~0290) 를 이 브랜치로 머지(`909f1684` · `175df7c7`).
- ADR-0292 박제(0288 · 0290 부분 개정 도장) · `/review doc light` FIX 5 반영(`40fa214a` · 재리뷰 생략 — 도장 · 영향 줄 정정뿐).
## 남은 단위 (TRD §5)
U2 가두는 자리(A1~A4 `contain` · 표시 끔 a~d · B1 허용 목록 + 시험 · `command_delivery.rs:1397` 주석) → U3 훅 설치(데몬 · 셸 · 확인 거리 = debug 데몬이 tokio 격리 패닉에 끝남 — `/qa full` · `stop_smoke` 먼저) → U4 확인 · 감사(CLI 정리 실프로세스 시험 · 낡은 주석) → U5 `Cargo.toml:40` 한 줄. U2 agent 몫과 daemon 몫은 파일이 안 겹쳐 나란히 가능.

## do-not / 주의
- 재리뷰는 리뷰어를 새로 띄우지 말고 resume(Claude = SendMessage · codex = `--resume` + 첫 줄에 「직전에 한 일」 확인).
- 이전 핸드오프 주의 그대로: Git Bash `rg` 없음(PATH 추가) · 줄끝 파일마다 · `sed -i` 금지 · 실 claude 시험 금지 · `taskkill /T` 금지.

## 정지 조건
- 리뷰 BLOCK · 리뷰어 정면 대립(자율 모드는 무거운 쪽 잠정 채택 + 기록) · 재수정 상한 초과 · U4 CLI 정리 시험 실패(B안) → 멈추고 사용자에게.
- master 머지 · 태그.
