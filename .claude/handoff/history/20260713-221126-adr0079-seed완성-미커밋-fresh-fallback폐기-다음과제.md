# 핸드오프: ADR-0079(JSON resume 대화 스크롤백 seed) **코드 완성·미커밋**. E2E는 빈 화면 — 원인 = resume "조기종료→fresh-fallback"이 seed된 세션을 폐기. **사용자 결정 = fresh-fallback(자동 초기화) 폐기** → 새 ADR + 고위험 구현이 다음 과제

## 한 줄 상태 · 다음 첫 액션
- **상태:** ADR-0079 seed 구현 완료 — 전 코드 게이트 + `/review code deep`(Claude doc-aware + Codex blind, FIX 2라운드 반영) PASS. **전부 미커밋(워킹트리).** 그러나 `/qa full` **GUI 실측 = 빈 화면(FAIL)**. 원인 규명됨: resume 프로세스가 3초 안에 종료→`fresh-fallback`이 seed된 세션을 버리고 빈 fresh 세션으로 교체.
- **다음 첫 액션:** 사용자가 정한 **"fresh-fallback(자동 초기화) 폐기"** 방향을 **새 ADR로 박고 구현**. 착수 전 2가지: ① **범위 미확정** — 전면(터미널+JSON·부팅복원+수동활성화 전부) vs JSON 먼저 → **사용자에게 물을 것**. ② **우리 케이스가 (a) M-1 오판(JSON 정상 종료를 실패로 오판) vs (b) 진짜 resume 실패(stdin EOF/SID 문제) 인지 미확정** → 재개 JSON 프로세스의 **실제 exit code/stderr 실측**해 확정(둘 다 빈 화면이지만 함의 다름).

## 사용자 결정 (박제 대기 — 새 ADR 감, ADR-0076/0077 개정)
- **resume 에러 시 자동 fresh 세션 생성 폐기.** 실패한 세션을 **에러 상태 그대로 남긴다**(자동 초기화 X). 사용자가 보고 삭제/재시도 판단.
- **실패 원인 로그로 남긴다** — 사용자·Claude 분석 가능하게. 구체: claude 에러를 `Error` 이벤트로 세션 출력에 표면화(디코더가 이미 함) + tracing 로그(logging-conventions) + `TerminalReason` 기록 → UI·로그·상태 3곳.
- (목표) 중요 자료 복원 가능하게 — `.jsonl`은 claude가 디스크에 계속 보유, 자동 초기화 안 하면 옛 세션 안 버림.
- **ADR-0079 seed와 시너지:** seed는 pump 전에 과거를 버퍼에 채우므로, 자동 초기화만 없애면 **에러 나도 [과거 대화(seed) + 에러 메시지] 함께 표시** → "자료 보이고+원인 보이고+사용자 결정" 그림 완성.
- **미결:** 범위(전면 vs JSON먼저). **판정을 "타이밍(3초)"이 아니라 "명확한 원인(No conversation found 등)"으로** 하자는 게 사용자 원칙 — 애매/일시적(Anthropic 일시 장애 등)은 초기화 말 것.

## 완료 (이 세션 · 전부 미커밋 · 워킹트리)
- **ADR 문서:** `docs/decisions/0079-jsonrichslot-모드-...-history-프레임으로-전달.md`(본문은 seed 방식으로 갱신됨 — 파일명 슬러그만 옛 "History 프레임" 잔존) + `docs/decisions/README.md` 인덱스 갱신. lint clean(advisory 5=기존 레거시).
- **코드 (미커밋 — `crates/engram-dashboard-core/src/agent/`):**
  - `output_core.rs`: `OutputCore::seed(events)`(Ring.push+seq, fanout 없음) 신설 · **`emit()` seq 발급을 replay 락 안으로 이동**(동시 emit 단조성 — Codex 적출, partition_point 전제 보호) · 신규 테스트(seed·동시emit 단조).
  - `backend/claude.rs`: `project_slug`(cwd 비영숫자→`-`, 45개 실측 일치)·`transcript_path`(`~/.claude/projects/<slug>/<sid>.jsonl`, `CLAUDE_CONFIG_DIR` 존중)·`parse_transcript_events`(순수, **기존 `ClaudeStreamDecoder::consume_line` 재사용**, `isSidechain` 스킵)·`read_transcript_events`(tail 4MB, 부분 첫 라인 폐기, `take(4MB)` 경계) + 테스트 + `ENV_LOCK`.
  - `backend/mod.rs`: `resume_transcript_events(command,cwd,sid)` dispatch(json-mode claude만).
  - `manager.rs`: `spawn_session`가 **맵 insert·pump 전에** `core.seed()` 호출(seed-before-publish, Codex 창 폐쇄). `AgentSession::seed` 래퍼는 제거(manager가 core 직접 seed).
  - `backend/fixtures/claude_transcript.jsonl`: 신규 픽스처(멀티블록 턴·sidechain·summary·file-history-snapshot 등).
