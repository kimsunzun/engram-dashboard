# 핸드오프: 3단계 transport 구현 중 — 3-1 U1~U3 · 3-0 U1a 착지 · 다음 = 3-0 U1b ∥ 3-1 U4

## 한 줄 상태 · 다음 첫 액션
브랜치 `v0.4.1/refactor/transport-attach` · HEAD `5bc3e633` · 작업트리 깨끗. 결정 26개 확정(ADR-0310) · TRD 6판 확정 · 단위 23 중 커밋 4(3-1 U1 · U2 · U3 · 3-0 U1a — U1a 는 3-0 U1 을 넷으로 쪼갠 첫 조각).
★다음 첫 액션 = 두 갈래 병렬로 재개★: ① 3-0 U1b(순서 칸 — 메인 체크아웃) ② 3-1 U4(명령 칸 넷 `Box<RawValue>` — 워크트리 `.claude/worktrees/stage3-protocol`, 분리 HEAD = `84615909`). 착수 전에 워크트리를 브랜치 HEAD 로 맞춘다(`git -C .claude/worktrees/stage3-protocol checkout --detach v0.4.1/refactor/transport-attach`). 게이트 사본 줄(ci.yml · qa 바인딩 · CLAUDE.md) 직렬 = 3-1 U2(끝) → **3-0 U7** → 3-1 U5 — 3-1 U5 는 3-0 U7 뒤에만.

## 사용자 방침 (이 세션)
- 3단계는 1:1 동작 + 데몬 N 바닥(D4 ①)까지. **다중 데몬은 3단계 바로 다음, 이 흐름에서 이어서**. 그 전에 **질의응답식 학습 자료**로 개념을 익히게 한 뒤 PRD(사용자가 구조 제안을 하려면 개념이 필요). 브리핑 문서보다 문답이 잘 맞는다.
- 「그만하라 할 때까지 에이전트 여러 개 합리적으로」 — 파일이 안 겹치는 갈래는 병렬.
- ★핸드오프는 단위 경계에서★ — 진행 중 단위를 커밋까지 끝내고 끊는다(사용자 지적 2026-10-31).
- 네트워크 세부는 메인 판단에 위임(「너가 판단해서 분배」). 사용자 체감 정책만 묻는다.

## 읽을 것
- 명세: `docs/process/S21-crate-boundaries/trd-3-transport.md`(6판 · §6 단위표 · §3-0 · §3-3 거름 세 규칙) · 결정: `docs/decisions/0310-*.md`.
- master 재확인(단위에 더해진 일 A1~A4 · 낡은 포인터): `.claude/handoff/attachments/trd3-master-recheck-2026-10-31.md` — 6판에 반영됨.
- 사용자용 그림: `trd-3-transport-zoom.html`(확대도) · `trd-3-transport-brief.html`.

## 검증 상태
- 기준선(구현 전 `66fcc5c9`): 결과 줄 60 · 5067 · 0 · 35 · `lib_unit` 1328.
- 3-1 U3 커밋 시점(워크트리, 3-0 미포함): 60 · 5076 · 0 · 35 · 격리 4~4k PASS.
- 3-0 U1a 커밋 시점(메인, 3-1 미포함): 60 · 5087 · 0 · 35 · transport 네 기능 조합 강제 재빌드 후 209/219/209/219 · 격리 PASS.
- ★미검: 두 갈래가 합쳐진 지금 HEAD 의 워크스페이스 회귀는 안 돌렸다★(파일 안 겹침 · transport 소비자 0 이라 위험 낮음 — 예상 ≈ 5096). 다음 단위 QA 가 겸한다. 재실행 = `scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4"`.
- push: ADR 커밋(`dd0499a7`)까지만 원격. 그 뒤는 로컬.

## do-not / 주의
- ★변이 시험 · QA 가 같은 target 을 공유하면 낡은 바이너리가 돈다★ — 원본 복원은 수정 시각을 갱신(`touch`)하고, QA 전 대상 crate `touch`. 이번 U1a 거짓 FAIL 이 그것이었다.
- ★codex 리뷰 슬롯에도 Claude 와 같은 맥락을 준다★(사용자 결정 2026-10-19 · review feedback.md — 스킬 본문 미반영). 이 세션 앞쪽은 blind 로 돌렸다.
- 리뷰어 지시서엔 「새 target 폴더 만들지 마라」. 이번에 지우지 못한 것(사용자 몫): `I:\Engram\apps\rv-u1a-target` · `%TEMP%\engram-review\31u2\tgt` · `%TEMP%\engram-review\31u2\blocks`.
- `rg` 는 Git Bash · PowerShell PATH 에 없다 — `C:\Users\kimsunzun\AppData\Local\OpenAI\Codex\bin\0354bc6c584f210c\rg.exe` 를 PATH 에. `python` 은 Store 스텁(`py`). run-detached 로그 경로는 슬래시로 변수에 먼저 담는다(`\$2` 가 그대로 넘어간 실수 2회).
- 시험이 `**/bindings/*` 줄끝만 바꾸면 `git checkout --`.
- U1a 는 U1d 에서 갈아엎을 임시 구조(≈1100줄)다 — U1b · U1c 는 그 위에 얹되 바이트 예산은 「연결마다 · 항목별 RAII」(줄 단위 아님 — 줄을 연속 연결이 같이 쓴다).

## 단위 진행 방식 (이 세션에서 굳힌 것)
코더 = worker-senior(단순 = worker-scout) → `/review code`(Claude worker-reviewer + codex:codex-rescue, 병렬) → 재수정(상한 3 · 넘으면 사용자) → `/qa`(worker-scout 위임 · run-detached) → 커밋. 3-1 은 워크트리에서 짓고 브랜치로 cherry-pick 후 워크트리를 브랜치 HEAD 로 다시 맞춘다. 강도: transport 동시성 · 수명 = critical(deep) · protocol 바이트 변경 = critical · 그 밖 standard.

## 정지 조건
- 리뷰 BLOCK · 정면 대립 · 재수정 상한 초과 · master 머지 · 태그 → 사용자에게.
- TRD 단위 범위를 바꾸는 판단(예: U1d 를 앞당기기)은 보고 후 진행.
