# 핸드오프 — Engram Dashboard S12 phase4-2 (DaemonClient 연결 완료)

작성 2026-06-15. 이전 핸드오프(`2026-06-15-S12-phase4-handoff.md`)의 후속. 본문(`docs/process/step-log.md`·`docs/decisions/`·`CLAUDE.md`)이 항상 우선.

## 0. 한 줄 요약
**phase4-2(프론트 DaemonClient WS 연결) 완료 — 데몬 모드 GUI E2E 실측 PASS.** spawn→출력→입력 echo→kill→**소켓 드롭→재연결 resume(무손실·무중복)** 까지 실제 앱(cdp 9223)에서 확인. git `114f9ac`까지 커밋 완료, 작업트리 깨끗(`.ccb/` 제외).

## 1. git 상태
- 브랜치 `master`. 이번 세션 커밋 4개:
  - `7ed756b` phase4-2 #6 반환 event(Created/Spawned)
  - `45af572` 프론트 DaemonClient(WS) + clientFactory 두-모드 토글
  - `6998404` DaemonClient 재연결 resume 출력 중복/유실 fix(high-water dedup)
  - `114f9ac` step-log 기록
- **push 미실행**(이전 핸드오프대로 master 직접 push 차단 — 사용자가 `! git push origin master` 직접 또는 명시 승인 시).
- 커밋 author: 이 repo local `user.email=kimsunzun@naver.com`(개인). 건드리지 말 것.
- 커밋 트레일러: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

## 2. 구현 실행 규약(불변, 이번에도 준수함)
메인=오케스트레이터. 코더(opus 서브에이전트) → **reviewer-deep**(무겁게, mutation 검증) → QA(build/test + **GUI 실측 cdp**) → 게이트 통과 후 메인이 커밋. 에이전트 프롬프트에 "포그라운드 동기·폴링/백그라운드 금지·끝나면 즉시 반환" 명시(이번엔 폴링버그 사고 없음). 사이클 후 mutation 잔여 점검.
- **이번 교훈: 정적 리뷰(reviewer-deep)가 재연결 resume Major 버그를 두 번 GO 했으나 GUI 실측이 적출.** dedup/resume 같은 와이어 의미 의존 로직은 **반드시 실제 앱으로 드롭→재연결 실측**할 것(정적 검수만으로 불충분).

## 3. 이번에 완료한 것
| 영역 | 내용 |
|---|---|
| 데몬 #6 | `AgentEvent::Created{request_id,profile}`/`Spawned{request_id,agent}` append. dispatch: CreateProfile→Created, SpawnByCwd/SpawnProfile→Spawned/Error. bare Spawn 은 프론트 미사용이라 Ack 유지. |
| 프론트 DaemonClient | `src/api/daemonClient.ts` — AgentClient WS 구현. discover_daemon→ws→Auth→Hello. JSON text=AgentEvent / ArrayBuffer=binary frame(`decodeOutputFrame` 순수함수, codec.rs 바이트 단위 일치, UUID network order). request_id pending Map. 지수 백오프 재연결. high-water(`lastDeliveredSeq`) dedup. |
| 토글 | `src/api/clientFactory.ts` — `window.__ENGRAM_MODE__`/localStorage `engram_client_mode`('embedded'|'daemon'), 부팅 1회 결정. 기본 embedded(회귀0), daemon opt-in. |
| Origin 실측 | WebView2 dev origin=`http://localhost:1420` → 데몬 `ALLOWED_ORIGINS` 에 이미 포함(설계 적중). prod `tauri.localhost`/`https://tauri.localhost` 도 포함. WS 핸드셰이크 수용 확인. **allowlist 변경 불필요.** |
| GUI E2E | daemon 모드 connected→spawn(Spawned)→subscribe(replay+live)→writeStdin echo(정확 디코드)→kill(0)→**드롭→재연결 resume [3] dupes=0/inOrder/무손실** PASS. |

테스트: protocol 32 / daemon lib 25 + e2e 44(+ignored 3) / core 55 / tsc 0 / clippy 0. workspace 전체 빌드 OK.

