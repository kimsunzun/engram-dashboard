# 핸드오프 — capability 합성(ADR-0030) + 스킬/문서 하네스 정비 + I:/Engram agents 정리(office-butler 분리·web-runner 폐기) + 컨텍스트위생 룰 글로벌 승격

작성 2026-06-26 (dashboard-main 세션). 본문(`docs/decisions/`·`docs/process/step-log.md`·`CLAUDE.md`)이 항상 우선. **이번 세션은 코드 변경 거의 없음 — capability 1건 + 문서/스킬/하네스 정비 위주.**

## ★★ 다음 세션 첫 행동 (필독) ★★
1. **읽기:** 이 핸드오프 → **ADR-0030**(capability 합성) → **ADR-0031**(검수체계=opus+Codex 2자) → ADR-0032(주석 컨벤션) → step-log dashboard11/main 엔트리.
2. **규약(중요·바뀜):** 비자명 변경 = 코더(opus)→**`/review` (opus + Codex 2자 적대, ADR-0031)**→**`/qa`** 게이트. 스킵 금지. 조사/웹/대량읽기는 **서브에이전트·`/research` 일임**(이제 global-rules에도 박힘 = 전 프로젝트 공통).
3. **다음 작업 = 프론트 본작업**, 단 **터미널 렌더 버그**(아래 §4)가 그 토대라 먼저 본다.

## 0. 한 줄 요약
① capability 산출을 transport(물리)⊕backend(프로그램) **합성**으로 정확화(shell resume=false 교정, ADR-0030). ② 스킬 4종(review/qa/adr/research) 하네스 정비 — flow 포인터 명령형 게이트화·피드백 절 공용 추출·학습용 usage-log(권장) 추가. ③ CLAUDE.md embedded 잔재 정리. ④ **I:/Engram repo** agents 정리: web-runner 폐기, office-butler(회사콘텐츠)를 Engram_Workspace(git 밖)로 분리, 컨텍스트위생 룰을 global-rules로 승격 — **이건 I:/Engram에 커밋·push 완료**.

## 1. 두 개의 repo — 헷갈리지 말 것
- **engram-dashboard** (`I:/Engram/apps/engram-dashboard`) — 이 프로젝트. **이제 origin 보유**(옛 핸드오프 "push 안 함"은 outdated). 현재 **master, ahead 4**(아래 미커밋/미푸시).
- **I:/Engram** (루트) — 개인 워크스페이스 sync repo, 원격 `github.com/kimsunzun/Engram.git`(**PRIVATE**), "집 PC 이어작업"용. 이번 세션 agents 정리분 **커밋·push 완료**(`d2709b0`).
- **I:/Engram_Workspace** — **git 밖**(추적 안 됨). 회사 민감 콘텐츠 파킹지.

## 2. engram-dashboard — 이번 세션 커밋 (master, ahead 4 = 미푸시)
| 커밋 | 내용 |
|---|---|
| `8640ec0` | **feat(capability) ADR-0030** — `TransportCaps`(물리)⊕`BackendCaps`(프로그램)→`Capabilities::compose`. claude resume=true/shell=false. 코더→reviewer-deep→QA(cdp 실측: 실제 IPC로 shell resume=false 확인) 통과. core test 76 green. ※이미 origin에 있음 |
| `481c5ee` | docs(rot) — ADR-0024 idle self-shutdown 미구현 정정 + CLAUDE.md 모듈맵 daemon-only. ※origin |
| `1dadec2` | docs(step-log) — 백엔드 잔여 처리방침. ※origin |
| `d1d2852` | docs(claude) — embedded 잔재 2곳(§4 AppState·§프론트 EmbeddedClient) daemon-only 정리. **(ahead, 미푸시)** |
| `88602ee` | refactor(skill) — 자기개선 피드백 절 4벌 복붙 → `_shared/self-improvement-feedback.md` 단일출처+포인터. **(미푸시)** |
| `0536450` | refactor(skill) — flow.md 포인터 서술형→**명령형 게이트**("실행 전 flow 반드시 Read, 즉석 발명 금지"). **(미푸시)** |
| `ebbd03d` | feat(skill) — 학습용 **usage-log 규약(권장·강제 아님)** 추가(review/qa/adr; research는 study-notes가 겸함). **(미푸시)** |

**미커밋(working tree):**
- `CLAUDE.md` M — **§55 컨텍스트위생 줄을 "global-rules 참조 + engram 바인딩(/research)"으로 슬림화**(global 승격의 engram 측, 옵션A). 사용자가 "확인해볼게" 한 상태라 **미커밋으로 둠** — 검토 후 커밋.
- `docs/research/multi-agent-hosting-orchestration-research-2026-06-22.md` ?? — 내 작업 아님(병렬/이전 세션 산출). 손대지 않음.

