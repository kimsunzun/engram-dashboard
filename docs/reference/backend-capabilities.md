# 백엔드 기능 대조표

우리가 붙이는 **에이전트 CLI 들이 실제로 무엇을 주는가**를 한 곳에 모은다. 코드가 아니라 **바깥 사실**이 대상이다 — 우리 구조는 `structure/agent-backend.md`, 이 문서는 그 구조가 기대는 **상대편 계약**을 적는다.

**쓰는 법 — 기능을 붙이기 전에 이 표의 그 줄을 먼저 채운다.** 비어 있으면 조사부터 하고, 조사 결과를 여기 적은 뒤 코드로 간다. 그때그때 찾아 쓰고 버리면 다음 세션이 같은 것을 다시 판다(실발생: `clientUserMessageId` 를 못 찾아 ADR-0193 이 「합칠 키가 없다」로 기각됐는데, 그 필드는 그때도 있었다).

## 이 표의 규칙

- ★**모르는 칸은 비워 두고 `미확인` 으로 적는다 — 추측으로 채우지 않는다.**★ 빈 칸은 다음 조사 대상 목록이다.
- ★**버전을 함께 적는다.**★ 여기 적힌 것은 **그 버전에서 관측한 사실**이고 벤더가 바꾸면 낡는다. 버전 없는 줄은 못 믿는다.
- **근거 등급을 붙인다:** `소스` = 벤더 소스/스키마 직독 · `실측` = 우리가 실제로 돌려 봄 · `문서` = 벤더 문서 · `관측` = 값이 그렇게 보였을 뿐.
  ★**`문서` 는 가장 약한 등급이다**★ — codex 공개 문서는 thread id 예시로 `"thr_123"` 을 쓰는데 **실서버가 그 값을 거부한다**(실측). 문서와 소스가 갈리면 소스를 믿는다.
- 우리 쪽 구현 여부는 여기 적지 않는다 — 그건 코드와 TRD 소관이다. 이 표는 **상대가 주는 것**만 적는다.

**마지막 갱신:** 2026-09-14 · **claude** 2.1.170 · **codex** 0.154.0

---

## 1. 세션 정체성·복원

| | claude | codex (app-server) |
|---|---|---|
| **누가 id 를 발급하나** | **우리** — 우리가 뽑아 `--session-id <uuid>` 로 강제한다 (소스) | **codex** — `thread/start` **응답**의 `thread.id` 로 받는다 (소스·실측) |
| **id 형식** | 우리가 뽑은 UUID | **UUID** — 타입이 `uuid::Uuid` newtype 이고 역직렬화가 `Uuid::parse_str` 를 부른다. 생성은 `Uuid::now_v7()`. **비-UUID 는 서버가 거부**한다 (소스·실측). ★단 JSON 스키마엔 그냥 `string` 으로 나온다 — 생성기 탓이 아니라 **그 타입이 스키마를 무제한 String 으로 직접 구현**하기 때문이다★. 그리고 벤더 주석은 「**codex 가 발급한** id 는 UUIDv7」로 한정한다 — 장래 hosted·import 경로까지 보장하지 않는다 |
| **이어붙이는 방법** | 스폰 **인자** `--resume <uuid>` (소스) | 스폰 **후** 핸드셰이크 다음 첫 요청을 `thread/start` 대신 `thread/resume { threadId }` 로 (소스) |
| **모르는 id 를 주면** | 미확인 | **에러** `-32600` `no rollout found for thread id` — 조용히 새 대화를 열지 않는다 (실측). ★**`-32600` 을 「모르는 스레드」 신호로 쓰면 안 된다**★ — 설정 로딩 실패 등 다른 잘못된 요청도 같은 코드다. 메시지를 봐야 가른다 |
| **cwd 가 다르면** | 미확인 | **이어진다.** 정체성은 워크스페이스가 아니라 `CODEX_HOME`(`~/.codex`) 단위다. 응답의 `cwd` 는 **스레드에 기록된 원래 cwd** (실측) |
| **시간 만료** | 미확인 | **없다** — 3개월·13 마이너 버전 전 스레드가 그대로 resume 됐다 (실측). 흔히 인용되는 30분은 만료가 아니라 **메모리 언로드 유예**다 (문서) |
| **세션 목록 조회** | 미확인 (`~/.claude/projects/**/*.jsonl` 파일 스캔이 통용되는 우회) | **있다 — `thread/list`.** 커서 페이지네이션 + `cwd`·`archived`·`searchTerm` 등 필터 (소스·실측) |
| **id 를 잃었을 때 회수** | 미확인 | **부분적으로만.** `thread/list` → `thread/resume` 입양이 되고 codex 자신도 데몬 복구에 그 쌍을 쓴다 (소스). ★**단 크래시 창은 이걸로 못 메운다**★ — 아래 참조 |
| **디스크 위치** | `~/.claude/projects/<경로>/<uuid>.jsonl` (관측) | `$CODEX_HOME/sessions/<YYYY>/<MM>/<DD>/rollout-<타임스탬프>-<uuid>.jsonl`. 첫 줄 `session_meta` 가 권위 (실측) |
| **파일명에서 id 복원** | ★**깨진 사례가 있다**★ — 최근 claude 는 transcript 파일명 UUID 가 hook 이 보고한 session_id 와 **다르다**(orca 주석에 박제) | 대체로 되지만 **`thread/revert` 는 스레드 id 를 유지한 채 다른 rollout id 로 새 파일을 만든다** — 권위는 파일명이 아니라 `session_meta.id` (소스) |

