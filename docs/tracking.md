# 추적 항목 (Deferred / 의사결정 보류)

코드 구현 중 발견된, 나중에 반드시 다시 다뤄야 할 항목들. 폐기와 보류를 명확히 구분한다.

## 보류 (재도입 예정)

### T-1. 로그 API 키 마스킹 (mask_api_keys) — 보안
- **상태:** 보류 (폐기 아님)
- **출처:** LLD §14 `LogConfig{mask_api_keys}`. 모듈 6a logging core 구현 시 단순화하며 누락(ed12 브리핑 실수). dr26 리뷰 지적.
- **현재 위험:** 낮음. 기본 로그 OFF(warn)라 PTY 출력이 로그로 흐르지 않음.
- **재도입 시점:** `set_log_level("debug")`로 PTY 내용이 로그에 찍힐 수 있는 때 — headless 테스트(모듈 6b)의 debug 로깅 또는 디버깅 단계.
- **요구사항:** 로그 출력에서 API 키/토큰 패턴(sk-…, Bearer …, Anthropic 키 등) 마스킹. 조직 보안룰(자격증명 산출물 포함 금지)과 직결.
- **담당 결정:** ed12 — 모듈 6b 착수 전 재검토.

### T-2. 프론트 seq dedup 확인 — Phase 3
- **상태:** 전제 조건, Phase 3에서 검증
- **출처:** dr26 session.rs 리뷰. drain이 replay push와 subscribers lock 취득 사이에 subscribe가 끼면 새 sink에 같은 seq 중복 전달 가능(§7 고유 속성).
- **방어:** 프론트가 seq로 dedup (frontend-integration-lld.md G-2: lastSeqRef). 백엔드는 이 전제 위에 설계됨.
- **조치:** TerminalSlot.tsx 구현 시 lastSeqRef dedup이 실제로 들어갔는지 확인.

