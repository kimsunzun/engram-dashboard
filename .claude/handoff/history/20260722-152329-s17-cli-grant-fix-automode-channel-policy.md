# 핸드오프: S17 CLI 발신 grant 정렬(0/38→10/10)·스폰 auto mode 채택·채널 정책 확정 — capability 스위치 구현 대기

> 직전 핸드오프(4ac8c70 대비 latest) 대비: CLI 격리 실증(옛 "1/1")이 **0/38로 반증**됐고, grant 배관 결함을 찾아 고쳐 10/10 회복 + auto mode 채택 + 채널 정책 확정. 이 파일이 최신 정본.

## 한 줄 상태 · 다음 첫 액션
- **상태:** 이 세션 **5커밋 완료·미push**(origin/master 대비 7 ahead). 소스 워킹트리 클린(`.codex/`·`.vs/` 스크래치만). CLI 발신 0/38 차단 원인(grant 미매칭) 규명·수정 → CLI 10/10 회복. 스폰 기본 auto mode(bypassPermissions) 채택(ADR-0097). 채널 정책 확정(사용자): **MCP 있으면 MCP, 없으면 CLI 폴백 = capability 스위치.**
- **다음 첫 액션(사용자 확정 완료 — 구현 대기):** **채널 = 백엔드 capability 스위치**를 구현. ① `ControlCaps`(types.rs, ADR-0030)에 `mcp_send` 류 플래그 ② `build_grants`(control/mod.rs)·프라이밍 선택이 그 플래그로 분기(MCP-capable→MCP입구+프라이밍 / 미지원→CLI만) ③ 같은-백엔드 MCP 실패 폴백 = both-teaching. **굵은 결정 = ADR 먼저 박고 /implement.** (현재는 build_grants가 MCP를 항상 provision → claude는 사실상 늘 MCP, CLI는 비-MCP 백엔드·실패 폴백 자리.)

## 이번 세션 커밋 (5개, 로컬 미push · origin/master 대비 앞섬)
- `57e7e7c` fix(roundtrip-smoke): CLI-지시 판정 파일명→내용 기반 (ADR-0094)
- `0f5ad87` fix(grant): **CLI 발신 grant bare-name+PATH 정렬** (ADR-0098) ★핵심
- `9f3fd34` feat(priming): 영어 v3 both 변형 신설 (v3-en/cli/both 분리)
- `6de13f0` feat(spawn): **스폰 기본 auto mode(bypassPermissions)** (ADR-0097) ★핵심
- `8676da3` docs(adr): ADR-0097·0098 박제 + 인덱스 + step-log + both v2
- (그 앞 `7c47947`·`05d80ec`는 직전 세션분 — 함께 미push)

## ★ 핵심 발견·해결 (근거 = ADR-0098/0097 본문)
- **문제:** engram-send(CLI) 발신 실측 **0/38 전량 permission-block**(colon·xml·sonnet·haiku 균일). 옛 핸드오프 "CLI 1/1 ✅"는 표본 1개 요행 = **반증됨.**
- **원인:** ADR-0094 grant `Bash(<절대경로> *)`(space-star) ≠ 에이전트 실호출 `$ENGRAM_SEND_EXE`(env변수) → claude 권한 매처 문자열 미매칭. claude.rs:280-287 주석이 이미 "미검증·best-effort"로 경고했던 지점 — 이번 실측이 그 후속 검증.
- **수정(ADR-0098):** 세 문자열을 bare `engram-send`로 정렬 — build_grants=bare name, grants_to_allowed_tools=`Bash({e}:*)`+`PowerShell({e}:*)`(colon-star, PowerShell 셸 실패모드 커버), build_spec=PATH 주입(형제 dir prepend·프로필 PATH 존중 last-wins+dedupe·join실패/비UTF8 loud skip). 프라이밍 bare 정렬. → **0/38 → cli-sonnet 10/10·haiku 8/10·xml 9/10**(entrance=cli).
- **auto mode(ADR-0097):** 헤드리스 워커 기본 거부의 구조적 벽(승인자 부재·grant 미매칭 전멸) → 모든 스폰에 `--permission-mode bypassPermissions` 무조건(args 첫 base 플래그, allowedTools variadic 맨끝 불변 유지). 발신 grant는 미래 공용 제약 레이어용 정책 표면으로 유지.

## 측정 결과 (실측, .codex/phase*-summary.txt — disposable)
- **grant 검증(default-deny 체제):** cli-sonnet 10/10 · cli-haiku 8/10 · cli-xml 9/10(전 cli) · both+MCP 10/10(mcp, 회귀 0) · both+MCP제거 1/10(seam 한계 ↓).
- **auto mode 채널 선택(MCP·CLI 공존):** auto-cli **8/8 cli** · auto-mcp 3/3 mcp · auto-both 3/3 mcp(주력 준수). → **auto mode에서 CLI 정상 + 채널은 프라이밍 따라 스위칭됨**(MCP 보여도 CLI 프라이밍이면 CLI).

