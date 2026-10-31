# 핸드오프: 3단계 transport — 3-0 U1b · U1c · 3-1 U4a~c 착지 · 다음 = 3-0 U1d

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.4.1/refactor/transport-attach` HEAD = 이 핸드오프 커밋(코드 마지막 = `92096d19` U1c). ★push 안 함 — 원격은 `dd0499a7` 까지★. 작업트리 깨끗(메인).
이번 세션 커밋: 6c391a37 3-0 U1b(순서 칸) · b100b97c 3-1 U4a(봉투) · a3097400 U4b(답장) · cd992361 U4c(등록 둘) · 92096d19 3-0 U1c(배압 · 바이트 셈 · 멈춤 · 건지기).
진척: 3단계 23단위 중 ≈5.75 — 3-1 U1~U4 완료(U5 는 3-0 U7 뒤) · 3-0 U1 의 a·b·c 완료(U1d 남음) · 3-2 · 3-3 미착수. S21 전체 = 1단계 · 2단계 · A 완료, 3단계 진행 중 → 그다음 다중 데몬(문답 학습 자료 → PRD).
★다음 첫 액션 = 3-0 U1d★ — 손잡이별 수신단 표면(D24 ②: `attach` · `add` 가 상대별 수신단을 돌려준다 · 훅 없음) · 수신단 놓임(`Sender::closed()`) 감지 → 닫기 · 끝 원인 가름(`ReadFailed` · `WriteFailed` 가안) · lib.rs 「받는 쪽 불변식」 5 의 뒤 절반 · U1a 의 임시 구조(registry ≈1100줄) 갈아엎기 · 60 s 비우기 상한 버림을 수신단 끝(None)으로 알림. 코드의 `TODO(TRD 3 §6-0 U1d)` 표시가 할 일 목록이다(`rg "U1d" crates/engram-dashboard-transport`). 그 뒤 U2 → … → U7(전부 transport 같은 파일 → 순차) → 3-1 U5(게이트 사본 순서 3-0 U7 → 3-1 U5) · 3-2 는 3-0 U1~U5 뒤.

## 문서 후속(load-bearing → /review doc 묶어서 한 커밋)
- TRD §3-0 ①: 읽기 문턱 = 스트림 없으면 B · 첫 스트림 조각부터 B/2 + 붙든 몫 B/2(총 B+M 유지) · 배압 문턱 = 순서 칸 비지 않음 · 앞잡이 건지기 · 재요청 창 시계 멈춤. TRD ≈195 「셈이 예산 B 이상이면」 문장 갱신.
- TRD §6-1 U4 행: 깨진 등록 칸 = `Error{Some(request_id)}` + INVALID_ARGUMENT(가안 None 에서 바꿈 — 번호가 칸 밖 · 셸 등록 대기 상한 없음). U5 행: CommandDecl golden · 드리프트 지킴이는 U4c 에서 셸로 먼저(남은 일 = Cargo 간선 삭제 + protocol 상한 게이트).
- step-log 에 이번 세션 단위들.

## 사용자에게 보고할 것(아직)
- U1b: lib.rs 2026-09-26 사용자 결정 기록 문구를 좁힘(결정 자체는 그대로).
- U4c: 깨진 등록 칸 Some(번호).
- 선재 결함 기록: 스트림 재생 vs 타이머(배압 무관 · `TODO(stream-replay-vs-timer)` — 운영 wire 는 스트림 미사용) · 깨진 페이로드 warn 래치 없음(U4a~c 공통 — 나중 래치 정책 하나로).

## 검증 상태
- U1c 커밋 시점 /qa standard PASS: 결과 줄 60 · 5186 통과 · 0 실패 · 35 무시 · lib_unit 1351 · transport 272/272/282/282(기본/test-support/ws/all-features) · 격리 4~4k · 생성물 내용 무변경 · 프론트 2802. 주석 한 줄 수정은 그 뒤(fmt · check 만 확인).
- 재실행 = `scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<경로>.log"`.

## 단위 진행 방식(이 세션에서 굳힌 것)
코더 = worker-senior → /review code deep(Claude worker-reviewer + codex 렌즈 둘(1 = diff 만 · 2 = diff + 파일 + 호출부), 같은 맥락 · 병렬) → 수정(상한 3 · 넘으면 사용자) → /qa standard(worker-scout · run-detached) → 커밋. 수정 확인 라운드도 같은 셋. 3-1 은 워크트리에서 짓고 cherry-pick.
- ★맡기기 전 크기 재기★: U1c 는 단위 하나에 코더 + 수정 3회 + 리뷰 4회 ≈ 250만 토큰. U1d 도 넓다(수신단 표면이 바뀌면 testing.rs · registry 가 같이 바뀐다) — 둘로 쪼갤지 착수 때 판단.

## 주의
- codex 리뷰가 Bash 600 s 에 걸리면 detach — `tasks/<id>.output` 을 직접 읽어 회수(세션 안에서만).
- 변이 시험 뒤 원본 복원은 cmp + touch · QA 전 transport touch(낡은 바이너리 거짓 결과 전례).
- 작업트리 CRLF/LF 섞임 — 스크립트 편집은 `\r?\n` · `python` 은 Store 스텁(py 사용) · `rg` 는 `C:\Users\kimsunzun\AppData\Local\OpenAI\Codex\bin\0354bc6c584f210c\rg.exe`.
- 워크트리 `.claude/worktrees/stage3-protocol` 은 쉬는 중(바인딩 122 줄끝 변경 — 내용 무변경). 쓸 때 `git -C … checkout --detach v0.4.1/refactor/transport-attach`.
- 지우지 못한 임시 폴더(사용자 몫): `I:\Engram\apps\rv-u1a-target` · `%TEMP%\engram-review\31u2\tgt` · `%TEMP%\engram-review\31u2\blocks`.
