# 핸드오프 — embedded 제거→daemon-only 통일 + 트레이 마무리 + 후속수정 완료, 다음=프론트(백엔드 ①②먼저 권장)

작성/갱신 2026-06-21 (dashboard10 세션). 본문(`docs/decisions/`·`docs/process/step-log.md`·`CLAUDE.md`)이 항상 우선. **master HEAD=`aacb0f2`, push 안 함(이번 세션 14커밋 로컬).** working tree 잔여 = dashboard-sub(병렬세션) 소유 docs 3건 + `.ccb`(로컬)뿐.

## ★★ 다음 세션 첫 행동 (필독) ★★
1. **읽기:** 이 핸드오프 → **ADR-0029**(embedded 제거, daemon-only) → **ADR-0028**(백엔드 이벤트버스) → ADR-0021(데몬 수명, 재연결 보강) → step-log dashboard10(2 엔트리).
2. **규약:** 비자명 변경 = 코더(opus)→reviewer-deep(**fable 1순위, 현재 접근 불가 → opus 고노력 대체. 스킵 금지**)→QA(cdp). 조사/대량읽기 서브에이전트 일임. 굵은 설계 = 사용자 결정·ADR.
3. **CLAUDE.md 갱신됨**(dashboard-sub): "새 문서=발견 체인 연결(고아 금지)" 줄 추가. 모듈 맵은 아직 stale(아래 백엔드 ② 참조).

## 0. 한 줄 요약
스텝3 모드 시스템(6커밋) → **사용자 결정으로 폐기하고 embedded 제거·daemon-only 통일**(분기 번식=1조 위반). 안전 게이트(데몬 호스팅 입증) 후 제거(A/B/C). 트레이 마무리(이벤트버스 생사 push·클릭=UI·메뉴 정리). 사용자 테스트에서 나온 후속 5건까지 수정. cargo/tsc/npm 전부 green.

## 1. 구조 변화 (daemon-only)
- **에이전트 호스트 = 항상 데몬**(별도 프로세스). 앱(Tauri)=데몬 상주 클라이언트(창·트레이·로컬제어 command + 데몬 discovery). in-process 호스팅 제거.
- **모드 개념 소멸**(`--mode`/`ENGRAM_MODE`/`__ENGRAM_MODE__`/AppMode/set_mode 전부 제거). "로컬 vs 원격"은 데몬 위치(WS URL)=transport seam.
- **통신:** 의도/명령↑(invoke), 상태/이벤트↓(이벤트버스, ADR-0028). invoke="내 WebView↔내 Rust" 전용(데몬·원격=WS). 백엔드가 이벤트버스 소유, 프론트는 구독만.

## 2. 이번 세션 커밋 (master 로컬, push 안 함)
**스텝3(폐기된 모드시스템, 흔적):** `6798603`·`ae1a2a3`·`5972432`·`21ce28b`·`02de2cb`·`4184938`.
**embedded 제거(daemon-only):**
| 커밋 | 내용 |
|---|---|
| `a9807c2` | **A 프론트** — InProcTransport·EmbeddedDaemonControl·모드결정 제거, WsTransport 고정. |
| `ef4b53a` | **B src-tauri/discovery/daemon 대수술(1676줄 삭제)** — AppState·in-proc AgentManager·embedded_carrier·instance.rs·모드시스템·shutdown_all 제거. single-instance/build_tray/X=hide/--hidden 무조건. default_data_dir 무인자(release=appdata/debug=walk-up, **ENGRAM_DATA_DIR override 유지**). conf 첫창 label="main". |
| `80a5d04` | **C 트레이** — ADR-0028 데몬생사 옵저버(3s)→단일 publish(LivenessState)→트레이 set_icon+emit. 더블클릭=UI, Show/Hide 메뉴 제거(command 유지). 끄기↔옵저버 race=grace 억제창(5s). |
| `ccb7e41` | step-log(embedded 제거). |
**후속 수정(사용자 테스트):**
| 커밋 | 내용 |
|---|---|
| `29d1d5d` | 트레이 좌클릭에 메뉴 안 뜨게(`show_menu_on_left_click=false`) — 메뉴=우클릭. |
| `ac25754` | **데몬 hot-swap 재연결** — `read_daemon_info`(no-spawn, daemon.json 재조회)로 옮겨간 데몬 추적. ADR-0021 "재연결=spawn 금지" 유지(재조회≠spawn). **+reviewer Blocker: 새 await 가 뚫은 좀비소켓 race → openGen 세대토큰 가드로 차단**(회귀테스트 2, mutation 검증). |
| `d6df88d` | UI 열기 더블클릭→**단일 좌클릭**(Left+Up). |
| `aacb0f2` | **부팅 다중 spawn 직렬화**(commands/discovery.rs ensure_lock=프로세스전역 async Mutex — 콘솔창 깜빡임 제거) + **재연결 시 트리 재동기화**(eventBus onConnectionStateChange→getAgents+refreshProfiles, 첫연결 스킵). |

## 3. 현재 동작 상태 (QA 실측 — 전부 PASS)
- 앱 기동(`npm run tauri dev`, env 불필요) → 데몬 자동 spawn(1개 수렴)+WS 연결 → 셸 에이전트 spawn→양방향 출력→kill→UI 갱신.
- daemon-status-changed emit 정확(끄기 false/켜기 true, grace 동작). hot-swap(port 교체) 리로드 없이 ~2s 재연결+spawn, 좀비 부활 없음, 트리 재동기화.
- cargo workspace green(실패 0) + npm 74 + tsc 0. 게이트(코더→reviewer-deep→QA) 전부 통과.