- **게이트:** review deep PASS(FIX 라운드: seed-publish 창·tail 부분라인·bounded read·emit race·무효테스트 전부 반영·재리뷰 PASS). qa full = 코드 6/6 PASS(build·core/protocol test·fmt·격리0·tsc·npm613), **GUI 실측 FAIL**.

## 검증 상태 (쌍)
- **돌림(green):** `cargo test -p engram-dashboard-core` · `cargo test -p engram-dashboard-protocol` · `cargo fmt --check` · `rg "use tauri" crates/engram-dashboard-core/src/`(=0, lib.rs doc주석 1건은 import 아님) · `npx tsc --noEmit`(0) · `npm test`(613). **재실행 = 이 명령들(루트, member-scoped만).**
- **검증 안 됨:** **E2E 동작** — resume 후 RichSlot에 스크롤백 실제 표시. GUI 실측서 빈 화면(fresh-fallback 원인). (a)/(b) 원인 미확정. seed→화면까지의 정상 경로는 한 번도 관측 못 함.

## 실패한 접근 / do-not
- **bare `cargo test`·`cargo test -p engram-dashboard` = WebView2 크래시** → member-scoped(`-p engram-dashboard-core`/`-p engram-dashboard-protocol`)만.
- 실행 중 앱 있으면 `cargo build` **파일락**(daemon `.exe`) → 재빌드 전 `taskkill` daemon+client(+stale vite 1420).
- GUI 실측은 **워킹트리 앱**(`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev` + `scripts/cdp.mjs`). **`run-dashboard-clean.bat`은 HEAD 재빌드**라 미커밋 변경을 검증 못 함.
- fresh-fallback 원인을 **매직넘버(3초 튜닝)로 우회 금지** — 원인 기반 판정으로 재설계(ADR-0038 정신).

## 정지 조건 (다음 세션)
- fresh-fallback은 **kill/lifetime 고위험 경로**(ADR-0076/0077, 부팅복원+수동활성화·터미널+JSON 공유) — 범위·구현 접근을 **임의 확정 말고 사용자 확인**.
- **워킹트리 미커밋 변경 discard 금지**(ADR-0079 완성 코드 + 문서). 커밋 여부도 사용자 판단(qa E2E 미통과라 이 세션서 커밋 안 함).

## 참조 (읽을 것)
- **코드 앵커:** `manager.rs:43-45`(EARLY_EXIT_WINDOW 3s), `manager.rs:481-498`(`resume_with_fresh_fallback` + ★fable M-1★ "성공 resume=TUI라 안 죽음" 전제 — JSON에서 틀림), `manager.rs:551`(`early_terminal_status`), `output_core.rs` `emit()`/`seed()`, `backend/claude.rs` decoder(`consume_line`/`consume_block`)·transcript 함수.
- **ADR:** 0079(seed, 이번) · **0076/0077(fresh-fallback — 개정 대상)** · 0008(sid 통제 resume) · 0044(stream-json 배선·디코더·`--replay-user-messages`) · 0046(view-scoped replay·gen fence·seq dedup) · 0006(락 순서) · 0005(finalize 1회).
- **리서치 자산(재사용 — 재조사 불필요):** Claude Code 래퍼 부류 표준 = resume 시 `.jsonl` 직접 읽어 재구성. Crystal=자체 스토어 단일소스, Claudia/opcode(우리 스택)=`.jsonl` seed-후-append(경계 race 버그). **CLI 실측: `claude --resume`(stream-json)은 과거를 stdout 재방출 안 함, 재방출 flag 부재. `--replay-user-messages`는 새 stdin 에코 전용(우리 이미 사용).**
- 앱 실행: `run-dashboard-clean.bat`(데몬 HEAD 재빌드+dev, 디버그포트 9223) — 단 **미커밋 검증엔 부적합**(HEAD 빌드). 미커밋 검증은 워킹트리 `npm run tauri dev`.
