# 핸드오프: S17 봉투 스위치(ADR-0096)·영어 v3 프라이밍 채택 완료 — 후속(both-teaching·O3·colon개행) 대기

> 직전 핸드오프(4ac8c70) 대비: 갈림길 2개 해소 + 구현 슬라이스 3개 완주. 이 파일이 최신 정본.

## 한 줄 상태 · 다음 첫 액션
- **상태:** 계획 슬라이스 3개 **완료·커밋**(미push 2개) — ① 봉투 colon 기본 + `set_envelope_format` invoke 스위치(ADR-0096) ② 영어 v3 프라이밍 정식 채택 ③ CLI-only 실측+발견. 워킹트리 클린(`.vs/`·`.codex/` 노이즈만).
- **다음 첫 액션(사용자 선택):**
  1. **both-teaching 프라이밍 검증** — send_message 주력 + engram-send 폴백을 함께 가르쳐도 MCP 있을 때 10/10 유지하나(MCP-less 런타임 robustness). *구체 실측감 — v3-en-both 파일 만들어 roundtrip n=10.*
  2. **O3 데몬 detect-and-nudge** 백스톱 착수 여부(설계 스케치 아래 §O3).
  3. **colon 개행 주입** 대응 결정(①의 잔여 — 아래 do-not/미결).
  4. **push 여부** — 2 commits 미push(사용자 결정).

## 이번 세션 커밋 (2개, 로컬 미push · origin/master=4ac8c70)
- `7c47947` **①** feat(envelope): colon 기본 + invoke 스위치 — ADR-0096 (18 files)
- `05d80ec` **②③** feat(priming): 영어 v3 정식 채택 + CLI-only 측정 노브 (6 files)
- 커밋은 pathspec으로 함(`.vs/` staged·`.codex/` untracked = **의도적 제외, disposable 스크래치** — 커밋메시지·blind리뷰 프롬프트).

## ① 봉투 포맷 스위치 (ADR-0096 — ADR-0095 결정5 부분개정)
- `wrap_message` 기본 bracket→**colon** `{sender}: {body}`. xml 대체 `<message from="{sender}">{body}</message>`(이스케이프 — 발신자 스푸핑 차단). msg_id는 스파이크 `{id}` placeholder만 씀.
- **데몬 전역 포맷 상태** = `ControlRegistry` AtomicU8(Release/Acquire, 기본 Colon, **재시작 colon 리셋 = 메모리만**, 영속화 백로그). `wrap_message`가 매 메시지 읽음(단일 wrap point 유지 ADR-0086).
- **스위치 = invoke `set_envelope_format(format)`** (src-tauri Tauri command → 데몬, 조종 표면 전용). `AgentCommand::SetEnvelopeFormat`. **워커 MCP 채널 미노출**(ADR-0094). **PROTOCOL_VERSION 2→3**(신 커맨드 비-tolerant additive — 구 데몬 auth 거부, 신클라 무한대기 방지).
- 게이트: 코더→`/review code full`(doc-aware[Claude]=PASS + blind[codex]=FIX 4건: xml스푸핑·버전범프·테스트약화·ordering — 2 FIX 라운드 반영)→`/qa full`(cdp: 커맨드등록·arg검증·구v2데몬 버전거부 실관측·happy-path Ack).

## ② 프라이밍 (prompts/agent-priming.md = 영어 v3)
- 구버전(옛 bracket 봉투 서술·"인젝션 아니다" 방어문구·HTML 메타주석 = **전부 v3 do-not**, colon전환으로 drift) → **영어 v3**. 라우팅 4속성 유지: 출력불가시성·원칙자앵커·사전승인귀속·자기판단.
- 게이트: `/review doc light` PASS(do-not 재유입 0·4속성 보존). 실험 아티팩트 = `prompts/experiments/agent-priming-routing-v3-en{,-cli}.md`.

## ③ CLI-only 실측 + ★발견 (sonnet, roundtrip-smoke, 봉투=colon)
| MCP 노출 | 프라이밍 채널 | 결과 |
|---|---|---|
| 있음 | send_message(v3) | **mcp 10/10** ✅ |
| 없앰(--disallow-mcp) | send_message(v3) | none 0/1 |
| 없앰 | engram-send(v3-cli) | cli 1/1 ✅ |
| 있음 | engram-send(v3-cli) | none ~6/7 |
- **발견:** ① 영어화 라우팅 회귀 0(10/10) ② CLI 입구 격리 실증 ③ **프라이밍이 가리키는 채널 = 실제 쥔 주력 도구여야 함**(어긋나면 ~6/7 미전송, 지시-도구 불일치로 얼어붙음).
- 하네스: `--disallow-mcp` + `ENGRAM_DISALLOW_MCP_SEND` seam(`control/mod.rs::build_grants` — env게이트·remove-only·ADR-0094).

