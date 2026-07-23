# 핸드오프: ADR-0099 채널 capability 스위치 완주(구현·라이브실측·push) — 다음 = 릴리즈 빌드 실사용 확인

## 한 줄 상태 · 다음 첫 액션
- **상태:** ADR-0099 채널 capability 스위치 **완주** — 구현·리뷰 3라운드·QA·라이브 실측·커밋·**push 완료**(HEAD `e02b7db`, origin/master 동기화, 워킹트리 클린). 이월 측정(both 폴백·opus)도 마감. 미커밋·미push 없음.
- **다음 첫 액션(사용자 확정):** **릴리즈 빌드를 뽑아 실사용 확인** — "사용자가 대시보드에서 에이전트에게 명령"이 실제로 되는지가 최초 관문. 그 체감 후 다음 방향 결정: **오케스트라 고도화 vs 제어 고도화(S17 재개) 중 택1**(사용자 결정 — 지금 미정).

## 릴리즈 확인 체크포인트 (알려진 지뢰 — 이 세션에서 식별, 미검증)
- 릴리즈 번들에 **daemon exe · engram-send.exe · `prompts/` 폴더**가 동봉되는가. 특히:
  - `prompts/agent-priming.md`·`agent-priming-cli.md`는 **exe-상대경로**로 해석(`FilePrimingProvider` FIXED_RELATIVE) — 번들 누락 시 **에러 없이 조용히** 프라이밍 없는 에이전트가 스폰됨(발신법 모르는 팀원).
  - engram-send는 **데몬 exe 형제 디렉토리** 규약(PATH 주입이 부모 dir prepend) — 배치 어긋나면 CLI 채널 사망(비-MCP면 fail-closed로 스폰 실패 = 시끄러움, MCP-capable이면 폴백만 죽음 = 경고 로그).
- tauri 번들 설정(tauri.conf.json)에 위 배치가 반영돼 있는지 **한 번도 확인된 적 없음**(릴리즈 빌드 자체가 미검증 영역일 수 있음).
- 3프로세스 토폴로지(tray-host + detached 데몬 + UI, ADR-0023)가 릴리즈에서 성립하는지.