## 4. ★다음 = 프론트엔드 (단 백엔드 ①② 먼저 권장)★
백엔드 코어(claude 호스팅·복원·graceful 종료·영속·재연결·인증)는 견고·검증됨. 프론트 전 백엔드 잔여(recon 조사):
- **① [권장·저비용] capability backend별 정확화** — 현 `PtyTransport.capabilities()`가 transport 단일 하드코딩이라 "shell인데 resume=true" 부정확. 프론트가 capability로 메뉴 회색처리 시작하기 **전에** claude/shell별 정확값 내게 고치는 게 쌈(§0 "지금 깐다").
- **② [권장·문서] rot 정리** — ADR-0024 idle self-shutdown 문구가 ADR-0021(무재시작)·코드(미구현)와 어긋남 → 폐기/후속 표기. CLAUDE.md 모듈맵 stale(daemon·discovery crate 누락, ADR-0029 미반영) 갱신.
- **③ [사용자 결정] 자동재시작 관측 seam** — 런타임 죽음 관측 훅 부재(AgentManager status 미구독). 동작은 ADR상 미래나 lifecycle 정책 PRD(restart 기본값·stable_secs B-1~B-5) 사용자 결정 대기. 프론트 "재시작" UI 전제.
- **④ [사용자 결정] idle self-shutdown** — 고아 데몬 방지. ADR-0021 "opt-in 후속". 트레이 상주 UX 직결.
- **⑤⑥⑦ [이연]** codex/gemini variant(stub·미연결, CLI spike 필요) · WSS/원격 TLS(현 ws://127.0.0.1만, auth는 됨) · ApiTransport 실체·메시지 시스템 — "원격/API 모델 때"(§0 고비용·불확실).

**프론트 본작업:** D-7(레이아웃/창 영속=프론트 localStorage, 미구현) · §5 LLM 제어표면(UI 액션을 command 버스로) · 멀티창·복원 배너 UX 등.

## 5. 핵심 불변식 (변경 금지)
- **데몬 직접 spawn 금지(ADR-0024 C1)** — discovery WMI 만. (set_mode self-relaunch 는 폐기됨 — 모드 없음.)
- **ENGRAM_DATA_DIR override = 테스트 격리**(default_data_dir 1순위). 제거 금지.
- **app·daemon default_data_dir() 동일 경로**(app이 daemon.json 찾고 daemon이 거기 씀): debug=walk-up, release=appdata.
- **재연결=attach-only, spawn 금지(ADR-0021)** — 단 `read_daemon_info`로 daemon.json **재조회(no-spawn)는 허용**(옮겨간 데몬 추적). discover_daemon(spawn)은 명시 start만.
- **재연결 소켓 openGen 세대토큰 가드** — await 재개 시 stale 시도 폐기(좀비소켓 차단). cleanupSocket이 openGen bump.
- **트레이 생사 = LivenessState 단일 publish 경유**(set_icon+emit 한 곳). 끄기 grace 억제창(death-window 재컬러 차단). StopOutcome 분류 유지.
- **이벤트버스 = 백엔드 소유, 아래로만 push(ADR-0028).** 프론트 상태 위로 push 금지(프론트→백엔드는 command/invoke=의도). 재연결 시 eventBus가 getAgents+refreshProfiles 재동기화.
- **ensure_internal 직렬화(ensure_lock)** — 부팅 다중 spawn 방지. 단 트레이 StartDaemon은 락 밖(named mutex가 최종 1개 보장).
- 코어(engram-dashboard-core) tauri import 0(데몬이 호스트 엔진으로 의존 — 제거 금지).

## 6. 환경/주의
- **빌드 잠금(os error 5):** 실행 중 dev/exe → `taskkill //F //IM engram-dashboard.exe //IM engram-dashboard-daemon.exe`(bash). check 는 락 무관(`cargo check`), full build/link 는 dev 닫고.
- **커밋 멀티라인 = PowerShell here-string(`@'...'@`)**(Bash 툴로 쓰면 리터럴 `@` 섞임 — 이번 세션 사례).
- **cdp shot 상대경로 금지**(mangled 파일명 생김). 절대경로.
- **dashboard-sub 병렬세션** 이 같은 디렉토리 작업 중 — working tree 의 `docs/README.md`·`docs/tracking.md`·`docs/research/control-surface-and-fleet.md` 는 그쪽 소유(내가 안 건드림). 커밋 시 파일 겹침 주의.
- 검증: `cargo test`(workspace) / `npm test`(74) / cdp 9223 / `cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored`(실프로세스 호스팅).

## 7. 핸드오프 종료 체크리스트
1. 새 결정 → ADR-0029·0028 ✅
2. 폐기 → ADR-0027 `폐기 (Superseded by ADR-0029)` ✅
3. README 인덱스 갱신(0027 폐기·0028·0029) ✅
4. step-log dashboard10(2 엔트리: 스텝3 + embedded제거) ✅ — ※후속 5건(좌클릭·hot-swap·click·boot/reconnect)은 step-log 미반영(커밋 메시지엔 있음). 다음 세션이 정리하려면 step-log 한 줄 추가.
5. push 안 함 ✅
6. working tree: 내 변경 전부 커밋, 잔여=dashboard-sub docs 3건+.ccb(로컬) ✅
7. **잔여 검증:** 트레이 네이티브 육안(좌클릭=UI·메뉴off·아이콘 색·X=hide)뿐.
