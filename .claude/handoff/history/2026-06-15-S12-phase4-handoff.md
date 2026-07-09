# 핸드오프 — Engram Dashboard S12 데몬화 / phase4 진입

작성 2026-06-15. 이 문서 하나로 다음 세션이 이어가도록 정리. 상세는 항상 `docs/process/step-log.md`(흐름)·`docs/decisions/`(ADR)·`CLAUDE.md`(불변 규약) 본문을 신뢰. 이 문서와 본문이 어긋나면 본문이 우선.

---

## 0. 한 줄 요약
**데몬화(S12) backend는 사실상 완성**(PTY/WS/discovery/keepalive/panic/lease/resize/프로필 wire). 다음은 **phase4-2 = 프론트 DaemonClient(WS 클라) + 두-모드 토글**이며, 이건 **GUI 실측(cdp)이 필수**라 자율로 못 끝내 여기서 멈춤. git `98c05e1`까지 push 완료, 작업트리 깨끗.

---

## 1. 지금 git 상태
- 브랜치 `master`, 최신 커밋 `98c05e1` (origin 동기 완료, 미커밋 0).
- repo: `github.com/kimsunzun/engram-dashboard` (private, 모노레포 `I:\Engram`에서 분리됨 — dashboard는 자체 .git).
- **커밋 author 주의**: 이 repo는 local `user.email = kimsunzun@naver.com`(개인) 설정됨. global은 `nm-fc.com`(회사) 유지 — 회사 프로젝트용. 개인 repo 커밋이 회사 이메일로 안 나가게 이 설정 건드리지 말 것.
- **push 차단**: auto 모드 분류기가 "에이전트의 master 직접 push"를 막을 수 있음. 사용자가 `! git push origin master`로 직접 하거나 명시 승인 시 진행.
- 커밋 메시지 끝 트레일러: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- LF→CRLF warning은 Windows 정상, 무해(무시).

---

## 2. ★ 구현 실행 규약 (강제 — 절대 생략 금지) ★
`CLAUDE.md`의 "구현 실행 규약" 그대로. 비자명 코드 변경마다:
- **메인 세션 = 오케스트레이터.** 직접 구현 편집 금지(1~2줄 사소·문서·주석·탐색만 인라인 예외).
- **코더 = 서브에이전트(opus) 스폰.**
- **리뷰어 = 스폰.** 이제 전용 에이전트 **`reviewer-deep`**(Project custom agent, 고노력 적대 리뷰)가 생겼으니 이걸 1순위로. 없으면 opus 고노력. **리뷰 스킵 절대 금지. 사용자 명시: "리뷰는 무겁게"** (mutation 검증 포함).
- **QA = build/test + GUI 실측(`scripts/cdp.mjs`) 항상.** 코드 통과해도 실제 화면 확인 전엔 미완.
- 커밋은 게이트 통과 후 메인이.

### ★ 에이전트 운용 함정 (이번에 실제로 터진 것) ★
1. **리뷰어/코더가 백그라운드 폴링 루프에 빠져 19분 헛돌고 죽는 사고 발생.** → 에이전트 프롬프트에 "**모든 셸 명령 포그라운드 동기 실행, run_in_background·폴링·대기 루프 금지, 끝나면 즉시 보고 반환**"을 반드시 명시. (reviewer-deep 스폰 시에도.)
2. **비정상 종료한 리뷰어가 mutation(검증용 코드 변조)을 원복 못 한 채 작업트리를 오염시킴.** ws.rs에 `LeasePass::Allow // MUTATION` 잔여 → build warning(`never constructed`)으로 적발·원복. → **에이전트 사이클 후 반드시 `git diff`/`rg "MUTATION"`/`cargo build` warning 0 확인**으로 mutation 잔여 점검. "보수적으로 가자"는 이걸 잡으라는 뜻이었음.
3. SendMessage 도구는 이 환경에서 불가 — 죽은 에이전트 이어가기 안 됨, 새로 스폰.

---

## 3. 완료된 것 (이번 세션, 전부 coder→reviewer→QA→커밋→push)
| 커밋 | 내용 |
|---|---|
| `b868458` | Step4c afterSeq resume (`subscribe_from` tail-only, on_ready 콜백으로 TOCTOU 제거) |
| `912e095` | Step5 WMI discovery (Tauri→데몬 발견·spawn, ComGuard, DaemonInfo.start_time로 PID 재사용 구분) |
| `5caa7dd` | Step6 격리 하네스 WS E2E (in-process 26 + dispatch 제어평면 11종) |
| `bd7e5d0` | phase2 전체검사 정리 (reflection→명시 From, dirs 통일, 테스트 갭) |
| `079d3a6` | step7 실프로세스 하네스 (실 .exe 3종, flaky=전역 mutex 경합 → ENGRAM_INSTANCE_KEY 격리) |
| `a451037` | 3대장 갭 v1 2건 — WS keepalive + 데몬 panic 격리 |
| `edfe582` | 데몬측 완결 갭 — 멀티 resize 협상(tmux smallest) + 다중입력 lease |
| `98c05e1` | phase4-1 프로필 CRUD + spawn WS wire (DaemonClient 전제) |