## §O3 데몬 detect-and-nudge 백스톱 (설계 스케치 — 대화에서 도출, 미박제·미구현)
- **왜:** v3 프레이밍은 확률적(말빨) — 잔여 non-send 실패 남음(특히 약모델). 기계적 백스톱 필요.
- **설계:** 발신 시 `expect_reply` 플래그(보내는 쪽이 답신 기대 여부 표시 — "질문 vs 실행지시" 구분 해결) → 데몬이 수신자 **턴 수** 감시(벽시계 아님) → N턴 내 답신 없으면 **nudge 주입**(데몬이 B stdin에 "너 메시지 받고 답 안 함" 턴 주입) 1~2회 → 그래도 침묵이면 **오케스트레이터/사용자 에스컬레이션**(무한 nudge 루프 방지).
- **관측 primitive 현황:** `DeliveryObserver` 훅 존재(전송 이벤트 발화, `control/ingress.rs:43`) — **지금은 roundtrip 하네스만 사용**. 없는 것: 비-전송 감지·`expect_reply` 개념·nudge 루프. (전송은 봄, 비-전송은 못 봄.)
- **주의:** "B 답 안 함=실패"로 단순 짜면 실행지시마다 헛재촉(false positive) → expect_reply 필수.

## 검증 상태 (쌍)
- **한 것:** `cargo build`·`cargo test`(전멤버 0실패)·`fmt --check`·코어 격리(tauri import 0)·`tsc --noEmit`·`vitest`(621) 전부 green. cdp E2E(set_envelope_format Ack·구v2데몬 버전거부·negative arg검증). roundtrip 매칭률(위 표, n=10+).
- **재실행 명령:** `cargo test -p engram-dashboard-daemon --features test-harness` · `cargo test -p engram-dashboard-protocol` · fmt `cargo fmt --check` · 격리 `rg "use tauri" crates/engram-dashboard-core/src/`(0줄=PASS, `//!` 주석 1건은 게이트 설명). roundtrip: `cargo run -q -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --priming <file> --model sonnet [--disallow-mcp]`.
- **안 한 것:** ① both-teaching 프라이밍 미검증 ② O3 미구현 ③ colon 개행 주입 미대응 ④ 영어판 opus/haiku 미측정(sonnet만) ⑤ 봉투 포맷 영속화 미구현(메모리만) ⑥ 2 commits push 안 함 ⑦ 사람용 스위치 트리거(단축키/UI) 미구현.

## do-not (누적)
- **프라이밍:** 메타주석·"인젝션 아니다" 방어문구·봉투 본문 내 행동 지시문·하드코딩 봉투포맷 금지(전부 라우팅 역효과·실측). **채널 지시는 실제 쥔 주력 도구와 일치**(불일치=~6/7 미전송).
- **봉투:** 조립=`wrap_message` 단일 지점(ADR-0086). `ENGRAM_WRAP_FORMAT`(봉투)·`ENGRAM_DISALLOW_MCP_SEND`(grant) = **스파이크/테스트 전용 env seam**, 운영 스위치 아님.
- **제어:** 포맷 스위치=invoke 조종 표면만(워커 MCP 노출 금지 ADR-0094). 비-tolerant 커맨드 추가 시 PROTOCOL_VERSION 범프.
- **명령:** 루트 bare `cargo test` 금지(src-tauri WebView2 크래시 — `-p` 명시). git commit은 pathspec(`.vs/`·`.codex/` 제외). roundtrip 스모크 **병렬 금지**(데몬 포트파일 충돌 — 순차). QA용 dev앱 스폰 후 데몬 프로세스 남으면 exe 락 → 빌드 전 `Stop-Process -Name engram-dashboard-daemon`.

## 정지 조건
- 후속 3개(both-teaching·O3·colon개행)는 방향·결정 걸림 → 착수 전 사용자 확인.
- push는 사용자 결정(2 commits 미push).
- 리뷰어 정면 대립(FIX vs BLOCK) = 사용자 에스컬레이션.

## 미결(carry-over)
- **MCP 토큰 비용 감** — 직전 핸드오프 (a) 항목. 이번 세션은 구현 경로 택해 미해결(개념 Q&A는 많이 했으나 토큰 실측은 안 함). 재등장 시 stream-json usage 대조 스파이크 제안.
- **both-teaching** 결정(프로덕션 프라이밍이 MCP-only vs MCP+CLI폴백) — ③ 발견으로 갈림길화, 위 다음액션 1.

## 참조
- **ADR-0096**(스위치)·ADR-0095(colon/xml)·ADR-0086(단일 wrap)·ADR-0094(최소권한)·ADR-0029(데몬 소유).
- 코드: `crates/engram-dashboard-daemon/src/control/ingress.rs::wrap_message`(봉투·이스케이프) · `.../control/registry.rs`(전역 상태) · `.../control/mod.rs::build_grants`(MCP seam) · `.../connection_core.rs`(dispatch) · `.../bin/roundtrip_smoke.rs`(하네스·매칭률). src-tauri `commands/agent.rs`(set_envelope_format).
- 프라이밍: `prompts/agent-priming.md`(프로덕션 v3) · `prompts/experiments/agent-priming-routing-v3-en{,-cli}.md`.
- `docs/process/step-log.md` 최하단 3개 S17 항목 · `docs/research/agent-send-routing-reliability-2026-07-21.md`(v1→v3 근거).
