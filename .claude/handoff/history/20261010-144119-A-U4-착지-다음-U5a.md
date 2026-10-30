# 핸드오프: A(데이터 파일 처리 통일) U1 · U8 · U2 · U7 · U3 · U4 착지 — 다음 = U5a → U6 → U5b-1 → U5b-2 → U9 → 패닉 정책

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.3.3/refactor/crate-boundaries` · HEAD `a7376042`(push 됨 · 트리 깨끗). 착지: U1 `66765fb5` · U8 `fa6cb2f3` · U2 `bfd96dbc` · U7 `4e4fa543` · U3 `caed0f13` · U4 `a7376042`. TRD = `docs/process/S21-crate-boundaries/trd-A-data-file-unification.md`(6판 + U7 개정) · ADR-0291.
★다음 첫 액션 = U5a(agent 저장소 채택 — agents · presets · 레지스트리는 아직 지금 꼴) 코더 스폰.★ 그다음 U6(작음 · 파일럿 먼저 — TRD §8-2 1) → U5b-1(레지스트리 · manager · reaper · PtyError — 정지 지점 안전) → U5b-2(버스 · WS 표면 · agent catalog 6 → 7 · 도움말) → U9(문서 정리) → 패닉 정책 구현(TRD 부터).
단위 흐름: 코더 worker-senior → /review code(standard = full: worker-reviewer + codex:codex-rescue · critical = deep: + reviewer-deep 전문 렌즈) → /qa(worker-scout · TRD 행의 QA 칸 강도) → 커밋 · push. 리뷰 2 라운드가 흔하다.

## 사용자 결정 (2026-10-10)
- 「딱히 이슈 없으면 쭉 진행」 · 막히면 간단한 건 메인이 정하고 「잠정 — 사용자 확인 대기」. 리뷰어 정면 대립은 사용자에게.
- 「잠정」 다섯 묶음(ADR-0291 약한 기각 셋 · 결정 13 제자리 개정 · U7 kill 방식 · TRD 리뷰 메인 결정들 · U1 sync_dir → U3)은 브리핑했고 사용자 「잘 모르겠다 — 이슈 없으면 진행」 → 기본안 유지.
- U3 리뷰 대립(codex BLOCK 2) 사용자 판정: ① 폴더 동기화 실패 무시 → tracking T-55 기록만(macOS 이식 때) ② 소수점 거대 버전 → 무시.
- 패닉 unwind 범위 = 워크스페이스 전체 + 셸에도 같은 훅 ★패닉 구현 ADR(0292)에 박을 것★. master 머지 · 태그 · panic-policy 머지 = 안 함.

## 검증 상태
- U4 QA full: 워크스페이스 59 결과 줄 · 4281 통과 · 0 실패 · 31 무시 · 셸 lib_unit 826 · layout_commands 91 · tsc · npm test 2021 · 실 앱: 새 판 state.json(바이트 불변 · 안내 문구 · restore.status newer · saves=false) · 새 판 settings.json(CONFLICT · 불변) · 빈 데이터 폴더 첫 실행(폴더 · 파일 생성).
- 선재 실패: T-56(codex 터미널 실 CLI 계약 시험 5건) · T-52(graceful 데몬 끄기 Timeout).
- 수락한 잔여(U4 codex r2 · C1 범위): 부팅 ③~⑤ 수 ms 안 바깥 편집 때 상태 표시 두 건(인코딩 오류 경로가 재판정 전에 돌아감 · 늦은 손상 떠 두기가 state_file 에 안 실림) — U9 에 한 줄 기록 후보.

## U5a 지시서에 넣을 것
- TRD §6-1 U5a 행 · §3-4(모양 검사 = 명부 · 프리셋 칸 역직렬화 — 읽기 쪽은 버전 키를 필수로 두지 말 것 · `persistence/mod.rs:27-30` · `presets.rs:24-27`) · §3-9(저장소 상태 칸 잎 락 · 트레이트 `save -> Result` + `status` · 가짜 16개 — 실제 18 impl, `manager.rs:3073 · 5016` 포함) · D7(적재 읽기 예산 ~1 초 — 운영 `OsHooks` 둘) · 옛 `agents.json.tmp` 직접 지우기 · 락 순서 문장 다섯 자리.
- serde 오류 문자열이 값을 실을 수 있다(U3 리뷰) — 모양 실패 사유를 어떻게 담을지 U5a 에서 정한다(지금 agent 도 이미 값을 로그한다 — 회귀 아님).
- base `file` API: `Spec{version_key,current,cap,shape}` · `parse` · `classify` · `load` · `Claim` · `write_policy` · `judge` · `check_cap` · `write_atomic(_unless)` · `copy_atomic` · `copy_aside` · `sweep_temps` · `OsHooks{retry,sync_dir}`(fn 포인터) · 상수 `BUSY_RETRIES`/`BUSY_PAUSE`. 셸 운영 값 = `src-tauri/src/file_hooks.rs`(선례).

## do-not / 주의
- Git Bash 에 `rg` 없음 → `C:\Users\kimsunzun\AppData\Local\OpenAI\Codex\bin\ada252862d154cdd\` 를 PATH 앞에(모든 워커 지시서). `bash` 가 WSL 이면 거짓 FAIL.
- 줄끝 파일마다 다름 → `git ls-files --eol <경로>` 로 확인 · `sed -i` 금지 · `node -e`. `cargo fmt` 가 CRLF 파일을 LF 로 바꿀 수 있다(U4 실발생).
- `lib_unit` 실행이 `src-tauri/bindings/*.ts` 전부를 LF 로 다시 쓴다 → 커밋 전 `for f in $(git diff --name-only -- src-tauri/bindings); do git diff --ignore-cr-at-eol --quiet -- "$f" && git checkout -- "$f"; done`.
- QA GUI 정리에 `taskkill /T` 금지(T-54) — 이번 실행이 띄운 PID 만 /F. 앱은 `quit_app` 으로 닫는 게 깔끔했다.
- 새 파일은 리뷰 diff 전 `git add -N`. 다음 ADR 번호 = 0292(/adr 스크립트는 로컬 최대로 채번 — 자리채움 우회 필요했다).
- U9 문서 몫 누적: ADR-0275 결정 2 · 6 → 0291 링크 · 0266 · 0269(결정 6 셋째 줄 포함) · 0274 · 0275 의 fsutil 언급 · S21-storage TRD §6-5 · §12 R3 · :263 · TRD-1-1 :161 · §8 D2 · `docs/reference/structure/session-path-ownership.md` · 메모 :38 · :88 · :308 · CLAUDE.md 수치(워크스페이스 4281 · lib_unit 826) · platform `retry_busy_with` doc · TRD A §3-6 줄 번호 · U4 C1 잔여 한 줄.

## 정지 조건
- 단위 재수정 하드캡 3 초과 · 리뷰어 정면 대립(설계 방향) · 사용자 눈에 보이는 새 갈림길(U5b 의 화면 표시 · 오류 문구는 관행대로 메인이 정하고 잠정).
- master 머지 · 태그.