**데몬 backend 완성 범위**: PTY spawn/kill/Job(KILL_ON_JOB_CLOSE)·OutputCore replay/resume(Reset/Truncated/Resume)·WS 서버(auth/Origin/단일writer FIFO/backpressure close_signal)·WMI discovery·keepalive·panic 격리·입력 lease·resize 협상·프로필 CRUD wire.

**최종 테스트(녹색)**: protocol 26 / daemon lib 25 + ws_e2e 44 (+실프로세스 ignored 3) / core 55 / clippy 0 / 코어 격리(`use tauri`·`engram_dashboard_protocol` import 0).

---

## 4. ★ 다음 작업 = phase4-2 (프론트 연결) ★
DaemonClient의 전제(프로필 wire)는 phase4-1로 깔림. 이제 프론트.

### 순서 (권장)
1. **#6 반환 event 추가 (데몬 backend, GUI 무관, 자율 가능)**
   - 문제: 현재 CreateProfile/SpawnByCwd/SpawnProfile은 `Ack{request_id}` + 후속 broadcast(ProfileListUpdated/AgentListUpdated)만 보냄. 그런데 broadcast event엔 request_id가 없어, DaemonClient가 인터페이스(`Promise<AgentProfile>`/`Promise<AgentInfo>`)를 채우려 할 때 "내가 만든 항목"을 동시생성 race 없이 식별 불가.
   - **권장 해법(리뷰어도 동의)**: protocol `AgentEvent`에 `Created{request_id, profile}` / `Spawned{request_id, agent}`를 **append**(하위호환). 데몬 dispatch에서 CreateProfile→Created, SpawnByCwd/SpawnProfile→Spawned를 **요청 연결에** 보냄(기존 Ack·broadcast는 유지해도 됨, 회귀 0). DaemonClient가 request_id로 매칭해 resolve.
   - ws_e2e로 검증 가능(GUI 불필요).
2. **DaemonClient 구현** (`src/api/daemonClient.ts`) — `AgentClient` 인터페이스(`src/api/agentClient.ts`)를 WS로 구현.
   - 연결: discovery로 port/token 회수(Tauri `invoke('discover_daemon')` — phase4-1에서 command 노출됨) → `ws://127.0.0.1:<port>` → 첫 frame `Auth{token, protocol_version}`.
   - 송신: AgentCommand JSON text. 수신: AgentEvent JSON text + binary frame(codec `decode_frame`: `[tag:1][agentId:16][epoch:4 BE][seq:8 BE][raw]`, FRAME_HEADER_LEN=29) 디코드 → OutputChunk{seq, bytes}.
   - subscribeOutput: Subscribe{agent_id, epoch, after_seq} → SubscribeAck → replay binary → ReplayComplete → live. **재연결**: 끊김 감지 → 지수 백오프 재연결 → 재auth → Subscribe{after_seq=마지막seq} resume → seq dedup. connectionState(connected/reconnecting/down) + onConnectionStateChange.
   - request_id로 Ack/Error/Created/Spawned 매칭(Promise resolve/reject).
   - **#13133 함정**: Channel/WS onmessage 정리 시 `delete obj.onmessage`(null 아님) — EmbeddedClient 패턴 참고.
3. **clientFactory 두-모드 토글** (`src/api/clientFactory.ts`) — Embedded/Daemon 선택(startup, 핫스왑 아님). `window.__ENGRAM_AGENT__` 노출 유지.
4. **Origin 실측** — WebView2 실제 Origin 문자열 측정: 앱 띄워 cdp `eval`로 `location.origin` 확인 + 데몬 WS 핸드셰이크의 실제 Origin 헤더. 데몬 `ws.rs`의 `OriginCheck` allowlist에 반영(현재 "Origin 없음 허용 + 토큰 주방어").
5. **데몬모드 GUI E2E** (cdp) — 데몬 띄우고 앱을 Daemon 모드로 → spawn→output 왕복→kill→재연결 resume 실측.

### GUI 실측 방법 (CLAUDE.md 그대로)
```bash
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev   # bash, 백그라운드
# 포트 대기: curl http://127.0.0.1:9223/json/version
node scripts/cdp.mjs eval "<js>"     # 앱 안 JS/invoke 실행 (검증은 shot보다 eval 텍스트가 토큰·정확도 유리)
node scripts/cdp.mjs shot out.png    # 스크린샷
```
포트 9223 고정(9222는 Gemini Chrome 충돌 회피).

