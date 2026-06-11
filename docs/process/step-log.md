# Step 타임라인 — Engram Dashboard

언제·무엇을·어떻게 했는지 시간순 기록. 산출 문서와 커밋을 매핑한다.
상세는 각 폴더 참조: `design/`(설계) · `reviews/`(검증) · `briefings/`(구현 지시) · `spec/`(요구사항) · `history/`(초안).

**검증 3-게이트:** 코더(dco23 Opus / dcs24 Sonnet) → LLD 리뷰(dr26 Fable) → QA(dq25, build/lint/test).

---

## S0 — View phase (이전 세션)
- **무엇:** 더미 데이터로 UI 골격 — 테마/폰트/레이아웃/슬롯/에이전트 트리/diff/팝업 (Step 1–10 + 슬롯 동적 분할)
- **결과:** Tauri 창에 더미 대시보드 동작 (백엔드 없이 UI 검증 완료)
- **문서:** `spec/view-spec.md` (+ `view-spec-gpt-review.md`), `spec/requirements.md`, `spec/research.md`

## S1 — 백엔드 설계 (이전 세션)
- **무엇:** 백엔드 아키텍처 → 상세설계(LLD) → 프론트 통합 설계
- **어떻게:** architecture 초안 → Gemini/GPT 리뷰 → final. LLD Stage 1 작성 → **fable/Gemini/GPT 3자 adversarial 검증** → GO. frontend-integration LLD도 동일 3자 검증.
- **결과:** 구현 계약서 확정 — `pty/` Tauri 격리(OutputSink/StatusSink), kill 6단계, C4 replay→live, AppState 단일 manager
- **문서:** `design/backend-architecture.md`, `design/backend-lld-stage1.md`, `design/frontend-integration-lld.md`, `reviews/*`

## S2 — Phase 0: Spike (2026-06-11)
- **무엇:** 본 구현 전 PTY kill 시퀀스를 Windows 실기기에서 실측
- **어떻게:** dco23 — `examples/spike.rs`로 portable-pty spawn → kill 6단계 → reader join 타이밍 측정
- **결과:** ✅ PASS. `master.take()`(ConPTY 종료)로 reader가 **17ms** 내 EOF. LLD 가정 그대로.
- **문서:** `briefings/phase0-spike.md`

## S3 — Phase 1: 백엔드 PTY 코어  · 커밋 `575e36d`
- **무엇:** `pty/` 6모듈 + `logging/` + headless 테스트
- **어떻게:** m1 types → m2 session(C4 subscribe) → m3 drain(lock-밖-send) → m4 platform/windows(JobObject) → m5 manager(kill 6단계) → m6 logging → headless. 각 모듈 3-게이트 통과.
- **결과:** ✅ headless PASS — spawn→write→resize→kill, 상태 Running→Exiting→Killed, kill **23ms**. `pty/` tauri import 0. dr26이 win_err 버그·transition race·poison 정책 등 실결함 포착.
- **문서:** `briefings/m1-types` ~ `m6b-headless`, `design/backend-lld-stage1.md`

