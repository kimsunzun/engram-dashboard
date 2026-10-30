# 핸드오프: 패닉 정책 구현 — U3 · U4-C 완료(push · CI 초록) · U4-T 사용자 결정 대기

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.3.3/refactor/crate-boundaries` · HEAD `c9d15023`(push · CI 초록) · ★워킹트리에 U4-T 미커밋 4파일★(아래). U3 `f03dd10c` · U4-C `c9d15023`.
★다음 첫 액션 = 사용자에게 U4-T 결정 한 줄 받기★ — (a) 4회째 수정(codex 제안: 새 회귀 시험이 `cli.pid.tmp` 를 읽어 catch 갈래 도달을 증명하고 기록된 손자 신원을 직접 확인) 후 재리뷰 → `/qa standard` → 커밋, 또는 (b) 그 실패 갈래 시험(`a_fake_cli_that_cannot_write_its_marker_ends_its_grandchild`)을 빼고 커밋. 그 뒤 U4 문서 정리 → U5.
사용자 지시(2026-10-12): 자율 모드 「쭉쭉」 · 단위마다 로컬 커밋 · push 는 몰아서 · master 머지 · 태그는 사용자 확인.

## U4-T — 미커밋 · 결정 대기 (결정 A 판정 자체는 확정)
- 파일: `crates/engram-dashboard-agent/tests/pump_panic_cli_cleanup.rs`(신규 · 미추적) · `.github/workflows/ci.yml`(gate 4 heredoc 한 줄) · `crates/engram-dashboard-platform/src/testing.rs`(`terminate_if_same` 시험 도우미 + 단위 시험) · `docs/process/S21-panic-policy/trd.md` §7(비-Windows 잔여 한 줄).
- 백업: scratchpad `u4/u4t-full-backup.patch`(스태시 전 전체) · `u4/pump_panic_cli_cleanup.r3.rs`. 스태시는 pop 해서 비었다. 꺼낸 시험 파일은 CRLF(내용 동일 확인 · autocrlf 라 커밋 시 LF).
- ★결정 A 성립★: PTY · stdio · codex 세 펌프 모두 가둔 펌프 패닉 뒤 명시 종료 없이 CLI 트리 전체(뿌리 · CLI · 손자 · conhost)가 상한 5 s(= ADR-0001 kill 예산) 안에 사라짐 — 실측 수 ms~20 ms. Job 없는 대조군에선 손자가 5 s 넘게 생존 → B안 불필요.
- 리뷰: Claude doc-aware PASS(3 라운드) · codex FIX 남음 — 재수정 하드캡 3 소진이라 멈춤. 남은 지적 = 3 라운드에 더한 실패 갈래 시험이 「catch 갈래를 실제로 탔는지」 확인 안 함(codex Medium · Claude Low).
- 미측정(헤더 · TRD 에 적음): 활성화 판정 창(`early_activation_verdict` · `link_activation_verdict`) 안 패닉 · codex 핸드셰이크 중 패닉 · 비-Windows.
- 커밋할 때 함께: CLAUDE.md 워크스페이스 결과 줄 59 → 60(이 파일이 `tests/` 루트에 하나 더함) · 이 시험의 qa(standard) 미실행.
- `%TEMP%\engram-u4-pump-panic-codex-49faef9a-…` 빈 임시 폴더 1개 잔여(권한으로 삭제 못 함 · 데이터 없음).

## U3 결과 (`f03dd10c`)
- 데몬 옛 로그 전용 훅 삭제 → `init_logging_and_panic_hook`(파일 로그 → base `install_hook`) · 셸 `run()` 첫 문장에 설치 · CLI 훅 없음. 데몬 lib 단위 `panic_hook_tests`(자기 재실행 3 갈래).
- TRD §7 잔여 추가: 로그 초기화 통째 실패 시 사유 줄 없이 끝남(닫으려면 기동 정책 = 사용자 결정).
- 게이트: `/review code deep` 4 라운드 PASS(Claude · codex 렌즈 둘) · `/qa full` PASS(59 · 4364 · 0 · 30 · GUI 실측 · stop_smoke 2/3 — 실패 1 = T-52 선재, 상세 로그로 정상 종료 경로 확인).

## U4-C 결과 (`c9d15023`)
- 40 파일 주석 · 로그 문구: 「release=abort · tokio 격리」 → 「표시 밖 패닉 = 즉시 종료(훅) · catch 그물은 훅 없는 시험 빌드에서만」. base sync 계약 3 재작성 · reaper 결정 A 경로 서술 · payload_text 통일(agent reaper · leftover — 비문자열 대체 문구가 `<non-string panic payload>` 로 바뀜, 소비처 없음) · `run_locally` → `spawn_local`.
- §4-6 ① 감사: 허용 목록 두 본문(`agent.list` · `agent.listQueuedInputs`)은 상태를 바꾸지 않는다(확실 · G2a 보고).
- 게이트: `/review code full` 2 라운드 PASS · `/qa standard` PASS(59 · 4364 · 0 · 30 · net · transport 게이트 포함).

## 사용자 판단 거리 (목록 — 하나씩 묻기)
1. U4-T: (a) 4회째 수정 vs (b) 실패 갈래 시험 빼고 커밋.
2. 실 claude 시험 6건: CLAUDE.md · qa 바인딩은 「로컬 standard 에서 돈다(의도)」, 옛 핸드오프는 「금지」 — U1~U4 전부 `--skip` 으로 돌렸다. 어느 쪽?
3. U2 허용 목록 범위(리뷰어 대립 → 안전 쪽 잠정 채택) 재판정.
4. 데몬 sweep 의 `catch_unwind` 갈래: 지금 어느 빌드에서도 닿지 않음(run() 만 띄우고 훅이 먼저) — 걷을지.
5. 로그 초기화 통째 실패 시 기동 거부 정책을 둘지(U3 잔여 · 지금은 문서화만).
6. 낡은 ADR 근거(아래 문서 목록) — ADR 근거 수정은 결정 날조 금지라 사용자 확인 필요.

## 미룬 문서 정리 (U4 문서 · 다음에)
- `docs/process/S21-panic-policy/trd.md`: :125 의 sweep 은 시험 빌드에서도 닿지 않음 · §4-6 ② 의 `pty.rs:283-297` · `reaper.rs:65-70` 줄 번호 낡음 · :162-163 `messaging_host.rs:769` → :770 · §4-7 에 「U5 를 되돌리면 `rg ADR-0292` 주석도 다시 볼 것」 한 줄(리뷰어 권고).
- ADR(근거가 release=abort 에 기댐): 0199:46 · 0226:43,75 · 0233:46 · 0262:105(K7) · 0275:16,52,87(S2=(e) — 결론은 유지된다는 코더 판단) · 0281:46 · 0142:57(미확인).
- TRD: S21-usage-limit-slot/trd.md:67 · S21-storage/trd.md:301,918 · S21-codex-backend/trd-resume-after-first-turn.md:101-103,298,529 · S21-crate-boundaries/trd-1-1-base-helpers.md:64,201,325,556 · S21-chat-ux/trd-t40.md:262 · S20-command-bus/trd.md:629.
- 코드(tests/): `src-tauri/tests/layout_commands.rs:3023` 「릴리즈는 panic=abort」.
- step-log: 패닉 U1~U4 항목(U1 · U2 커밋도 step-log 를 안 고쳤다 — 단위 끝에 몰아서).

## 남은 단위
U4-T 커밋(결정 후) → U4 문서 정리 → U5 `Cargo.toml:41` `panic = "abort"` 한 줄 삭제 + release 빌드 + `/qa full` + 실행 파일 크기 측정.

## 검증 재실행 명령
- U4-T 시험: `cargo test -p engram-dashboard-agent --test pump_panic_cli_cleanup -- --test-threads=4`(scripts/run-detached.ps1 경유)
- 훅 자식 프로세스 시험: `cargo test -p engram-dashboard-daemon --lib panic_hook_tests -- --test-threads=4`
- 워크스페이스: `/qa` 바인딩 standard(실 claude 6 `--skip` 붙임 — 판단 거리 2)

## do-not / 주의
- Git Bash 에 `rg` 없음 — QA 워커가 scratchpad `qa/bin/rg.exe`(codex 동봉 rg 15.1.0 · PCRE2)를 PATH 앞에 둠(세션 scratchpad 라 다음 세션엔 없을 수 있다).
- 시험 실행이 `crates/engram-dashboard-agent/bindings/*.ts` 를 줄끝만 바꿔 ` M` 으로 띄운다 — `git diff --numstat` 0 이면 `git checkout -- <그 폴더>`.
- 재리뷰는 리뷰어 resume(Claude = SendMessage · codex = `--resume` + 첫 줄 스레드 확인). codex 를 병렬로 둘 띄운 뒤엔 `--resume` 금지.
- codex blind 지시서엔 단계 순서(U5 는 뒤 단위) 같은 범위 사실을 처음부터 넣을 것 — 빠뜨려 BLOCK 이 한 번 났다(U3).
- `taskkill /T` 는 자기가 띄운 PID 에만(qa 바인딩 teardown · stop_smoke 정리는 그 범위).
- 실 claude 시험 · `real_wmi` 실행 금지(판단 거리 2 답 전까지).

## 정지 조건
- 리뷰 BLOCK · 리뷰어 정면 대립 · 재수정 하드캡 3 초과 → 사용자에게.
- master 머지 · 태그 · ADR 근거 수정.
