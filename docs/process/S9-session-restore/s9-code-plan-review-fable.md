# S9 코드 기획 검토 — fable

**검토자:** fable (pane 8), 2026-06-12
**대상:** `session-restore-code-plan.md` (코어 GO 범위)
**확신 수준:** [확실] / [가능성 높음] / [불확실]

---

## 종합 판정: 골격 GO — 단 구조 결손 1건(ProfileRegistry 부재)과 순서 결함 1건(E가 D보다 뒤)은 착수 전 수정

A~G 분해와 시그니처 수준 명세는 컴플라이언스 리뷰 가능한 품질이다. 아래 (3)의 ProfileRegistry가 가장 큰 누락 — 프로필의 인메모리 소유자가 기획에 없어서 fallback·CRUD·watcher 콜백이 갱신할 대상이 정의되지 않는다.

---

## (1) LLD 개정 7건(a~g) 커버리지 — 매핑은 완료, 구멍 3건

E절이 a~g를 전부 명시 ✓. 단:

1. **epoch++ 지점 누락 [확실]:** 기획상 epoch++는 `restart_agent`(게이트)에만 있다. 그러나 **restore_all의 fresh fallback도 "같은 AgentId로 새 PtySession 맵 교체"**다 — resume 실패한 세션에 프론트가 이미 구독했을 수 있으므로 fallback respawn에도 epoch++가 필요하다. 규칙화하라: **"sessions 맵에 같은 AgentId로 교체 삽입이 일어나는 모든 지점 = epoch++"** (restart_agent·fallback 공통).
2. **A의 필드 누락 [확실]:** LLD §1 대비 `restart_policy`와 `last_restore`가 빠졌다. restart_policy는 자동재시작 게이트 의도라면 의도임을 명시하되 — **필드는 지금 넣고 기본 Never를 권장**한다(나중에 넣으면 schema 마이그레이션 1회 추가). `last_restore`는 fallback UX("언제부터 새 대화였나")의 persist 근거라 **코어 GO 범위다** — 복원하라.
3. **순서 역전 [확실]:** 구현 순서에서 E(LLD 개정)가 G.6, manager 구현이 G.5다. G.5의 restore_all/fallback이 구현하는 의미론(맵 교체+epoch, stale unsubscribe 무해)이 바로 E(a)(c)다 — **기준 문서를 코드보다 먼저 고쳐야 컴플라이언스 리뷰가 성립**한다. E를 G.4와 G.5 사이로 당겨라.

---

## (2) session_tracker — 방향 타당 [확실], 구현 허점 3건

pid 우선 → sid 스캔 fallback, 디렉토리 watch, 관측 즉시 persist — 최종 검토 권고 그대로다. 허점:

1. **spawn 직후 타이밍 [확실]:** claude 기동에 수 초가 걸리므로 `<pid>.json`은 spawn 직후 존재하지 않을 수 있다. `self_test`를 spawn 직후 1회 동기 호출하면 false negative(가정붕괴 오경보)가 기본 동작이 된다. **재시도 루프(수 초 간격, 상한 ~30s) 또는 watcher의 첫 create 이벤트 시점에 수행**으로 변경.
2. **watcher 정지 핸들 부재 [확실]:** `spawn_session_watcher`가 핸들을 반환하지 않는다 — kill/respawn 시 옛 watcher를 멈출 방법이 없고, respawn 후엔 옛 PID 기준 watcher가 좀비로 남는다. 핸들(stop 가능)을 반환해 PtySession 또는 manager가 보관·정리.
3. **에이전트당 watcher 1개 = 같은 디렉토리에 N개 watcher [확실]:** 전부 같은 `sessions/`를 본다. **manager 소유 단일 watcher + 파일명→AgentId 디스패치**로 통합 권장 — 정지 핸들 문제(2)도 단순해진다.

부수: self_test/watcher/sid 로직은 **Claude variant 한정**임을 시그니처나 호출부에 명시(Shell엔 불요 — 현 D 의사코드는 Shell에도 sid를 생성한다, 무해하나 오염).

---

## (3) spawn/restore/fallback 흐름 — 허점 4건