---

## 5. 더 뒤 (phase4-2 이후, 미결)
- **Interrupt-lease 정책**: 현재 Interrupt(Ctrl+C)도 입력 lease 게이트 대상 → 다른 뷰어가 lease 보유 중 긴급 중단이 막힐 수 있음. lease가 v1엔 멀티클라(phase4) 전 미사용이라 실영향 0. **phase4 멀티클라 실사용 때 "긴급 중단은 누구나 허용"으로 풀지 결정** (ws.rs Interrupt dispatch arm에서 check_input 제거 여부).
- **모바일(phase4 후반)**: VT-framebuffer 화면상태 동기(Mosh 갭2, 대역폭+alt-screen 정확성), roaming TCP→UDP(Mosh 갭3, transport 교체급·실측 필요), predictive echo(Mosh 갭4, xterm 프론트).
- **별건**: `pty/`→`agent/` 폴더 rename(대량 경로, 내용과 불일치), core `AgentCommand`→`SpawnSpec` 개명(protocol 동명 충돌, serde 무영향=wire 안전), ts-rs `bindings/*.ts`를 프론트 `src/api/`에서 실제 소비(현재 손-작성 미러).
- **alt-screen evict 케이스**(tmux 지적): truncated=false여도 alt-screen 진입 시퀀스가 ring 밖으로 밀리면 화면 복원 깨질 수 있음 — daemon-design.md에 문서화됨, VT-framebuffer로 근본 해소.

---

## 6. 빌드·검증 명령 (workspace 루트 `I:\Engram\apps\engram-dashboard`)
```bash
cargo test -p engram-dashboard-protocol          # 26 (codec golden + ts_export)
cargo test -p engram-dashboard-core --lib        # 55
cargo test -p engram-dashboard-daemon            # lib 25 + ws_e2e 44 (+ignored 3)
cargo test -p engram-dashboard-daemon -- --ignored   # 실프로세스 3 (실제 데몬 .exe)
cargo clippy --workspace --all-targets -- -D warnings  # 0
cargo fmt
rg "use tauri" crates/engram-dashboard-core/src/        # 0 (주석 1줄 제외)
rg "engram_dashboard_protocol" crates/engram-dashboard-core/src/   # 0 (코어 격리)
npm run tauri dev                                 # 전체 E2E (프론트)
```
데몬 단독 실행 검증(격리): `ENGRAM_DATA_DIR=<temp> ENGRAM_INSTANCE_KEY=<uniq>` 환경변수로 사용자 실환경/실데몬과 충돌 없이 띄움. daemon.json에 pid/host/port/token/protocol_version/start_time 발행.

---

## 7. 핵심 불변식 (변경 금지 — 깨면 회귀)
- **kill 인과 2동사**: `transport.shutdown()` → `core.join_pump(5s)`. master drop→reader EOF→pump break→`core.finish`.
- **finalize 1회**: `OutputCore.finalized.swap(AcqRel)`. pump panic도 catch_unwind→finish(Error)로 1회 보존(done_tx는 catch_unwind 밖이라 join hang 0).
- **락 순서**: sessions Arc clone 후 즉시 해제. status lock 보유 중 외부호출 금지. emit은 subscribers clone 후 lock 미보유 send. MultiViewState(viewport/lease) lock도 짧게 잡고 해제 후 manager 호출.
- **C4**: subscribers lock 보유 중 replay 전송(subscribe_from은 on_ready 콜백으로 Ack를 replay binary보다 먼저 큐잉).
- **backpressure**: OutputSink.send는 try_send만(non-blocking). full→close_signal(out-of-band Notify). emit 절대 블록 금지.
- **코어 격리**: core는 tauri·protocol import 0. core↔wire 변환은 데몬에 **명시 함수**(serde reflection 왕복 금지 — 한 번 제거한 안티패턴).
- **epoch**: 맵 교체마다 +1. 재구독 `[agentId, epoch]`.
- 상세·근거: `docs/decisions/` ADR.

---

## 8. 시작 첫 행동 제안
1. `docs/process/step-log.md` 맨 끝부터 거꾸로 읽어 최신 흐름 확인(이 핸드오프와 대조).
2. phase4-2 **#6 반환 event**(데몬 backend)부터 coder(opus) 스폰 → `reviewer-deep` 무겁게 → QA(ws_e2e) → 커밋.
3. 그 뒤 DaemonClient(프론트) — 이건 GUI 실측 필요하니 cdp로 앱 띄워 검증.
4. 매 에이전트 프롬프트에 "포그라운드 동기·폴링 금지" 명시. 사이클 후 mutation 잔여 점검.