## S4 — Channel spike: tauri 핀 결정 (Phase 2 직전)
- **무엇:** tauri 버전 확정을 위한 Channel 무손실 실측 (LLD는 "2.5 금지"였으나 caret이라 2.11.2 resolve)
- **어떻게:** dco23 — 임시 `channel_spike` command로 1000회 연속 send, 프론트 수신 카운트
- **결과:** ✅ 1000/1000 무손실 (이슈 #11421은 Linux 특정, Windows WebView2 미재현). **최신 2.x(2.11.2) 유지 확정.**
- **문서:** `briefings/channel-spike.md`

## S5 — Phase 2: Tauri 연결 계층  · 커밋 `f959304`
- **무엇:** commands 8개 + `lib.rs`(AppState / ChannelOutputSink / TauriStatusSink / setup)
- **어떻게:** dcs24 — thin wrapper + OutputSink/StatusSink의 Tauri 구현. 3-게이트.
- **결과:** ✅ dead_code 전멸(코어가 command 통해 사용됨), 앱 기동. `RunEvent::ExitRequested → shutdown_all` graceful 종료.
- **문서:** `briefings/m7-commands-lib.md`

## S6 — 백엔드 마감  · 커밋 `26dc649`
- **무엇:** ① 로그 API키 마스킹(T-1) + ④ shutdown 병렬 kill(T-8). ② cwd 검증은 **claude 자체 권한과 중복이라 스킵**.
- **어떻게:** dcs24 마스킹(regex, 키 패턴 6종) / dco23 병렬 kill(`std::thread::scope`). 3-게이트.
- **결과:** ✅ debug 로깅 시 키 노출 차단, 앱 종료 worst case N×5s→5s.
- **문서:** `tracking.md` T-1 / T-6 / T-8

## S7 — Phase 3: 프론트 통합 3a–3c  · 커밋 `ca61cbd`
- **무엇:** API 레이어(types/ptyApi) → eventBus+store → TerminalSlot 실제 PTY 연결
- **어떻게:** dcs24 — 3a→3b→3c. C2 reset / T-2 seq dedup / G-1 cleanup / 입력가드 / resize debounce.
- **결과:** ✅ **첫 E2E** — 실제 창에서 claude 기동, **PTY ↔ Tauri ↔ React 전체 파이프라인 실증**. 3d(popup+monaco)는 보류.
- **문서:** `briefings/m8a-api-layer` ~ `m8d-popup`, `design/frontend-integration-lld.md`

## S8 — 문서 정리  · 커밋 `fdf6d06` + 진행 중
- **무엇:** core/dashboard 통합 → docs 트리 재편 → 과정/정설 분리 + 타임라인
- **결과:** ✅ 모든 자료 `apps/engram-dashboard` 일원화. `process/`(과정 기록) + `reference/`(정설, 추후) 분리, 이 `step-log.md` 작성.

## S9 — 세션 저장/복원 (코어 GO)  · 커밋 `8981cb9`~`7052bc2`
- **무엇:** claude 세션 무손실 복원 + 에이전트 프로필 영속화 + 약간의 추상화(claude.rs/profile.rs 분리). 자동재시작(restart_agent)은 **게이트**(설계만).
- **어떻게(H-4 순서, 매니저 직접 구현 + fable 리뷰 게이트):**
  1. `profile.rs` + ProfileRegistry(단일 소유자, sid 생성·갱신) + dunce — `8981cb9`
  2. `persistence/` atomic agents.json(tmp+sync_all+rename+parent fsync, schema_version, .corrupt 보존) — `d7f42f4`
  3. `session_tracker.rs` sid drift 폴링(best-effort, PID shim 우회 스캔, 단일 스레드+정지핸들, degraded 강등) — `0a59fa3`
  4. LLD 개정 a~g(backend §18 / frontend §11, Stage-1 보존+addendum) — `1b9499c`
  5. `claude.rs` 격리 + manager `spawn_agent(profile,mode)`/`restore_all`(백그라운드)/fallback(조기종료 윈도→fresh, 종점 Failed) — `f67476b`
  6. profile CRUD 커맨드 + 프론트 TS 미러(epoch 재구독) + **fable 리뷰 수정** — `7052bc2`
- **검증:** unit test 19, headless PASS, `cargo fmt`·`tsc` 클린, `pty/` tauri import 0.
- **fable 리뷰(조건부 GO→수정 완료):** C-1 remove_session drain 대기(stale Killed 경합 제거), M-1 resume 조기종료 code 무관 fallback, Mn-1 status_changed epoch 동봉, Mn-2 Started variant, Mn-5 단일 persist.
- **핵심 메커니즘:** spawn 시 `--session-id <uuid>`로 우리가 sid 통제 → 재시작 `--resume`로 무손실 복원. `/clear`로 sid 바뀌면 `sessions/<pid>.json` 폴링으로 따라잡아 즉시 persist. 복원 정확성은 우리 통제 sid에만 의존(추적 파일은 best-effort).
- **문서:** `S9-session-restore/session-restore-lld.md`, `-code-plan.md`(§H), `spike-results.md`, `s9-*-review-*.md`

---

## 다음 (미진행)
- **[게이트] 자동 재시작** — `restart_agent` 전용 태스크(사다리 resume→fresh→정지, backoff). 코어 안정 후.
- **실제 claude 복원 E2E** — headless는 shell만 실증. claude `--session-id`/`--resume` + `sessions/<pid>.json` PID 일치를 실제 claude로 실측(spike) 필요.
- 메시지 시스템(에이전트 간 통신) — 백엔드 추가 설계.
- Phase 3d (popup URL 전달 + monaco) + 프론트 상세(복원 배너 UX).
- `reference/` 정설 문서 집필 (시스템 안정화 후)
