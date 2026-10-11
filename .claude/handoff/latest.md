# 핸드오프: 패닉 정책 구현 — U1 완료(744c060b · push) · U2 코더 진행 중

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.3.3/refactor/crate-boundaries` · HEAD = 이 핸드오프 커밋(그 앞 U1 `744c060b` — push 완료 · CI 확인 안 함). ★U2 코더 둘(agent 쪽 · daemon 쪽 — 파일 안 겹침)이 워킹트리에서 작업 중이었다 — 세션이 끊겼으면 미커밋 워킹트리 변경이 남아 있을 수 있다(git status 로 확인 · 리뷰 전 상태로 간주)★.
★다음 첫 액션 = U2 코더 결과 회수(끊겼으면 워킹트리 diff 를 보고 `/review code full` 부터) → FIX 반영 → `/qa standard` → 로컬 커밋 → U3.★
사용자 지시(2026-10-12): 「쭉쭉 진행」(자율 모드) · ★단위마다 로컬 커밋, push 는 적당한 때 몰아서(매번 X)★ · master 머지 · 태그는 사용자 확인.

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
