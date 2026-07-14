# 추적 항목 (Deferred / 의사결정 보류)

코드 구현 중 발견된, 나중에 반드시 다시 다뤄야 할 항목들. 폐기와 보류를 명확히 구분한다.

## 보류 (재도입 예정)

### T-2. 프론트 seq dedup 확인 — Phase 3
- **상태:** 전제 조건, Phase 3에서 검증
- **출처:** dr26 session.rs 리뷰. drain이 replay push와 subscribers lock 취득 사이에 subscribe가 끼면 새 sink에 같은 seq 중복 전달 가능(§7 고유 속성).
- **방어:** 프론트가 seq로 dedup (frontend-integration-lld.md G-2: lastSeqRef). 백엔드는 이 전제 위에 설계됨.
- **조치:** TerminalSlot.tsx 구현 시 lastSeqRef dedup이 실제로 들어갔는지 확인.

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

### D-7. 레이아웃/창 영속화 저장 위치 = 프론트엔드 localStorage (2026-06-14 결정, 구현 보류)
- **결정:** 다중창 레이아웃·테마·좌표 영속화는 **프론트엔드 localStorage**에 둔다. 백엔드 파일 불필요.
- **사용 시나리오(확정):** 개인용(팀/공유 없음). 백엔드 1·프론트 1 코드베이스. **창마다 독립 레이아웃+독립 테마**(A창 슬롯4 / B창 슬롯3 식, 멀티모니터 배치용). 각 창의 `{id, x, y, w, h, theme, layout(슬롯 트리)}`를 저장 → 부팅 시 그대로 재생성·복원.
- **왜 백엔드가 아니라 프론트인가(검토 결론):**
  1. **저장 위치 ≠ 제어 표면.** §5가 요구하는 건 *커맨드(제어 표면)*의 LLM 도달성이지 상태 바이트의 백엔드 저장이 아니다. LLM은 dispatch에 커맨드만 쏘고, 저장은 프론트 구현 디테일 — 직교한다.
  2. **콜드 부팅 다중창 복원을 프론트가 할 수 있다.** main 창 JS가 localStorage 읽고 → 자기 좌표 `getCurrentWindow().setPosition()` → 나머지 창 `new WebviewWindow(id,{x,y,w,h,url})`(`@tauri-apps/api/webviewWindow`) → 각 창이 자기 슬라이스로 hydrate. `withGlobalTauri:true`. 창 생성은 Tauri JS API 호출일 뿐 데이터는 백엔드 불요.
  3. 좌표 추적도 프론트: 창 이벤트 `onMoved`/`onResized`/`onCloseRequested`에서 localStorage 갱신.
- **진짜 마찰(알고 가야 함, 해결책 동반):**
  1. localStorage는 origin 단위 창 공유 → 동시 쓰기 클로버 위험. **창 id별 키 분리**(`engram:win:<id>`)로 각 창이 자기 슬라이스만.
  2. LLM(백엔드 두뇌)이 현재 레이아웃을 *읽으려면* localStorage 직접 접근 불가 → 프론트 query 커맨드 왕복 또는 §5 임시경로(`cdp.mjs eval`). "저장"은 무문제, "읽기"만 왕복.
- **구현 순서(착수 시):** 세션 모델(창별 객체) 정의 → 좌표 추적(창 이벤트)+부팅 복원 시퀀스 → 단일 창 layout+theme persist부터 검증 → 다중창 동적 생성(`WebviewWindowBuilder`/JS `WebviewWindow`, 현 conf.json은 정적 3창 고정이라 신규 기능) 복원.
- **데몬과의 관계:** 창들이 서로 독립 상태라 "창 간 실시간 동기화" 불요 → **데몬 없이 localStorage만으로 성립.** 데몬화(에이전트 생존)와 별개 트랙.
- **보류 사유:** 데몬화를 먼저 진행하기로(2026-06-14, "걸리적거림" 해소 우선). 이 결정은 그대로 보존, 데몬 후 착수.