## 3. I:/Engram repo — 커밋·push 완료 (`d2709b0`, origin/master)
- **web-runner 폐기**(git 삭제). 단 **디스크에 빈 폴더 `agents/web-runner` 잔존** — 아직 그 폴더를 cwd로 둔 세션이 있어 빈 디렉토리만 제거 막힘. git 무관. 그 세션 닫고 `Remove-Item -Recurse -Force I:\Engram\agents\web-runner`.
- **office-butler → `I:\Engram_Workspace\agents\office-butler` 이동(보존)**, git에서 본체+잔재(`links/office-butler.md`·`engram-manager/.claude/agents/office-butler.md`·`team-office-butler` 스킬) 제거. ※**private 이력엔 잔존**(스크럽 안 하기로 합의 — 필요시 filter-repo 별도작업).
- **global-rules.md** — `## 컨텍스트 위생 — 수집성 작업은 서브에이전트로` 룰 추가(전 프로젝트 공통).
- 그 외 워킹트리 일괄 동기화(skills 정리·obsidian·wezterm 등 48파일).
- **주의(잔재):** `engram-manager/CLAUDE.md`가 삭제된 office-butler를 아직 참조(dangling) — 다음에 engram-manager 손볼 때 정리.

## 4. ★다음 = 프론트 본작업 (단 터미널 렌더 버그가 토대)★
백엔드 잔여 ③④⑤⑥⑦는 전부 보류/이연 확정(③자동재시작=메모만 ④idle self-shutdown=보류 ⑤codex/gemini=넘어감 ⑥WSS ⑦메시지시스템=프론트 쓸만해진 뒤). **다음=프론트:** D-7 레이아웃/창 영속(localStorage) · §5 LLM 제어표면(command 버스) · 클로드 화면.

**★터미널 렌더 버그(실측 발견)★** — claude welcome 화면 깨짐(글자 겹침). 진단:
- 확실: spawn 시 PTY가 **기본 80×24 고정**, xterm은 ~50행 → **초기 fit/resize가 PTY로 전파 안 됨**. `resizePty(id,80,50)` 직접 호출은 동작(파이프 OK), 마운트 시 자동 발사가 안 됨.
- 가능성 높음: 행 맞춰도 겹침 안 사라짐 → **폭 문제**. pane이 claude에 80칸만 줘서 welcome 박스(>80칸)가 양끝 클리핑+덧그림.
- 함의: "클로드 화면"이 프론트 토대인데 터미널 렌더 파이프라인(fit→PTY resize 전파→redraw)이 깨져 있음. **착수 시 `/prior-art`나 `/research`로 xterm.js fit/resize/초기사이징 OSS 관행 조사 후 진행**(사용자: 바닥부터 짜지 말고 참조). 관련 코드: `src/components/slot/TerminalSlot`, `src/api/wsTransport`·`protocolClient`(resizePty).
- 라이브 검증 수단: `scripts/cdp.mjs`(포트 9223), `window.__ENGRAM_AGENT__`(resizePty/spawnAgent/getAgents).

## 5. 규약·하네스 변화 (이번 세션 — 반드시 인지)
- **검수 = opus + Codex 2자 적대(ADR-0031)** — `/review [prd|trd|code|doc] [self|light|full|deep]`, `/qa [quick|standard|full]`. 정본 = `.claude/skills/review/references/flow.md §2`. (reviewer-deep 단독은 옛 방식)
- **스킬 3층 구조 인지:** SKILL.md(description 상주) → references/flow.md(실행 정본, **자동 로드 X — Read 필수**) → bindings/<project>.md. flow 포인터가 명령형 게이트화됨.
- **컨텍스트 위생 = global-rules에 박힘** — 웹/대량읽기/OSS조사는 서브에이전트·`/research` 일임, 결론만 회수. 핀포인트/즉답만 인라인.
- ADR-0030(capability 합성)·0031(검수)·0032(주석 2계층) 신규. ADR 인덱스 = `docs/decisions/README.md`.

## 6. 열린 스레드 / 미결
1. **CLAUDE.md §55 슬림 미커밋** — 사용자 검토 대기(§2). 검토 후 커밋 or 수정.
2. **dashboard ahead 4 미푸시** — origin 있음. 푸시 여부 사용자 결정(이번 세션 미요청).
3. **web-runner 빈 폴더** — 세션 닫고 삭제(§3).
4. **engram-manager dangling office-butler 참조**(§3).
5. **주석 정책** — 사용자가 "지금 정도 유지" 잠정 + 리서치 후 검수 의향이었으나 ADR-0032가 이미 착지. 흐지부지 — 필요시 재확인.

## 7. 환경/주의
- 빌드 잠금(os error 5): 실행 중 dev/exe → `taskkill //F //IM engram-dashboard.exe //IM engram-dashboard-daemon.exe`. cdp 포트 9223 고정.
- 커밋 멀티라인 = PowerShell here-string(`@'...'@`). 커밋 트레일러 Co-Authored-By.
- **Windows cwd-lock 주의:** agents 폴더 이동/삭제가 "file in use"로 막히면 그 폴더를 cwd로 둔 claude/셸 세션 때문. `Get-CimInstance Win32_Process | ? CommandLine -like *<dir>*`로 PID 찾고 사용자 확인 후 종료(임의 kill 금지 — claude 세션 다수).
- I:/Engram는 PRIVATE GitHub sync(집 PC와 공유) — force-push/이력재작성 시 집 PC clone 깨짐 주의.