## 검증 상태 (쌍)
- **한 것:** 매 변경 코더(worker-senior)→`/review code full`(doc-aware Claude + blind codex 적대, 매 라운드 findings CLOSED까지)→`/qa standard`. 최종 게이트: `cargo build`·`cargo test -p engram-dashboard-core`(236)·`-p engram-dashboard-daemon --features test-harness`(전 스위트)·`cargo fmt --check`·코어격리(rg use tauri→0, lib.rs 주석 매치만)·`tsc --noEmit`·`vitest`(621) 전부 green. auto mode 라이브 roundtrip 발신 실측 성공. bypass 헤드리스 non-granted Bash 실행 실측 확인.
- **재실행 명령:** `cargo test -p engram-dashboard-core` · `cargo test -p engram-dashboard-daemon --features test-harness` · `cargo fmt --check` · `rg "use tauri" crates/engram-dashboard-core/src/`(0=PASS). roundtrip(auto mode): `cargo run -q -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --priming <file> --model sonnet`. 봉투 xml = `ENGRAM_WRAP_FORMAT='<message from="{sender}">{body}</message>'`.
- **안 한 것:** ① 채널 capability 스위치 미구현(사용자 확정, ADR+구현 대기) ② both 폴백 v2("없거나 차단/실패하면") 재측정 — auto mode에선 `--disallow-mcp`로 안 됨(진짜 MCP-부재 = 서버 미부착 필요) ③ opus 미측정(sonnet/haiku만) ④ CI 미도입(아직 안 함, 사용자 계획) ⑤ 5커밋 push 안 함 ⑥ 봉투 포맷 영속화(메모리만, 직전 세션 이월).

## do-not (누적)
- **명령:** 루트 bare `cargo test` 금지(src-tauri WebView2 크래시 — `-p` 명시). git commit은 pathspec(`.vs/`·`.codex/` 제외). roundtrip 스모크 **병렬 금지**(데몬 포트파일 충돌 — 순차). 빌드 전 데몬 잔존 시 `Stop-Process -Name engram-dashboard-daemon`. 임시폴더 `engram-roundtrip-*`는 런 강제종료 시 샘(주기적 청소 — 이 세션 18개 청소함).
- **측정 seam:** `--disallow-mcp`/`ENGRAM_DISALLOW_MCP_SEND`는 MCP **grant만** 제거. **auto mode(bypass)에선 무력화** — 권한 게이트 없고 mcp-config 서버 연결 잔존 → send_message 여전히 호출됨. 순수 채널-강제 측정은 default-deny 체제 전용(ADR-0097 기록).
- **grant:** bare name 단일 출처 = 컨트롤 채널(build_grants), 문법만 claude.rs(ADR-0004). 프라이밍의 `engram-send` 리터럴은 의도적 중복(roundtrip 내용 판정이 강제). blanket Bash·bypassPermissions를 grant 목록에 넣지 않음(auto mode는 스폰 플래그지 grant 아님).
- **측정 드라이버:** 긴 순차 루프를 *백그라운드 서브에이전트*에 맡기면 자기 작업을 백그라운드로 던지고 죽음(이 세션 1회 발생) → *내가 제어하는 백그라운드 셸 스크립트*(PowerShell run_in_background)가 견고. 집계는 in-memory 말고 **파일 스캔**(phase2 집계 버그 겪음).

## 정지 조건
- 채널 capability 스위치는 굵은 결정 → **ADR 먼저**, 구현 갈림길(capability 플래그 위치·프라이밍 분기 방식)은 사용자 확인.
- push는 사용자 결정(5커밋 미push).
- 리뷰어 정면 대립(FIX vs BLOCK)·근거 없는 BLOCK = 사용자 에스컬레이션.

## 미결(carry-over)
- **전 LLM 공용 제약 레이어** (사용자 구상, step-log 백로그) — auto mode의 정식 대체. 공용 셋팅 정본→LLM별 설정 파일 materialize, 셋+상속 조합 주입. Windows 경계(WSL2/컨테이너+PTY) 스파이크가 전제. **auto mode = 임시 체제**(신뢰 환경 전용 — 인젝션 안전 상한 = 미래 경계 품질).
- **CI 도입** (사용자 계획, "아직 아님") — TDD 3중 강제(impl 만들게·review 충분한가·qa 돌게)로 스위트 누적 중이라 CI-ready. 게이트 명령 = `/qa` 바인딩 정본. 함정: 루트 bare cargo test 금지 + 측정 bin(roundtrip/priming/saturation)은 실 claude 스폰이라 CI 밖(순수 유닛만 CI). 도입 시 GitHub Actions 스캐폴드 = qa 바인딩에서 뽑기.
- **MCP 토큰 비용** — 이번에 정성 정리(MCP=낮은지연·standing 툴스키마 토큰비용 / CLI=프로세스스폰지연·standing 0·범용). 정확 수치는 stream-json usage 측정 미실행.
- **both 폴백 진짜 MCP-부재 측정** · **opus 채널 측정** · **봉투 포맷 영속화**.

## 참조
- **ADR:** 0098(grant 정렬)·0097(auto mode) 둘 다 Amends 0094 · 0096(봉투 스위치)·0095(colon/xml)·0086(듀얼입구·단일wrap)·0030/0002(capability 매트릭스 — 채널 스위치 구현 자리)·0029(데몬 소유)·0004(백엔드 격리).
- **코드:** `backend/claude.rs`(`build_spec` auto pair `//ADR-0097` · `grants_to_allowed_tools` `//ADR-0098` Bash+PowerShell colon-star · PATH 주입) · `control/mod.rs::build_grants`(bare name) · `bin/engram-send.rs`(얇은 HTTP 클라 = 데몬 /control/send POST) · `agent/types.rs`(ToolGrant·ControlCaps — 스위치 플래그 자리).
- **프라이밍:** `prompts/experiments/agent-priming-routing-v3-en{,-cli,-both}.md`(3종 분리) · 프로덕션 `prompts/agent-priming.md`(=v3-en, MCP).
- **측정:** `.codex/phase*-summary.txt`·`phase*-runs/`(disposable 스크래치 — 재측정 시 참고, 청소 가능).
- `docs/process/step-log.md` 최하단 S17 항목(2026-07-22) · `docs/decisions/README.md` 인덱스.