**그래서 codex 에만 있는 위험:** id 가 **응답으로 와야** 손에 들어오므로, 받은 뒤 디스크에 적기 전에 죽으면 그 스레드를 잃는다. claude 는 우리가 뽑으니 그 창이 없다.

★**그리고 그 창은 `thread/list` 로 못 메운다 — 적대 검증이 뒤집은 항목이다**★: codex 0.154.0 자기 테스트가 **새 스레드는 첫 사용자 메시지가 오기 전까지 rollout 이 없다**고 단언한다. 즉 `thread/start` 직후 죽으면 id 는 발급됐는데 디스크에 아무것도 없어 `thread/list` 에도 안 잡히고 `thread/resume` 도 못 찾는다. **입양은 「이미 대화가 오간 스레드」를 되찾는 수단이지 크래시 창의 해법이 아니다.** 해법 후보는 **보내기 전에 우리 쪽에 먼저 적어 두는 것**(id 를 받기 전이라도 「이 프로필이 스레드를 시작하려 한다」를 기록) — 아직 설계 안 함.

**덧붙는 한계 둘:** `thread/list` 는 기본 필터에서 archive 된 것과 일부 source 종류를 **빼고** 준다 — 완전한 명부가 아니다. 그리고 `archive` 는 되돌리기 전까지 resume 을 막고 `thread/delete` 는 영구 삭제다.

## 2. 입력과 에코

| | claude | codex (app-server) |
|---|---|---|
| **보낸 입력을 되울려주나** | 기본 안 준다. **`--replay-user-messages`** 를 줘야 되울린다 (소스) ★`--input-format`·`--output-format` 둘 다 `stream-json` 이어야 한다★ | **준다** — `item/*` 의 `userMessage` (소스·실측) |
| **우리 id 를 실을 칸** | 있다 — 우리가 stdin user 라인에 넣은 uuid 를 그대로 되울려준다 (소스) | ★**있다 — `turn/start` 의 `clientUserMessageId`**★. 되울린 항목이 그 값을 `clientId` 로 달고 온다. **`turn/start` 경로에서도 에코된다** — 0.154.0 통합 테스트가 그것을 단언한다(소스). **codex 0.140.0 부터**, 0.135.0 엔 없다 ★공개 문서엔 이 필드가 없어서 문서만 보면 「없다」로 읽힌다★ |
| **같은 id 로 두 번 보내면** | 미확인 | ★**중복 제거를 해 주지 않는다**★ — 같은 `clientUserMessageId` 로 두 번 보내면 **모델에 두 번 갈 수 있는데 화면엔 합쳐져 한 줄로 보인다**. 이 id 는 **상관 키이지 멱등 키가 아니다** (소스·벤더 이슈). 재전송·재연결 경로를 짤 때 이걸 전제로 깔지 말 것 |
| **되울린 내용이 원문 그대로인가** | 미확인 | ★**아니다 — 정규화한다**★. 그래서 codex 자기 TUI 는 매칭이 성사되면 서버 투영을 버리고 **로컬 사본**을 렌더한다 (소스) |
| **벤더 자기 클라이언트는 어떻게 하나** | — | 턴이 안 돌면 **보내기 전에** 로컬 렌더 후 에코는 내용 동일성으로 무시. 턴이 돌면(steer) 로컬 렌더를 **안 하고** 대기했다가 에코 도착 시 로컬 사본을 렌더. 매칭 키는 `clientId` 우선, 없으면 「본문 + 이미지 개수」 폴백 (소스) |
| **구버전 폴백** | — | 내용 비교 키. ★오디오·mention 만 다른 두 전송은 **같은 키가 된다**★(벤더 TODO 주석) |