## 이번 세션 완료분 (전부 push — 주요 커밋)
- `56bfe69` **feat: ADR-0099 채널 capability 스위치** — backend `accepts_mcp_config`(claude=true, dispatch는 `backend_for` 단일 match) → provision 분기: MCP-capable=mcp-config 기록+both 프라이밍 / 비-MCP=**config 미생성**(`config_path: Option<PathBuf>`=None 타입 인코딩)+CLI 프라이밍. grants 채널별([Mcp,Cli] vs [Cli]). 프라이밍 정적 2파일 승격(`prompts/agent-priming.md`=both-teaching / `agent-priming-cli.md`=CLI-only·send_message 단어 부재). fail-closed(비-MCP&&send_exe=None=ProvisionError). 측정 seam `ENGRAM_FORCE_CLI_ONLY_SEND` + roundtrip `--cli-only`(strict 판정: b_sent&&entrance=cli). 실험 프라이밍 10파일 삭제·C1~C3 별칭 제거.
- `d74417d` **test: 배선 tripwire** — `expected_channel_matrix`(와일드카드 없는 exhaustive match, backend/mod.rs 테스트 모듈). 새 AgentCommand variant 추가 시 **컴파일부터 깨져** capability 의식적 선언 강제(체크리스트는 그 자리 주석 = 실행물이라 rot 안 함). CI 도입 시 자동 발화.
- `a0fa80c` chore: .vs 스테이징 해제+ignore(이후 폴더 삭제)·.codex ignore(이후 삭제)·핸드오프 기록 커밋.
- `e02b7db` docs: 측정 마감 기록.
- **비-repo:** `I:\Study\`(우산) + `I:\Study\testing\`(테스트 심화 학습 스캐폴드 — 별개 git repo, CLAUDE.md에 학습 계약·커리큘럼 8챕터. 사용자가 내부에서 고도화 예정).

## 라이브 실측 결과 (이 세션)
- **cli-only(진짜 MCP-부재, sonnet): 3/3 PASS** — 옛 노브(grant만 제거)로 불가능했던 측정이 물리 스위치로 처음 가능해짐.
- **MCP 기본(sonnet): 1/1** entrance=mcp(회귀 0).
- **both 폴백(이월 백로그 해소): 3/3** — `ENGRAM_FORCE_CLI_ONLY_SEND` 수동 + `--priming agent-priming.md`(--cli-only 아님) 조합 = MCP 물리 부재 + both 프라이밍 → 폴백 조항대로 cli 발신.
- **opus(이월 해소): cli-only 2/2 · MCP 1/1** — 전 모델(sonnet/haiku/opus) 스위치 준수.
- **CLI 지연 정량:** engram-send.exe warm ~33ms(PS 직접)/~110–130ms(bash 내)/전체 체인 추정 200–500ms — 툴 실행은 추론과 직렬이나 발신당 추론 수 초 대비 몇 % → "CLI 느림"은 결정 변수 탈락.

## 검증 상태 (쌍)
- **돌린 것:** `/review code full` 3라운드(doc-aware+codex blind — R1 codex BLOCK은 사용자 결정 "claude 하나 MCP 떼고 테스트"=seam으로 해소) · `/qa standard`(build·전 멤버 회귀 28+12스위트·fmt·격리·tsc·vitest 621) · tripwire는 `/implement simple`(review light+qa quick). 재실행: `cargo test -p engram-dashboard-core` · `-p engram-dashboard-daemon --features test-harness` · `cargo fmt --check` · 격리 = Grep "use tauri" core src(lib.rs 주석 1건 = baseline PASS) · roundtrip: `cargo run -q -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- [--cli-only] [--priming <파일>] --model <m>`.
- **안 한 것:** ① 봉투 포맷 영속화(ENGRAM_WRAP_FORMAT 메모리만 — **저장 위치 = 사용자 결정 필요**) ② 릴리즈 빌드 검증(위 체크포인트 전부 미검증) ③ codex/gemini 백엔드 연결(의도적 — "codex 본격 사용 시" 사용자 결정, tripwire가 대기) ④ CI(사용자가 주말 직접 진행 — GitHub Actions·**windows-latest 러너**(OS 의존 테스트)·측정 bin 제외 권고 전달됨) ⑤ OS 이식성: claude.rs PATH 테스트 fixture가 Windows풍(`;` 구분자) — Linux 러너면 깨질 것.

## do-not (누적 + 이 세션 신규)
- **기존:** 루트 bare `cargo test` 금지(-p 스코프). roundtrip 병렬 금지(포트파일 충돌 — 순차 + 실행 전 `Stop-Process -Name engram-dashboard-daemon`). `engram-roundtrip-*` temp 누수(청소). `--disallow-mcp`는 grant만 제거(auto mode에서 무력).
- **신규:** ① `ENGRAM_FORCE_CLI_ONLY_SEND` + `ENGRAM_PRIMING_FILE` 수동 조합 = 짝 불변식(깐 채널=가르친 채널) 위반 가능 — **의도적 측정에만**(both 폴백 측정이 그 케이스). `--cli-only` 모드는 상속 env를 SETUP-FAIL로 거부함(정상). ② rg가 PowerShell PATH에 없음 — 격리 게이트는 Grep 툴/bash로. ③ 측정 축 구분: `--disallow-mcp`(grant)와 `--cli-only`(물리)는 다른 축.

## 정지 조건
- 다음 방향(오케스트라 vs 제어 고도화) = **사용자 결정** — 릴리즈 실사용 체감 후.
- 봉투 영속화 저장 위치 = 사용자 결정.
- 리뷰 정면 대립·근거 없는 BLOCK = 사용자 에스컬레이션(이 세션 선례: codex BLOCK → 사용자 절충안 채택).

## 미결 (carry-over)
- 봉투 포맷 영속화 · CI 도입(사용자 직접) · codex CLI spike→백엔드 연결(비-MCP 첫 실소비자) · 전 LLM 공용 제약 레이어(auto mode 정식 대체 — Windows 경계 스파이크 전제) · 풀 메일박스(수신함·영속·ACK) · O3 detect-and-nudge · S17 제어 표면 재개(UI relay ADR-0081 구현 + 슬라이스2 obs/view 인터페이스 표).

## 참조
- **ADR:** 0099(이번 결정 정본 — 거부 대안: hard 단일채널·동적 용어집 치환·CLI 보편화) · 0097(auto mode — 채널선택=프라이밍) · 0098(bare grant+PATH) · 0086(듀얼입구·단일wrap) · 0030/0004(capability·backend 격리).
- **코드:** `control/mod.rs::provision`(분기+seam `//ADR-0099`) · `control/priming.rs`(PrimingVariant) · `backend/mod.rs`(accepts_mcp_config dispatch + tripwire 테스트) · `backend/claude.rs`(Option config 주입) · `bin/roundtrip_smoke.rs`(--cli-only).
- `docs/process/step-log.md` S17 항목 3개(설계 착수·구현·측정 마감, 2026-07-22~23) · `docs/decisions/README.md`.
- 학습: `I:\Study\testing\CLAUDE.md`(테스트 심화 커리큘럼 — engram ADR-0099 테스트 20종이 실물 교재).
