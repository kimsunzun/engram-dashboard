# 핸드오프: 3단계 transport — 3-0 전부 착지(U1~U7) · 다음 = 3-1 U5 → 3-2

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.4.1/refactor/transport-attach` HEAD = 이 핸드오프 커밋(코드 마지막 = `baf8e42a` U7 · 문서 = `7ee4a241`). 작업트리 깨끗. ★push 안 함 — 원격은 `dd0499a7` 까지(로컬 커밋 다수 앞섬)★.
이번 세션 커밋: 098dad63 U1d-1 · 379d5cf8 U1d-2 · 25439d4f 문서 · fd7552a8 U2 · b35af1c7 U3 · 00fc52d4 U4 · de104f69 U5 · 9be94cd9 U6a · 5d981aaf U6b · 10fcae05 핸드오프 · baf8e42a U7 · 7ee4a241 문서(U2~U7 착지).
★다음 첫 액션 = 3-1 U5★(TRD §6-1 U5 행 — 착지로 줄었다: protocol → command Cargo 간선 삭제 + protocol 의존 상한 게이트 + 문서. 지킴이 · golden 은 U4c 가 셸로 옮겼다). 게이트 사본 셋(ci.yml · qa 바인딩 · CLAUDE.md 「빌드·검증 명령」 줄 — 3-1 은 CLAUDE.md 도 고친다)을 같은 커밋 · `/review doc`.
그 뒤 3-2(셸 부착 — §6-2 U1~U5 · GUI 실측 `/qa full` 포함) → 3-3(데몬 부착 · net 걷기 — §6-3 U1~U6). 3-2 착수 전 TRD §6-2 U2 의 「3-0 착지가 더한 주의 셋」(DecodeClass · 옛 세대 Dropped 는 UI 경고 아님 · 예산 소진 문안은 마지막 ConnectFailed 코드)을 읽을 것.
진척: 3단계 23 단위 중 ≈12(3-0 U1~U7 · 3-1 U1~U4 완료). S21 전체 = 1단계 · 2단계 · A 완료, 3단계 진행 중.

## 사용자 결정 · 지시(이번 세션)
- 다중 데몬(3단계 뒤)은 바로 착수하지 않는다 — 먼저 사용자에게 네트워크 지식 · 구조 강의.
- 토큰 절약 개선은 스킬 지시서에 쓰지 말고 핸드오프로만(스킬 시스템 개선 중 — 새 지시서 들어오면 사용자가 사인).
- 「토큰 소모하라」 지시는 기록에 없었다(확인함).
- 사용자가 「쭉쭉 진행 · 끝나면 핸드오프」로 위임 — 단위마다 리뷰 · QA · 커밋까지 자율로 돈다.

## 진행 방식(토큰 절약 — 이 주제에서 적용 중 · 실측 근거)
- 실측: 코더 15만~50만/단위(작은 수정 15만~22만) · Claude 리뷰어 읽기 전용 16만~25만/회(예전 빌드 + 변이 27만~36만) · codex ~8만/회 · QA 12만~17만/회.
- Claude 리뷰어 = 읽기 전용(빌드 · 변이 금지 — 코더 변이 결과를 검토). 첫 회차 = Claude + codex 렌즈 1(diff) + 렌즈 2(호출부 — 새 API 뿐이거나 transport 밖 소비자 0 이면 생략). 확인 회차 = 고친 델타만 Claude 하나 · 작은 기계적 수정은 메인 직접 확인으로 갈음.
- 문맥 30만 넘은 코더는 이어 쓰지 않는다 → 새 코더 + 짧은 지시서. 코드 · 문서는 파일이 안 겹치면 병렬로.
- QA = worker-scout · run-detached · 4f~4k 는 `source` 로(자식 bash 는 rg 함수가 없어 거짓 FAIL) · 헬퍼 = scratchpad `qa8/rd.sh` · `qa6/g4*.sh`(세션 스크래치 — 새 세션엔 없음, 바인딩 원문에서 다시 뽑는다). 시험 뒤 바인딩 57~66 개 `M` 은 줄끝뿐 — `git diff --quiet` 확인 후 `git checkout --` 로 되돌리고 커밋.

## 검증 상태
- U7 커밋 시점 /qa standard PASS: 결과 줄 61(새 `ws_accept` — 기본 기능에선 0 건) · 5294 통과 · 0 실패 · 35 무시 · lib_unit 1351 · transport 380 / all-features 405 + ws_accept 12 · ws_dial 2 · ws_reject 5 · ws_write_deadline 4 · 격리 게이트 전부 · 프론트 2802.
- 재실행 = `scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<경로>.log"`.
- 안 돌린 것: GUI 실측(3-2 에서) · 실 데몬 · 실 WMI 레인(transport 만 바뀜).

## 리뷰 기록에 남는 것
- U2 확인 회차 = codex 한도로 Claude 대체 — 단독 family 리뷰(cross-family 교차검증 없음).
- 첫 문서 리뷰: codex 「U5 행 취소선 묶음 삭제」 미반영(저장소 관례 취소선 보존) — 사용자에게 보고함.
- U2 · U4 · U6a · U6b 의 lib 헤더 고침은 별도 `/review doc` 없이 `/review code deep` 안의 doc-aware 리뷰어가 봤다(step-log 에 기록).

## 남은 메모 · 보류
- 3-3 U4: 인증 대기 칸 · 운영 칸 없음 — 지금 업그레이드 칸이 Live 까지 그 몫 · 운영 중 받은 상대 수 상한 없음(TRD 에 「TRD 와 다른 점」으로 기록).
- 받은 `Close.reason` 가림 = TRD §9 후속.
- 선재 하네스 결함 보류: `testing.rs` `MemTx::ping`(MemoryEndpoint) ping 멈춤 · 쓰기 멈춤 두 단계 대기 — 고치면 기존 peer 시험 타이밍이 바뀔 수 있음.
- `mask.rs` 는 게이트 ③ 순수 층 목록에 안 넣었다(그 게이트는 파일 단위라 import 를 안 본다 — U7 코더 판단).
- 의존성: transport `[dev-dependencies] tokio` features `rt-multi-thread`(U6b 시험) — 운영 그래프 무관 · Cargo.lock 무변경(사용자에게 보고함).
- 주의(이전 그대로): codex 리뷰가 Bash 600 s 에 걸리면 `tasks/<id>.output` 회수 · `python3`/`python` 은 Store 스텁(`py` 사용) · `rg` = `C:\Users\kimsunzun\AppData\Local\OpenAI\Codex\bin\0354bc6c584f210c\rg.exe` · 워크트리 `.claude/worktrees/stage3-protocol` 쉬는 중 · 지우지 못한 임시 폴더(사용자 몫) `I:\Engram\apps\rv-u1a-target` · `%TEMP%\engram-review\31u2\{tgt,blocks}`.