1. **★ProfileRegistry 부재 [확실 — 최대 누락]:** B는 save/load 함수, D는 `&AgentProfile` 인자, F는 CRUD command — 그런데 **프로필 목록을 메모리에서 소유·갱신하는 컴포넌트가 없다.** fallback의 "새 uuid로 claude_session_id 갱신 + old_session_ids push + persist", watcher 콜백의 sid 갱신, F의 update_profile이 전부 갱신할 *대상*이 미정의. **신설:** `ProfileRegistry { profiles: Mutex<HashMap<AgentId, AgentProfile>>, 변경 시 save_profiles 호출 }` — B에 합치거나 별도 모듈. manager가 Arc로 보유. 이게 없으면 G.5에서 즉석 설계가 일어난다.
2. **resume 실패 감지 메커니즘 미정의 [확실]:** "exit 1 → fallback"이라 쓰여 있지만 exit 1은 **drain thread의 EOF→transition으로 비동기 도착**한다. restore가 이를 어떻게 아는가? 정의 필요: spawn 후 **조기 종료 윈도**(예: T=10s 내 `Exited{code:1}` 전이 → resume 실패로 판정 → fallback; T 경과 시 Resumed 확정). 판정 주체는 restore 태스크(상태 구독 또는 폴링).
3. **restore_all 호출 위치 [확실]:** F의 "setup에서 restore_all 트리거"를 동기 호출하면 stagger × N + 조기종료 윈도만큼 **앱 창이 블로킹**된다. setup은 백그라운드 태스크 spawn만 하고 즉시 반환.
4. **fallback 사다리 종점 명시:** fresh fallback마저 실패(claude 미설치 등)하면 재귀 없이 `Failed` + restore-result로 종료 — 한 줄 명시(현 문구 "사다리"는 재시작 절에만 있다).

---

## (4) ★미래 확장성 — WS ◎ / codex ○ / 종량제 API △. 지금 할 일은 2건, 나머지 YAGNI

| 확장 | 흡수 가능? | 근거 |
|---|---|---|
| **모바일 원격 (WebSocket)** | **◎ 큰 변경 없이** | `OutputSink`/`StatusSink`가 정확히 그 seam이다 — WebSocketSink impl 추가면 코어 무변경. pty/가 Tauri-free인 것(1단계 C1 격리)의 배당금. 그때 가서 추가할 것: 인증, **느린 원격 클라이언트 backpressure**(로컬 Channel엔 없던 문제 — sink별 큐 상한), 원격 resize 정책(프론트 리뷰 M1의 재등장). 지금 준비: **없음** — pty/ Tauri-free 원칙 유지가 곧 준비다. |
| **codex CLI** | **○ enum variant로 흡수** | 프로필/persist/spawn 골격은 이미 중립. 비용은 variant + per-CLI 세션 의미론(인자, resume, 세션 추적). 단 현 기획은 **claude 전용 지식이 D(인자 빌드)와 C(sessions 추적)에 분산** — codex 추가 시 두 모듈을 또 건드린다. |
| **종량제 API (비-PTY)** | **△ — 정직하게, 큰 변경 없인 안 된다** | PtyManager가 PTY 1:1 전제(drain/master/resize)다. 비-PTY 세션은 manager 일반화(AgentSession 추상) 리팩토링 필요. 다만 **프로필·persist·epoch·이벤트 층은 이미 전송 중립적**이라 리팩토링 반경이 manager에 국한된다. 지금 trait 일반화 도입은 YAGNI — 1단계에서 공들인 동시성 설계만 흐려진다. |

**지금 미리 할 것 (둘 다 현재 가독성에도 이득 — 투기 아님):**
1. **`pty/claude.rs` 신설** — claude 전용 지식(인자 빌드, sessions/<pid>.json 추적, resume 의미론, CLAUDE_CONFIG_DIR)을 단일 모듈로 격리. C(tracker)는 그 안으로, D의 args match는 그 모듈 호출로. codex는 후일 `codex.rs` 병렬 추가.
2. **AgentProfile 등 프로필 타입을 `pty/types.rs`가 아닌 중립 모듈(`profile.rs`)로** — 프로필은 PTY 개념이 아니다. 종량제 API 시나리오에서 그대로 살아남는 층을 물리적으로도 분리해 두면 경계가 자란다. (+ AgentProfile에 PTY 전용 필드를 넣지 않는 현 상태 유지)

---

## (5) 분배 단위 — 적정 [확실], 조정 3건

시그니처 수준 명세 + 모듈 단위 분배는 컴플라이언스 리뷰(LLD 일탈 검사)와 정확히 맞물린다. 조정:

1. **ProfileRegistry를 분배 단위에 추가** (B 확장 또는 독립 단위).
2. **E(LLD 개정)를 G.5 앞으로** — (1)-3.
3. **의존/병렬:** A 완료 후 B·C는 병렬 가능, D는 B·C·E 대기, F는 D 대기. 각 모듈 산출물에 **"구현한 LLD 절 번호" 명기를 요구**하라 — 리뷰어(나)가 일탈을 절 단위로 대조할 수 있어 리뷰 속도가 오른다.

사소: F의 CRUD 명명 불일치(`create_agent` vs `list_profiles`) — profile로 통일 권장. `delete_agent`는 실행 중 에이전트 처리(거부 vs kill 후 삭제) 의미론 한 줄 필요.