## 4. 다음 작업 (phase4-2 잔여 → 마무리)
1. **★프론트 event 배선(핵심 갭)★** — DaemonClient 의 `handleEvent` 가 현재 `StatusChanged`/`AgentListUpdated`/`RestoreResult`/구조화 `Output` 를 **무시**한다. EmbeddedClient 는 이것들을 Tauri event(`eventBus`)로 받지만 DaemonClient 는 WS event 라 경로가 다름. store/eventBus 가 두 모드 공통으로 상태 갱신(트리/상태바)을 받도록 배선 필요 — 안 하면 daemon 모드에서 spawn/kill 시 **트리/상태가 실시간 갱신 안 됨**(출력은 됨, 상태 브로드캐스트만 누락). EmbeddedClient 의 eventBus 소비 구조 참고해 DaemonClient 에 동일 의미 콜백 노출.
2. **자동 부팅 daemon 모드 배선** — 현재 daemon 모드는 localStorage 수동 설정 후에만. 정식 설정 UI/부팅 자동 discover_daemon 배선.
3. **nit 후속(저위험·장기, §0 기준 깔 만함)**:
   - getAgents/listProfiles/getSnapshot 응답에 `request_id` 동봉 응답 variant 추가(현재 broadcast 편승 매칭 — 동시호출/타클라 트리거 오매칭 가능, 값은 정확하나 깨지기 쉬움). #6 과 동형 작업.
   - `SubscribeAck.truncated` 를 UI 로 전파(현재 console.warn 만 — 사용자가 출력 손실 인지 못 함). `OutputSubscription` 인터페이스 확장 필요.
   - core `pid_alive`(process.rs:66-68): OpenProcess 실패를 무조건 live 취급 → ERROR_INVALID_PARAMETER(부재=dead) vs ERROR_ACCESS_DENIED(권한=live) 구분. start_time=0 fallback 한정 저영향(정상 daemon.json 무관).

## 5. 그 뒤 (모바일/별건, 미결)
- **모바일**: VT-framebuffer 화면상태 동기(Mosh 갭2), roaming TCP→UDP(갭3, transport 교체급), predictive echo(갭4). **Mosh 는 턴키 라이브러리 없음 → 아이디어만 차용해 직접 구현**(브라우저 UDP 불가·libmosh 없음). 로컬/근거리에선 현 WS+replay 로 충분, 실제 모바일 답답할 때 갭별 보강.
- **별건**: `pty/`→`agent/` 폴더 rename, core `AgentCommand`→`SpawnSpec` 개명, ts-rs `bindings/*.ts` 프론트 실소비(현재 손-미러), Interrupt-lease 정책(긴급중단 누구나 허용 여부, phase4 멀티클라 실사용 때).

## 6. 빌드·검증 (workspace 루트)
```bash
cargo test -p engram-dashboard-protocol            # 32
cargo test -p engram-dashboard-core --lib          # 55
cargo test -p engram-dashboard-daemon              # lib 25 + e2e 44 (+ignored 3)
cargo clippy --workspace --all-targets -- -D warnings
npx tsc --noEmit                                   # 프론트 타입 게이트
cargo build --workspace
```
### GUI 실측(데몬 모드) — 이번에 쓴 절차
```bash
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev   # bash, 백그라운드
# 포트 대기 후:
node scripts/cdp.mjs eval "localStorage.setItem('engram_client_mode','daemon'); location.reload()"   # daemon 모드 전환
# 리로드 후 window.__ENGRAM_AGENT__ 가 DaemonClient, connectionState 'connected' 확인
node scripts/cdp.mjs eval "(async()=>{const c=window.__ENGRAM_AGENT__; const i=await c.spawnAgent('<cwd>'); ...})()"
# 재연결 테스트: c.ws.close() 로 소켓만 드롭(데몬 생존) → 재연결 → cmd 보내 dupes/순서/유실 확인
```
- **stale daemon.json 주의**: 기능 이전(start_time 없는) 옛 daemon.json + 죽은 PID 면 discover 가 죽은 포트 반환(§3 부수 발견). 증상=daemon 모드 'reconnecting' 고착. 해소: `rm "$APPDATA/com.engram.dashboard/daemon.json"` 후 재시도(정상 daemon.json 은 start_time 있어 무관).
- discover_daemon 이 WMI 로 데몬 .exe 자동 spawn(`target/debug/engram-dashboard-daemon.exe`). daemon.json 에 pid/port/token/start_time 발행.

## 7. 핵심 불변식 (변경 금지)
이전 핸드오프 §7 동일 + 추가:
- **DaemonClient dedup**: 클라 실배달 high-water(`lastDeliveredSeq`) 기준. **`replay_from` 은 dedup 기준 아님**(=데몬이 보내는 첫 seq, output_core.rs:322 / ws_e2e:758). resubscribe 는 알려진 epoch 전송(데몬 Resume tail-only). epoch 변경 시에만 lastDeliveredSeq 리셋.
- 데몬 SubscribeAck FIFO: `on_ready`(Ack)가 replay binary 보다 먼저 같은 conn_tx 에 큐잉(C4) → 클라가 Ack 로 epoch/dedup 재동기 후 frame 처리 보장.

## 8. 시작 첫 행동 제안
1. `docs/process/step-log.md` 맨 끝 phase4-2 항목 + 이 핸드오프 대조.
2. 다음 = **#1 프론트 event 배선**(daemon 모드 트리/상태 실시간 갱신). EmbeddedClient↔eventBus 구조 먼저 읽고 DaemonClient 에 동일 의미 경로 추가. coder(opus)→reviewer-deep→**GUI 실측(daemon 모드 spawn 시 트리 갱신 확인)**.
3. 환경: 이번 세션이 띄운 tauri dev(포트 9223) + 데몬이 아직 살아있을 수 있음 — 새 세션이면 상태 확인 후 재사용/재기동.