## 2.5. 기동 인자·모드 (이미 알고 있던 실측 — 흩어져 있던 것을 모은다)

### claude

| 사실 | 근거 |
|---|---|
| JSON 모드는 `-p` **전용**이다 — `--input-format stream-json` · `--output-format stream-json` 과 함께 쓴다 | 소스 · `--help` 가 "only works with --print" 라 적는다 |
| ★**`--verbose` 가 없으면 즉사한다**★ — `--help` 엔 문구가 없는데 런타임이 "When using --print, --output-format=stream-json requires --verbose" 로 죽인다. 증상은 **스폰 직후 에이전트 소멸** | 실측 2026-07-02 (claude 2.1.170) |
| stream-json 헤드리스도 `--resume <sid>` 를 지원한다 — `-p` 계열과 공존하고 "session already in use" 없이 과거 대화를 무손실 재개 | 실측 2026-07-13 (claude 2.1.170) |
| ★**`thinking` 블록은 env `MAX_THINKING_TOKENS` 가 있어야 나온다**★ — CLI 기본은 꺼져 있다 | 실측 2026-07-06 (ADR-0049) |
| 작업 루트는 `--cd` 가 정한다. env 는 넘긴 상태 디렉터리로 실제로 쓴다 | 실측 (계약 시험대가 실 프로세스로 잰다) |
| 세션 id 가 **도중에 바뀐다** — 사용자가 `/clear` 하면 갈린다. 그래서 파일 폴링 감시가 필요하다 | 소스·관측 |
| 제어 채널을 지원한다 | 소스 |

### codex

| 사실 | 근거 |
|---|---|
| **터미널 모드는 세션 개념이 없다** — 그냥 PTY 다. 이어붙이기·되울림 둘 다 해당 없음 | 소스 |
| `thread/start` 파라미터 = `threadId` 없이 `cwd` · `approvalPolicy` · `sandbox` · `clientUserMessageId` · `input` · `turnTrigger` · `toolOutput` · (턴 단위 cwd 덮어쓰기) | 소스 (0.154.0 스키마) |
| `thread/resume` 는 `threadId` 만 필수. 선택 인자 다수 — `cwd` · `model` · `sandbox` · `approvalPolicy` · `excludeTurns` · `initialTurnsPage` · `personality` 등 | 소스 |
| `excludeTurns: true` = `thread.turns` 를 안 채우고 메타데이터와 live-resume 상태만 준다. **이력 전량 하이드레이션은 deprecated** 이고 `thread/turns/list`·`thread/items/list` 와 함께 쓰라고 스키마가 안내한다 | 소스 |
| 우선순위 규칙: 비실행 스레드는 `history` > 비어있지 않은 `path` > `threadId`. `threadId` 가 **실행 중인** 스레드를 가리키면 거기 재합류하고 `path` 는 일치 검사로만 쓰인다 | 소스 |
| **sub-agent 스레드는 단독 resume 이 거부된다** — 부모를 먼저 resume 하거나 `thread/read` 로 들여다봐야 한다 | 소스 |
| `thread/*` 메서드가 70여 개 있다 (`thread/read` · `thread/turns/list` · `thread/items/list` · `thread/search` · `thread/fork` · `thread/archive` · `thread/delete` …) | 소스 |
| `history` · `path` 인자는 **UNSTABLE** 로 표시돼 있다(`history` 는 Codex Cloud 전용, 스키마가 "DO NOT USE" 라 적는다) | 소스 |

### gemini

붙인 적이 없다. 명령 변형도 갈림 표 항목도 없어 **도달 불가**이고, 이 문서에 적을 관측이 하나도 없다.

## 3. 승인·권한

| | claude | codex (app-server) |
|---|---|---|
| **승인 요청이 오나** | 우리는 `--permission-mode bypassPermissions` 로 무조건 주입해 안 오게 한다 (소스 · ADR-0097) | `thread/start`·`thread/resume` 에 `approvalPolicy`·`sandbox` 를 넘긴다. **실제 요청이 오는 경로는 우리가 실행해 본 적이 없다** — 미확인 |

## 3.5. 붙일 때 밟는 함정 (피어 구현이 주석으로 남긴 것)