### T-3. tauri 버전 핀 결정 — ✅ 해소 (2026-06-11, 최신 2.x 유지)
- **결정:** Channel spike 실측 PASS(1000/1000 무손실, #11421 Windows 미재현) → **최신 2.x(2.11.2) 유지 확정.** Cargo.toml `tauri = "2"`. LLD "2.5 금지"는 폐기(Windows WebView2 무관 실측 확인).
- **(이력) 상태:** 미결정. Phase 2(commands/lib.rs) 착수 전 결정.
- **발견:** Cargo.toml `tauri = "2.4"` 는 caret semver라 실제 2.11.2로 resolve. LLD가 피하려던 "2.5+ Channel silent failure"가 포함될 수 있음(dco23 spike 중 발견).
- **조사(2026-06-11):** LLD 인용 이슈 #13721은 검색에서 미확인(번호 부정확 추정). 실제 Channel 이슈:
  - #11421 Channel 1회만 전송 — **Linux Gnome 특정**(우리는 Windows WebView2, 무관 가능성 높음)
  - #10901 webview 미수신 시 send 비실패 — feature request
  - #13133 Channel 메모리 누수
- **권고:** Windows 타겟이라 핵심 이슈가 무관할 가능성. spike 방식으로 **실측 결정** — Phase 2에서 Tauri Channel 연속 send + 창 닫힘 시 동작 소규모 테스트. 우리 설계는 send 실패 감지에만 의존 안 함(명시적 unsubscribe M2 + replay).
- **선택지:** (A) `=2.4.x` 정확 핀(보수적) (B) 최신 2.x + 실측 검증. 실측 결과로 택일.
- **담당:** ed12 — Phase 2 착수 전.

### T-5. monaco TS worker optimizeDeps — Phase 2
- **상태:** Phase 2 monaco 통합 시 검토.
- **출처:** Channel spike 중 vite 경고 'ts.worker.js optimizeDeps 미존재'. spike엔 무해.
- **조치:** monaco diff editor 통합 시 `optimizeDeps.exclude`에 monaco worker 추가 검토.

### T-6. cwd 워크스페이스 검증 (CwdDenied) — 보안 보류
- **상태:** 보류(폐기 아님). 실제 멀티 에이전트 운영 단계에서 정책 결정.
- **출처:** dr26 commands+lib 리뷰. PtyError::CwdDenied가 dead variant — spawn_agent가 임의 cwd 허용(검증 없음).
- **위험:** 에이전트를 임의 디렉토리에서 spawn 가능. 현재 개발/검증 단계엔 과하나 운영 시 워크스페이스 화이트리스트 필요.
- **재도입:** 운영 단계에서 허용 cwd 정책(예: Engram 워크스페이스 하위만) 결정 후 manager.spawn_agent 또는 command 층에서 검증.

### T-7. get_agent_snapshot wire 포맷 — Phase 3 info
- **상태:** Phase 3에서 snapshot 사용 여부에 따라 결정.
- **출처:** dr26 리뷰. get_agent_snapshot이 PtyChunk(data:Vec<u8>→JSON number[]) 반환 — live PtyEvent의 base64와 불일치.
- **조치:** subscribe가 replay를 자동 전송하므로 프론트가 snapshot 안 쓰면 무관. 쓸 거면 base64로 포맷 통일.

### T-8. shutdown_all 순차 종료 지연 — info
- **상태:** 허용(현재). 거슬리면 추후 개선.
- **출처:** dr26 ExitRequested 재확인. shutdown_all이 agent마다 kill_agent의 recv_timeout(5s) 순차 누적 → drain hang 시 N개=5Ns 지연. 정상 경로는 즉시(spike 23ms 실측).
- **조치:** 다수 에이전트 + 비정상 hang 동시 발생 시에만 체감. 필요 시 병렬 kill(스레드/join) 또는 timeout 단축.

## 결정 완료 (기록용)

### R-1. Exiting 상태 살림 (옵션 A)
- dr26 manager 리뷰: Running→Exiting 전이가 코드에 없어 Exiting이 dead variant.
- **결정(ed12): 살린다.** kill_agent 맨 앞에서 status lock으로 Exiting 설정 + status_changed(Exiting) 알림.
- 근거: kill worst case 5초간 "종료 중" UX 필요, 프론트 타입에 이미 존재, terminal 가드로 race 안전.
- **알림 분담 갱신:** 과도기 Exiting = kill_agent / terminal(Killed·Exited·Failed) = drain 단독.

### T-4. 프론트 terminal 판정 — Phase 3
- **상태:** Phase 3 프론트 주의사항.
- **출처:** dr26 Exiting 재확인. kill의 Exiting 알림과 drain의 terminal 알림이 lock 밖 동시 발생 → 프론트가 status_changed를 역순 수신 가능. 직후 agent_list_updated가 정정함.
- **조치:** 프론트는 `status_changed`만으로 terminal(종료) 판정하지 말 것. agent_list_updated(목록에서 사라짐) 또는 명시적 종료 신호로 판정. eventBus/store 구현 시 반영.

## LLD 문서 갱신 — ✅ 완료 (2026-06-11, 코드 반영)

§9(R-1 알림분담+Exiting), §10(D-3 poison fail-fast 규칙4), §13(D-2 JobObject API), §14(D-1 로깅+mask_api_keys 보류), §6(D-4 drain 4인자) 모두 backend-lld-stage1.md에 반영 완료.

---

### (이력) 갱신 항목 상세

### D-4. §6 drain 시그니처
- LLD §6: `spawn_drain_thread(session, reader)` 2인자.
- 실제 코드: `(session, reader, status_sink, done_tx)` 4인자. §4 PtySession에 status_sink/done_tx 필드가 없어 LLD 자체 모순을 코드가 해소(drain이 종료 알림+완료신호 보내려면 필요). dr26 권고.
- 조치: §6 시그니처를 4인자로 갱신.

### D-3. poison 정책 명문화
- 코드: 모든 Mutex poison을 expect 패닉(fail-fast)으로 일관 처리.
- LLD에 poison 정책 명세 없음. dr26 권고: fail-fast 채택을 한 줄 명문화.
- 조치: LLD §10에 "Mutex poison = fail-fast(패닉), 복구 안 함" 명시.


### D-1. §14 로깅 명세
- LLD §14: ENGRAM_LOG · 기본 INFO · LogConfig{mask_api_keys} · handle 반환 · verbose-log feature
- 실제 코드: RUST_LOG · 기본 warn(=OFF, 사용자 요구) · 전역 OnceLock · 인자 없음
- 조치: §14를 코드에 맞춰 갱신. 단 mask_api_keys는 T-1로 보류 명시.

### D-2. §13 JobObjectHandle API 형태
- LLD §13: `create_and_assign(pid) -> Result<Self, PtyError>` 단일 함수
- 실제 코드: `new()` / `assign()` / `terminate()` 분리 + io::Result (에러 처리에 유리, spike 실측 통과)
- 조치: §13을 분리 API 형태로 갱신.
