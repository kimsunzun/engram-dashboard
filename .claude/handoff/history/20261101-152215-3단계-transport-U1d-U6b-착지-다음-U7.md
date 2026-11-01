# 핸드오프: 3단계 transport — 3-0 U1d~U6b 착지 · 다음 = 3-0 U7

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.4.1/refactor/transport-attach` HEAD = 이 핸드오프 커밋(코드 마지막 = `5d981aaf` U6b). 작업트리 깨끗. ★push 안 함 — 원격은 `dd0499a7` 까지★.
이번 세션 커밋: 098dad63 U1d-1 · 379d5cf8 U1d-2 · 25439d4f 문서(TRD · step-log) · fd7552a8 U2 · b35af1c7 U3 · 00fc52d4 U4 · de104f69 U5 · 9be94cd9 U6a · 5d981aaf U6b.
★다음 첫 액션 = 3-0 U7★(ws 수락 강화 — TRD §6-0 U7 행 · §5 1·2·12·14~18): `AcceptPolicy`(필수 · Default 없음) · 무검사 `accept` 삭제 · `split_link` → `pub(crate)` · 업그레이드 대기 상한(U6a `PermitPool` 재사용) · 유계 준비 큐 · 시한 · Origin/Host 훅 · 양방향 크기 · loopback 단언 · ws `Listener` 구현 · ★거절 보고는 trait 밖 — bind 가 리스너 + 거절 수신단을 함께 준다(메인 결정)★ · 실소켓 스위트를 transport gate 4c 에 등록 — 사본 셋(`ci.yml` · qa 바인딩 · transport 헤더 ⑤)을 같은 커밋 · `/review doc`. 덤: `mask.rs` 를 게이트 ③ 순수 층 목록에 넣을지 판단.
그 뒤 3-1 U5(게이트 사본 순서 3-0 U7 → 3-1 U5) → 3-2(셸 부착) → 3-3. 문서 후속(아래)은 U7 의 `/review doc` 과 묶어도 된다.
진척: 3단계 23 단위 중 ≈11(3-0 U1~U6 · 3-1 U1~U4 완료).

## 사용자 결정 · 지시(이번 세션)
- 다중 데몬(3단계 뒤)은 바로 착수하지 않는다 — 먼저 사용자에게 네트워크 지식 · 구조 강의.
- 토큰 절약 개선은 스킬 지시서에 쓰지 말고 핸드오프로만(스킬 시스템 개선 중 — 새 지시서 들어오면 사용자가 사인).
- 「토큰 소모하라」 지시는 기록에 없었다(확인함).

## 진행 방식(토큰 절약 — 이 주제에서 적용 중 · 실측 근거)
- 실측: 코더 30만~50만/단위 · 예전 Claude 리뷰어 27만~36만/회(자기 빌드 + 변이) → 읽기 전용으로 바꾼 뒤 16만~24만 · codex ~8만/회 · QA 14만~17만/회.
- Claude 리뷰어 = 읽기 전용(빌드 · 변이 금지 — 코더 변이 결과를 검토). 첫 회차 = Claude + codex 렌즈 1(diff) + 렌즈 2(호출부 — 새 API 뿐이면 생략). 확인 회차 = 고친 델타만 Claude 하나(작은 기계적 수정은 메인 직접 확인으로 갈음).
- 문맥 30만 넘은 코더는 이어 쓰지 않는다 → 새 코더 + 짧은 지시서.
- QA = worker-scout · run-detached · 4f~4k 는 `source` 로(자식 bash 는 rg 함수가 없어 거짓 FAIL). 시험 뒤 바인딩 66개 `M` 은 줄끝뿐 — `git diff --quiet` 확인 후 `git checkout --` 로 되돌리고 커밋.

## 검증 상태
- 앞선 QA(U6a 커밋 시점) /qa standard PASS: 결과 줄 60 · 5274 통과 · 0 실패 · 35 무시 · lib_unit 1351 · transport 360/370(+ ws 2/5/4) · 프론트 2802.
- U6b 커밋 시점 /qa standard PASS: 결과 줄 60 · 5294 통과 · 0 실패 · 35 무시 · lib_unit 1351 · transport 380/390(+ ws 2/5/4) · 프론트 2802.
- 재실행 = `scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<경로>.log"`.

## 리뷰 기록에 남는 것
- U2 확인 회차 = codex 한도로 Claude 대체 — 단독 family 리뷰(cross-family 교차검증 없음).
- 문서 리뷰: codex 「U5 행 취소선 묶음 삭제」 미반영(저장소 관례 취소선 보존) — 사용자에게 보고함.

## 문서 후속(load-bearing → /review doc 묶어 한 커밋)
- TRD 착지 표시: U2 · U3 · U4(★TRD 와 다른 점★ — Wire::DecodeError 경계 Display → DecodeClass(3-1/3-2/3-3 Wire 구현이 구현해야) · Refused 에 reason · Rejected.code non-Option · Retryable 은 Rejected 사건 없이 ConnectFailed.cause 로 → 예산 소진 문안은 마지막 ConnectFailed 코드로 · 핸드셰이크 중 4002–4005 = Retryable) · U5(HandshakeStep Debug 의 닫기 문구는 그대로) · U6a/U6b 분할 · U7 거절 보고 결정 · §5 18.
- 3-2 셸 어댑터 TRD 메모: 옛 세대로 버린 명령 답장(stale notify_in)이 진짜 유실과 같은 Dropped{Outbound} 로 나간다 — 화면 경고로 띄우지 말 것.
- step-log 에 U2~U6b.

## 기타
- 의존성: transport `[dev-dependencies] tokio` features `rt-multi-thread`(U6b 시험) — 운영 그래프 무관 · Cargo.lock 무변경(사용자에게 보고함).
- 선재 하네스 결함 보류: `testing.rs` `MemTx::ping`(MemoryEndpoint) 도 ping 멈춤 · 쓰기 멈춤 두 단계 대기 결함 — 고치면 기존 peer 시험 타이밍이 바뀔 수 있음.
- 미사용 메모: `mask.rs` 를 lib.rs 게이트 ③ 순수 층 목록에 넣을지 = U7(게이트 사본 고칠 때).
- 주의(이전 그대로): codex 리뷰가 Bash 600 s 에 걸리면 `tasks/<id>.output` 회수 · `python3` 는 Store 스텁(`/c/Users/kimsunzun/AppData/Local/Python/bin/python.exe`) · `rg` = `C:\Users\kimsunzun\AppData\Local\OpenAI\Codex\bin\0354bc6c584f210c\rg.exe` · 워크트리 `.claude/worktrees/stage3-protocol` 쉬는 중 · 지우지 못한 임시 폴더(사용자 몫) `I:\Engram\apps\rv-u1a-target` · `%TEMP%\engram-review\31u2\{tgt,blocks}`.