### D-8. 데몬화 IPC — 턴키 라이브러리 없음, 커스텀 불가피 (2026-06-14 consult, 착수 보류)
- **결론:** JS↔Rust 데몬 IPC에 핵심 불변식(replay→live 순서·seq dedup·epoch 재구독)을 공짜로 주는 성숙 라이브러리 **없음**. 커스텀 구현 불가피, 규모 MVP 1~3주 + production 3~6주+. 사용자 지시("커지면 하지 말 것")에 따라 **착수 보류**.
- **방법:** `/consult` GPT·Gemini·Claude-Opus 블라인드 + judge. 3종+judge 만장일치: 라이브러리는 transport(전송선)만, 정확성은 데몬 session core 책임.
- **합의(judge 검증 옳음):** ① replay=데몬 자체 보유(OutputCore source of truth, broker 위임 X) ② 경로=B-contract-first(JS↔데몬 WS 직결, envelope를 B 기준 고정, tarpc/jsonrpsee로 A 따로 구현 금지) ③ 커스텀=YES(OutputCore 재사용으로 0부터는 아님).
- **탈락:** tarpc(JS 미도달)·tonic/grpc-web(브라우저 bidi 불가)·jsonrpsee 재연결(손실분 식별 불가)·NATS/nats.ws(브라우저 클라 2026-05-08 아카이브).
- **★사용자 결정 필요(착수 전):** (1) raw `tokio-tungstenite` vs Socket.IO+socketioxide — judge는 raw WS(lock-in 회피·의존성 최소), 양쪽 다 seam 뒤면 후회비용 낮음 (2) 모바일 원격 TLS·인증 (3) backpressure 정책.
- **상세:** `docs/process/S12-daemonization/ipc-library-consult.md`.

### T-9. claude 프로세스 풀링 — 에이전트 부팅 속도 (보류)
- **상태:** 보류(폐기 아님). headless가 메인 경로인 한 무의미.
- **출처:** 제어표면/fleet 선행조사 논의(2026-06-21). 메모리·스폰속도 설계 갈래에서 파생.
- **결론(현재):** 워밍된 claude.exe 풀을 다른 에이전트로 retarget하려면 cwd 변경이 필요한데 headless는 런타임 cwd 변경(`/cd`) 불가 → 풀링 무의미. (인터랙티브 풀이면 `/cd`로 가능하나 비메인 경로 + 메모리 비용.) session-id는 블로커 아님(풀 슬롯을 engram-통제 sid로 부팅 후 그 sid를 정체성으로 채택).
- **재도입 시점(측정 가능):** ① cold spawn→first-output 지연이 실측상 병목(기준선 측정 후 확정, 후보 >~2s) · ② 동시 활성 에이전트 증가로 스폰 빈도↑ 누적지연 문제 · ③ "headless 메인" 전제 변경.
- **조치:** 빠른 스폰은 풀링 대신 `--bare`(autoload 제거) + warm 티어(hot 에이전트 유지). 근본 출구는 API transport(claude.exe 미사용). **막다른 길·근거 상세:** `docs/research/control-surface-and-fleet.md` §6.

### T-10. discovery crate 통합 — 구조 정리 (2026-07-05 논의, 저우선 보류)
- **상태:** 보류(급하지 않음, 착수 트리거 없음). 방향까지 합의된 상태라 재논의 불필요 — 여유 생기면 그때.
- **방향:** discovery의 `default_data_dir`(경량·공유)만 core로 내리고 나머지 앱 전용 발견 로직(ensure_daemon/status/stop/WMI)은 src-tauri 모듈로 올려 crate 삭제. 기계적 이동이라 난이도 낮음. 착수 시 ADR-0024/0029 amend + 앱 전용 테스트 이관(현 src-tauri 테스트 하네스 DLL entrypoint 이슈 동반 처리).