- ★**codex 항목은 두 번 온다**★ — `item/started` 와 `item/completed` 로 같은 항목이 두 번 도착한다. 중복 제거 없이 렌더하면 두 벌 뜬다 (피어 구현이 그 가드를 둔다).
- ★**`thread/list` 행의 `cwd` 는 optional 이다**★ — 없는 행을 「cwd 가 같다」로 취급하면 엉뚱한 스레드에 매칭된다. cwd 로 거를 땐 **없는 행을 버리고**, 목록 상한을 넉넉히 잡는다(대부분 행이 다른 cwd 라 기본 상한으론 못 찾는다).
- **`thread.sessionId` 와 `thread.id` 가 같은 값으로 관측된다** — 둘이 항상 같다는 보장은 확인 못 했다. 하나를 다른 하나의 대용으로 쓰지 말 것 (관측).
- **claude 의 transcript 파일명 UUID 는 세션 id 와 다를 수 있다** — id 로 파일 경로를 조립하는 코드는 깨진다. 경로가 필요하면 경로를 따로 들고 다닌다 (피어 구현의 실제 회귀).
- **파일을 원자적으로 바꾸는 것과 내구성은 다르다** — rename 은 읽는 쪽엔 원자적이지만 fsync 없이는 전원이 끊기면 옛 내용이나 빈 파일이 남는다 (피어 구현 주석).

## 4. 미확인 — 다음 조사 대상

- **claude 쪽 칸 다수** — 위 표에서 `미확인` 인 줄 전부. claude 는 오래 써 왔지만 이 축들을 정리해 둔 적이 없다.
- ★**크래시 창을 어떻게 메울지 — 설계가 없다**★. 입양이 답이 아니라는 것만 확정됐다(§1). 후보 = 보내기 전에 우리 쪽에 먼저 적는 내구 기록(스레드 id · 우리 메시지 id · 보낸 내용 · 상태를 한 덩어리로).
- **애매한 끊김에서의 복구** — codex 는 요청을 받았는데 우리가 응답을 못 받(거나 적기 전에 죽)은 구간. 위 「멱등 아님」과 맞물려 **재전송이 안전하지 않다**.
- **archive 된 스레드**를 우리가 만나면 어떻게 보이나 — resume 이 막힌다는 것만 안다.
- **같은 스레드를 둘이 동시에 열면** 어떻게 되나 — 소유권 규칙 미확인.
- **rollout 파일이 손상·누락됐을 때** 의 동작 미확인.
- **재시작 후 `CODEX_HOME` 이 달라지면** 이어붙이기가 통째로 실패한다 — 우리가 그 값을 고정하고 있는지 확인 안 했다.
- **hosted/cloud 백엔드가 비-UUID thread id 를 주나** — 로컬 app-server 는 비-UUID 를 거부하지만 벤더 주석이 「codex 가 발급한 id 는 UUIDv7」로 한정한다.
- **rollout 파일 정리(GC)·보존 정책** — 「만료 없음」은 성공 1회에 근거한 하한이다.

## 조사 방법 메모 (다음 사람이 싸게 하도록)

- ★**벤더 CLI 는 `--help` 로 스키마 내보내기 서브커맨드를 먼저 찾는다**★ — codex 는 `codex app-server generate-json-schema` · `generate-ts` 가 있어 **우리가 쓰는 바로 그 버전의 스펙**을 뽑는다. GitHub main 보다 정확하다.
- ★**「이 백엔드에 X API 가 있나」는 API 문서보다 그것을 쓰는 클라이언트 클론을 먼저 grep 한다**★ — 존재뿐 아니라 실제 파라미터와 함정까지 같이 나온다. 클론 위치 = `../../../Engram_Workspace/opensource/`(repo 밖 · 미추적).
- 웹 검색은 이 부류에서 수확이 낮았다 — 이슈 제목 말고는 결정적 근거를 준 적이 거의 없다.

## 출처

벤더 소스는 `openai/codex` 의 `codex-rs/protocol/src/thread_id.rs` · `codex-rs/app-server-protocol/src/protocol/v2/thread.rs` · `codex-rs/app-server/README.md` · `codex-rs/tui/src/chatwidget/`. 공개 문서는 `learn.chatgpt.com/docs/app-server`(★예시가 실제와 어긋난다★). 피어 구현은 위 클론의 `paseo`(codex app-server 실사용) · `orca`(에코 매칭·durability 주석).