### T-11. child별 권한 스코프(R7) — 보안 보류, 모바일/원격 단계 (2026-07-13 결정)
- **상태:** 보류(폐기 아님, 후순위). 착수 트리거 = 신뢰경계 확장(모바일/원격 데몬). 로컬 단일 PC 단계에선 미착수.
- **출처:** S17 제어표면 PRD R7 + `/review prd` BLOCK(2026-07-13). 쟁점 = "데몬 opaque-relay(UI 의미 무지) ↔ child별 스코프" 모순.
- **배경(현행, 확실):** 데몬 auth = 단일 마스터 토큰 하나(`ws.rs` `expected_token` + `constant_time_eq`) → 통과/거부만 판정. child 신원 구분·스코프 0(통과한 연결은 전부 동일·전권). 즉 R7은 순수 신규 추가 작업.
- **왜 후순위:** 로컬 PC 부모-자식 = 단일 신뢰경계. 마스터 토큰은 portfile 접근자에게만 노출. 세밀 스코프의 가치는 다중 신뢰수준(원격·비신뢰 child)이 생길 때 발생.
- **모순이 지금 사라지는 이유:** 세밀 인가를 아무도 안 하면 데몬 opaque-relay가 깨끗함. 모순은 R7이 *현재 요구*일 때만 발생 → 보류 시 MVP 설계 무결(BLOCK + "R7 미AC" FIX 동시 해소).
- **재도입 방향(현재 lean 추정 — 확정 시 갱신):** 데몬은 per-child 토큰으로 **신원 도장**만(UI 의미 무지 유지), 인가(스코프)는 UI 권위 소유자 `ViewManager`(클라이언트)가. 거부 후보: (a) 데몬 coarse 게이트 병행 = 인가 분산 / (b) 클라 직결 엔드포인트 = 모바일 부적합.
- **순수 추가 전제:** MVP 라우팅을 relay-through-daemon(child→데몬→`ViewManager`)으로 유지해야 (a)/(lean)이 열린 채 남음(ADR-0080이 이미 그 모양). 이 전제가 foreclose하는 건 (b)뿐 — 모바일 비친화라 무해.
- **보안 담당 확인:** per-child 토큰 발급·수명·portfile ACL·배포 정책은 보안 담당 판단.
- **리뷰 재도전 + 사용자 재확인 (2026-07-14):** `/review prd`에서 cross-family(blind) 리뷰어(codex)가 이 보류를 BLOCK으로 재도전 — 근거: prompt-injection된 child 하나가 공유 마스터 토큰으로 **형제 agent kill·write·전 UI 조작**(child↔child 횡이동) 가능, "single PC"는 외부 경계만 덮고 내부 오염은 못 덮음. **사용자 재확인 = 보류 유지(수용된 알려진 위험)** — 현 작업에 지장 없음, 제약은 한참 나중. 전제 = 로컬·신뢰 콘텐츠.
- **더 큰 그림 (사용자 프레이밍 2026-07-14):** 권한/제약은 R7(child 토큰 스코프) 하나가 아니라 **오케스트레이션 제약·명령 제약까지 아우르는 별도 후기 단계**다 — 핵심 줄기와 직교하며 나중에 통째로 착수. T-11은 그 제약 레이어의 첫 항목으로 본다.

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

---

## 해소됨 (아카이브)

### T-1. 로그 API 키 마스킹 — ✅ 구현 (2026-06-11)
- **구현:** logging/mod.rs `mask_secrets` (regex). 커버: Bearer, Anthropic sk-ant-, OpenAI sk-/sk-proj-, AWS AccessKeyID AKIA, GitHub ghp_/gho_/github_pat_, Google AIza. LogSink에 적용. dr26 LGTM.
- **한계(문서화):** AWS Secret Key(40자 base64)는 패턴 식별 불가 — best-effort. generic api_key= 는 오탐 리스크로 미적용.
- **규칙(명문화 필요):** 추후 production에 PTY 텍스트 로그 추가 시 반드시 mask_secrets 적용 → CLAUDE.md/LLD 명시 예정(D-6).

### (구) T-1 보류 메모
- **상태:** 보류 (폐기 아님)
- **출처:** LLD §14 `LogConfig{mask_api_keys}`. 모듈 6a logging core 구현 시 단순화하며 누락(ed12 브리핑 실수). dr26 리뷰 지적.
- **현재 위험:** 낮음. 기본 로그 OFF(warn)라 PTY 출력이 로그로 흐르지 않음.
- **재도입 시점:** `set_log_level("debug")`로 PTY 내용이 로그에 찍힐 수 있는 때 — headless 테스트(모듈 6b)의 debug 로깅 또는 디버깅 단계.
- **요구사항:** 로그 출력에서 API 키/토큰 패턴(sk-…, Bearer …, Anthropic 키 등) 마스킹. 조직 보안룰(자격증명 산출물 포함 금지)과 직결.
- **담당 결정:** ed12 — 모듈 6b 착수 전 재검토.

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

## LLD 문서 갱신 — ✅ 완료 (2026-06-11, 코드 반영)

§9(R-1 알림분담+Exiting), §10(D-3 poison fail-fast 규칙4), §13(D-2 JobObject API), §14(D-1 로깅+mask_api_keys 보류), §6(D-4 drain 4인자) 모두 backend-lld-stage1.md에 반영 완료.

### D-5. frontend-integration-lld 동기화 (Phase 3 중 발견)
- **상태:** 미반영. Phase 3 마무리 시 frontend LLD 갱신.
- 경로: LLD가 src/types/pty.ts·src/lib/ptyApi.ts·src/lib/eventBus.ts 표기 → 실제 src/api/types.ts·src/api/ptyApi.ts·src/store/eventBus.ts. LLD 경로 표기 갱신.
- getSnapshot: LLD §2가 {seq,data_b64}[] 오기 → 실제 PtyChunk는 data:number[]. 코드는 unknown[]+Phase3c 보류로 처리(T-7). LLD §2 수정.

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
