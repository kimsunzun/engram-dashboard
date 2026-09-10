# TRD — codex 백엔드 Phase 2a (S21)

> 상태: **5판(2026-09-10) — 일곱 ADR + 사용자 결정 셋을 반영한 판.** 승인된 설계 브리핑이 확정한 3단계 중 **둘째 단계의 앞 절반**만 다룬다. Phase 0·1 의 TRD 는 `trd.md` 이고 이 문서가 그것을 대체하지 않는다 — 그쪽은 시점 기록이고 이쪽은 그 다음 단계다(ADR-0185 「영향」의 처분 그대로).
>
> **읽는 법:** 이 문서는 codex 동작에 관한 모든 주장에 **출처 등급**을 붙인다 — `[실측]`(이 PC에서 실제로 돌려 본 것) · `[문서]`(codex 공식 문서·README·상류 소스·이슈를 읽었을 뿐 돌려 보지 않은 것) · `[스키마]`(codex 자신이 내보내는 JSON 스키마·생성 TS 를 읽은 것) · `[미확인]`(어느 쪽도 아닌 것). **등급 없는 codex 주장은 이 문서에 없어야 한다.** 이 저장소 코드에 관한 주장은 등급 대신 `file:line` 을 단다(판독 시점 2026-09-09, 4판 갱신분 2026-09-10).
>
> ★**표기 하나 — `§10-N` 은 절 번호가 아니다**★. §10 에는 하위 절이 `10-A`~`10-D` 넷뿐이고, 본문이 `§10-13`·`§10-16` 처럼 가리키는 숫자는 **그 안 표의 행 번호**(`#` 열)다. 닫힌 것은 **10-A 표**에서, 열린 것은 **10-B 표**에서 그 번호를 찾는다. `10-6-b`·`10-11-a` 처럼 알파벳이 붙은 것은 그 행에서 갈라져 나온 항목이다.
>
> > ★**그 표기는 이제 하나뿐이다 — 5판이 섞여 있던 것을 걷었다**★. 4판까지는 같은 행을 `§10-5` 와 「§10-A 표 5 행」 두 가지로 가리켰고, 그래서 **같은 행이 두 이름으로 인용됐다**. **본문 형태는 `§10-N` 하나로 고정한다.** 옛 판·리뷰·인계 메모에서 「§10-A 표 N 행」·「§10-B 표 N 행」을 만나면 **그것이 `§10-N` 과 같은 것**이고, 앞의 `A`/`B` 는 위 문단이 이미 말한 「닫혔나 열렸나」를 되풀이한 것뿐이다. ★**새로 쓸 때 긴 형태를 되살리지 말 것**★.
>
> ★**초판·2판과 달리 이 판은 고른 것을 적는다**★ — 1·2 판은 「이 문서는 아무것도 고르지 않는다」로 서 있었고 §5 가 선택지 넷을, §10 이 미결 열여섯을 이고 있었다. 그 사이에 **ADR 일곱이 핵심을 닫았다**: **ADR-0187**(Phase 2 통로 = `codex app-server` — `exec`·SDK 는 실측으로 기각) · **ADR-0188**(권한·승인을 백엔드 중립 축으로, 값 설계는 후속) · **ADR-0189**(app-server 통로 = `backend/codex/` 안의 `AgentTransport` 구현체) · **ADR-0190**(json 모드 입력 큐) · **ADR-0191**(백엔드가 자기 통로를 만들어 넘긴다 — 가르는 switch 는 한 곳뿐) · **ADR-0192**(백엔드 동작의 기본값을 claude 와 맞춘다 — 권한값 하나 · 죽은 뒤 재연결 안 함) · **ADR-0193**(큐 해제 판정의 주인을 각 통로 구현체로 — ADR-0190 결정 4 의 개정). ★**그리고 사용자 결정 셋이 5판에서 더 닫았다**★ — 입력 영수증은 하나로 퉁친다(§4-10 · §10-20) · 무기한 쓰기 정지에 강제 종료 감시를 만들지 않는다(§4-9 · §10-21) · 실행 모드 표현은 열어 두되 **구현 진입 앞**에 답한다(§10-7 ① · §10-C). ★**닫힌 것은 「누가 닫았나」를 함께 적고, 안 닫힌 것은 열린 채로 §10·§11 에 남는다**★ — 이 문서가 스스로 고르는 것은 여전히 없다(CLAUDE.md 「개발 스텝」의 순서 불변).
>
> ★**전제가 결정으로 바뀌었다**★ — 1·2 판은 「app-server 가 맞는 경로」를 §10-8 의 미결 전제로 이고 있었다(조사 표본 편향 판정 — §2 「뒤집힌 자리」 HIGH 5). **ADR-0187 이 세 모드를 실제로 돌려 그 항목을 닫았다** — `exec --json` 은 글자를 흘리지 않고(중단까지 두 줄) 공식 SDK 는 존재하지 않는다. 그래서 §3~§6 이 다시 열릴 조건은 이제 그 ADR 의 「다시 열 트리거」 둘뿐이다.
>
> **가리키는 두 문서(★베끼지 않는다★):**
> - **실측 정본** = `.claude/handoff/attachments/codex-measurements-2026-09-09.md` — codex-cli 0.153.4 · Windows 11 · 2026-09-09 의 실행 스냅샷. 이 문서의 `[실측]` 행은 전부 그 파일에서 왔고, ★버전이 바뀌면 그 파일이 낡는다★(그 파일 자신이 그렇게 도장 찍는다).
> - **소유권 지도** = `docs/reference/structure/session-path-ownership.md` — 세션 경로의 객체별 소유·수명 추적(2026-09-09 실측 스냅샷). ADR-0189 가 「막힌 자리」를 확정한 근거이고, ★쓸모 있는 부분은 **PART B**(설계가 실제로 답해야 하는 질문 열)와 **PART C**(「문이 없다」 목록)다★ — 통째로 읽지 말고 그 둘을 집어 읽는다.
>
> 앵커: **ADR-0187**(통로 확정) · **ADR-0188**(권한 중립 축) · **ADR-0189**(통로의 자리) · **ADR-0190**(입력 큐) · **ADR-0191**(통로 생성 seam) · **ADR-0192**(claude 기준 기본값) · **ADR-0193**(큐 해제 판정의 주인) · **ADR-0088**(배달 관측 — 완결성 = Ok-vs-Err) · **ADR-0185**(세션 복원 = 발급 주체 중립 — 이 단계의 sid 축 근거) · **ADR-0186**(무엇을 어디에 적나 — 이 문서가 「이 작업만 필요」 그릇이다) · **ADR-0004**(백엔드 지식 격리 — 이 작업 전체의 근거) · ADR-0002/0030(capability 산출·출처 분리) · ADR-0044(입력 인코딩·통로 무정제) · ADR-0045(tag1 StructuredEvent) · ADR-0001(kill 인과) · ADR-0005(finalize 1회) · ADR-0012(모듈 격리·단독 하네스) · ADR-0099(채널 capability 트립와이어) · ADR-0113(턴 신호 분류자) · ADR-0127(턴 관측 사실 층) · ADR-0145(챗 빈 상태 구성) · ADR-0110(메시징 커널 격리 — 2b) · ADR-0129(net 경계 — 손대지 않는 쪽) · ADR-0163/0164(화신 표식·구독 키) · `trd.md` §0·§2·§4-8·§4-9·§6-1 · `.claude/handoff/attachments/codex-app-server-survey.md`(조사 보고서 + 그 적대 리뷰).

---

## 0. 범위

### 이 문서가 정하는 것

**Phase 2a = 상주 JSON 서버(`codex app-server`) transport + 번역기(decoder) + 그 출력이 화면에 중립 어휘로 그려지는 것까지.**

넷을 정한다: **① 프로세스·연결·스레드·턴 네 상태기계**(§3) **② wire 규율 + json 모드 입력 큐**(§4) **③ 통로의 모양 — ★ADR-0189 가 확정한 그대로★**(§5) **④ codex 이벤트 → 우리 중립 어휘 번역 표**(§6). 그리고 그 산출물이 오늘 프론트에 닿을 때 무엇이 깨지는지를 갭 셋으로 적는다(§7).

### 이 문서가 정하지 않는 것 (범위 밖 — 재론 금지)

| 항목 | 어디로 | 이유 |
|---|---|---|
| resume 배선 셋 — 수령 배선 · `needs_session()` 쪼개기 · 활성화 입구 가드 | **Phase 2b** | ADR-0185 결정 2 의 ①②③ 그대로다. **첫 게이트(§9)가 스레드 생성 응답의 실제 모양을 확정하기 전에 설계하면 그 실측이 뒤집을 문장을 쓰게 된다** — 그 ADR 자신이 「거기서 어긋나면 결정 2가 흔들린다」로 적는다 |
| 우편(에이전트 간 메시징) 수신 | **Phase 2b** | `reads_messages()` = false 의 사유가 「바쁜 때를 못 가린다」다(`crates/engram-dashboard-agent/src/backend/codex/mod.rs:76-77` — 그 doc 은 `:76-81`, fn 은 `:82-84`). 켜는 조건 = 턴 관측이고 그 관측이 **2a 의 산출물**이다 — 2a 가 끝나야 켤 값이 생긴다. ★**단 「2a 가 끝나면 관측이 생긴다」가 자동은 아니다 — 조건 넷이 다 서야 한다**★: §7-4 가 그 넷을 줄로 적는다. 하나라도 빠지면 2b 가 이 값을 여는 순간 **턴 중에 편지가 꽂힌다** — 그리고 그 넷 중 하나(`turn_classifier` 선언)는 오늘 `backend/codex/mod.rs` 에 **없다**(§8) |
| 승인 UI | **Phase 2b** | 승인은 서버→클라 **요청**이라 답을 안 주면 에이전트가 멈춘다`[문서]`. ★**축은 여전히 ADR-0188 이 소유한다**★(권한·승인은 백엔드 중립, 값 설계는 후속 ADR). ★**단 2a 가 띄울 때 넣을 값 하나는 ADR-0192 가 정했다 — 승인 안 묻기 + 작업폴더 쓰기, 잠정**★(§10-5 · ★이 통로에서는 argv 가 아니라 `thread/start` params 로 간다★). **사람이 고르는 화면은 2b** — 이 문서가 만드는 것은 「안 묻는다」 하나이고 「물으면 어떻게 보여 주나」가 아니다 |
| 프론트 생성 경로의 백엔드별 갈림(갭 4) | **Phase 3** | 생성이 파라미터가 아니라 **메서드로** 갈려 있다(`src/api/agentClient.ts:204,217` · `src/api/protocolClient.ts:878,899`)+백엔드 박힌 command id·i18n 키(`src/commands/agentCommands.ts:136,177` · `src/i18n/ko.ts:57`). 2a 는 그 문을 안 건드린다 — codex 를 띄우는 문은 Phase 1 이 이미 열었다 |
| 프론트에 샌 claude 스키마 해석의 **전면** 회수 | **Phase 3** | `trd.md` §6-1 사용자 결정 — 「프론트의 claude 스키마 해석은 이번에 안 걷는다」. ★단 2a 가 **그것과 부딪히는 자리 셋**은 피할 수 없다 — §7 의 갭 1·2·3 이 그것이고, 그 셋의 처분은 §10 에 오른다★ |
| MCP 주입(`-c mcp_servers.…`) · 시스템 프롬프트 주입 | Phase 2b 이후 | `accepts_mcp_config()` = false 를 그대로 둔다(사유 doc = `backend/codex/mod.rs:67-70` · fn = `:72-74`). 기제가 claude 와 다르다는 사실만 기록돼 있다(`trd.md` §2 M5) |
| 프로세스 모델(대화마다 하나) | 브리핑에서 이미 확정 | 조사 보고서의 후보 B(한 프로세스가 여러 스레드)는 그 결정과 부딪힌다 — 재론하지 않는다 |

### 다른 세션이 쥐고 있는 파일 — 손대지 않는다

`trd.md:31-41` 이 적은 경계를 그대로 잇는다. **Phase 2a 의 어느 변경도 여기 들어가지 않는다.**

- `src-tauri/src/daemon_client/**`
- `crates/engram-dashboard-net/**` (그 crate 의 경계·격리 게이트 정본은 그 `src/lib.rs` 헤더 · ADR-0129)
- 워크스페이스 루트 `Cargo.toml` 의 members 목록

★**이 세션은 그 병렬 작업이 아직 진행 중인지 다시 확인하지 않았다**★ `[미확인]` — 위 목록은 `trd.md` 가 2026-09-07 에 적은 상태를 옮긴 것이다. 끝났다면 경계가 풀리지만, 끝났다는 관측이 없으므로 **끝난 것으로 가정하지 않는다.** 착수 전에 확인할 일이다.

---

## 1. 왜 2a 가 먼저인가

```
Phase 0  터전 + 실측        (완료) 시험대 + 실 codex PTY 관측
Phase 1  배선              (완료) variant · dispatch · wire 칸 · 프론트 · 트립와이어
   ↓
Phase 2a 상주 서버 + 번역기  이 문서. 통로(= 상관까지 안에서 끝난다)·번역기·화면까지
   ↓     게이트: §9 의 첫 게이트(스레드를 만들어 id 를 받는다)
Phase 2b resume 배선 · 우편 · 승인 UI
   ↓
Phase 3  프론트 해석 회수 · 생성 경로 정리
```

**순서를 정한 것은 브리핑이다** — 「상주 서버 + 번역기 먼저」. 그 위에 이 문서가 더하는 근거 둘:

★**resume 세부는 지금 설계할 수 없다**★ — 그것이 매달린 것은 첫 게이트 하나였다: **실제로 스레드를 만들어 그 id 를 받는 것.** ★**그 게이트의 앞 절반은 2026-09-09 에 통과했다**★ — `thread/start` 응답을 받았고 `result.thread.id` 가 **UUIDv7** 이며 `thread.sessionId` 와 같았다(§2 L4). ★**그러나 뒷 절반은 그대로 남았다**★ — `thread/resume` 을 **한 번도 부르지 않았다.** 그 앞에서 배선을 설계하면 **실측이 뒤집을 문장을 쓰는 것**이고, 그것이 Phase 0 을 앞세운 것과 같은 판정이다(`trd.md` §1: 「값어치는 「먼저 안다」가 아니라 「틀렸을 때 배선을 안 짓는다」다」).

★**그리고 2b 의 수령 배선은 「먼저 떼어 낼 수 있는 것」이 아니다**★ — codex 가 우리에게 스레드 id 를 알려 주는 채널은 **app-server 응답 하나뿐**이다`[실측]`. 2a 가 없으면 **받을 것이 없다.** 오늘 `observe_session_id` 의 유일한 생산 호출자는 claude `/clear` watcher 이고(`crates/engram-dashboard-daemon/src/lib.rs:288`), 그 watcher 는 codex 의 수령 경로가 될 수 없다 — `session_id_source` 가 **우리가 발급한 기준 sid**(`expected_sid: Uuid`)를 요구하는 파일 폴러이기 때문이다(`crates/engram-dashboard-agent/src/backend/mod.rs:348-353`). 즉 의존 방향이 한쪽이다: **2a → 2b.** 그 반대는 없다. ★단 **그 콜백을 만드는 조립 모양**은 codex 가 그대로 물려받는다 — 그것이 ADR-0189 가 고른 기록 경로다(§5-6).★

### 2a 의 첫 게이트가 무엇을 뒤집을 수 있나 — ★네 행 중 셋이 답을 받았다★

| 관측 | 무엇이 흔들리나 | 결과 |
|---|---|---|
| 생성 응답에 id 가 **안 실려 온다** | ADR-0185 **결정 2**(수령 지점 = 기존 `observe_session_id`)와 §6 의 순서 제약이 놓일 자리 | ★**안 흔들렸다**★ — `thread/start` 응답의 `result.thread.id` 로 왔다`[실측]` |
| id 가 **UUID 가 아니다** | `backend_session_id: Option<Uuid>`(`crates/engram-dashboard-agent/src/profile.rs:165`)의 **타입** | ★**안 흔들렸다**★ — UUIDv7 로 파싱된다`[실측]`. 「새 필드·새 매핑을 만들지 말 것」이 그대로 선다 |
| id 가 turn 마다 **재발급된다** | 결정 2 전체 + §3 스레드 상태기계 | ★**안 흔들렸다**★ — 같은 `threadId` 로 둘째 턴이 돌았다`[실측]` |
| `thread/resume` 이 순차 재시작 뒤에 **안 선다** | Phase 2b 전체(resume 이 성립하지 않는다) · `session.resume` capability | ★**아직 미관측**★ — §9-1 이 그대로 이 한 행을 진다. **결정 1 은 여기서도 안 흔들린다** — 「복원이 무엇에 의존하나」의 답이라 복원이 불가능해도 그 문장은 안 틀리고, 대신 **그 축의 기능이 없어진다** |

★**결정 1 이 어느 행에서도 안 흔들린 이유**★ — 그것은 **스키마(`ThreadStartParams` 에 id 계열 속성 0개) + 상류 거절** 위에 서고, 위 넷은 전부 **응답 쪽** 관측이다(ADR-0185 자신이 그 구별을 적어 두었다).

★**2a 는 이 순서 제약 하나를 지고 간다**★ — ADR-0185 「영향」의 Phase 2 요구사항 중 하나가 2a 안에 떨어진다: 「응답으로 받은 sid 는 **첫 턴을 허용하기 전에** persist 한다」. **2a 가 그 응답을 소유하는 유일한 단계**라 다른 데 걸 자리가 없다. 그 ADR 이 그것을 「아직 불변식이 아니다 — 성립한 불변식으로 인용하지 말 것」으로 도장 찍었고, 지금 강제하는 것이 아무것도 없다. **설계가 가능해지는 시점이 2a 다** — 자리가 §6 끝이고, ★그 게이트를 지는 객체를 ADR-0189 가 확정했다★(통로 구현체 — §5-1 · §6-3).

---

## 2. app-server 에 대해 우리가 아는 것 — 출처 등급별

대상: **codex-cli 0.153.4, 이 PC, 인증된 상태.** 상류 소스 판독은 `main` 이었다 — ★그 둘 사이의 드리프트는 `[미확인]`★.

★**출처 정본이 둘이고 우선순위가 있다**★ — **`[실측]` 의 정본 = `.claude/handoff/attachments/codex-measurements-2026-09-09.md`**(실행 스냅샷, 측정 2026-09-09 17:18~17:29 KST). **`[스키마]`·`[문서]` 의 정본 = `.claude/handoff/attachments/codex-app-server-survey.md`**(판독 보고서 — F 번호는 그 보고서의 발견 번호). ★**둘이 어긋나면 실측이 이긴다**★(그 파일 자신이 그렇게 도장 찍는다). ★그리고 **버전이 바뀌면 실측 문서가 낡는다** — 이 문서는 수치를 베끼지 않고 그 파일을 가리킨다★.

### `[실측]` — 실제로 돌려서 확인한 것

★**1·2 판의 이 표는 넷이었고 「app-server 와의 왕복은 이것이 전부다 — 스레드를 만들어 본 적이 없다」로 닫혀 있었다. 그 문장은 죽었다**★ — 스레드를 만들었고, 턴을 돌렸고, 중단했고, 이어서 한 번 더 돌렸다.

| # | 사실 | 2a 에 미치는 영향 |
|---|---|---|
| L1 | `codex app-server --stdio` 가 **Windows stdio 로 뜬다.** `initialize` → `result{userAgent, codexHome, platformFamily, platformOs}`, **약 130ms** | §4-10 의 큐가 흡수해야 하는 창의 크기가 이 수치다 |
| L2 | **stdin 을 닫으면 종료코드 0** — 46ms(2회차) / 118ms(1회차). ★**단 활성 턴이 없는 상태의 관측이다**★ | §4-8. 턴 도중은 여전히 `[미확인]`(§11) |
| L3 | `codex app-server daemon version` → 「only supported on Unix platforms」. Windows 전송은 **stdio 와 ws 뿐** | 공유 데몬 모드는 안 건드린다 |
| L4 | ★**`thread/start` 응답을 실제로 받았다**★ — `result.thread{id, sessionId, …}` + 형제 `result.{model, approvalPolicy, sandbox, …}`. ★**`thread.id == thread.sessionId` 이고 UUIDv7**★(예 `01a08544-94ef-7043-8a1e-87d22dbf8b49`) | ★`backend_session_id: Option<Uuid>`(`crates/engram-dashboard-agent/src/profile.rs:165`) 칸에 **그대로 든다**★ — §9-1 첫 게이트의 절반이 여기서 답을 받았다 |
| L5 | ★**턴 id 는 `result.turn.id` 이고 `result.turnId` 가 아니다**★. `turn/start` → `result.turn{id, items, itemsView, status:"inProgress", …}` | 번역기·인터럽트 봉투가 그 경로를 읽어야 한다 |
| L6 | ★**app-server 는 조각을 흘린다**★ — 턴 중 `item/agentMessage/delta` 가 `{threadId, turnId, itemId, delta}` 로 연달아 온다(「1부터 30까지 세라」에서 중단 전까지 **10개**) | ADR-0187 이 `exec` 를 기각한 직접 근거. §6-1 의 `TextDelta` 행이 이것 위에 선다 |
| L7 | **턴 중 도착한 알림 11 종**(첫 등장 순서): `remoteControl/status/changed` · `thread/started` · `mcpServer/startupStatus/updated` · `thread/status/changed` · `turn/started` · `item/started` · `item/completed` · `item/agentMessage/delta` · `thread/tokenUsage/updated` · `account/rateLimits/updated` · `turn/completed`. 전부 `emittedAtMs` 를 싣는다 | §6-1 번역 표의 왼쪽 열이 **처음으로 실측 위에 선다** |
| L8 | `item/started`·`item/completed` 에서 본 아이템 타입 = **`userMessage`** · `agentMessage` · `reasoning`. ★`reasoning` 아이템이 `summary:[]`·`content:[]` 로 닫혔다★ — `reasoningOutputTokens: 34` 인데도 비었고 `item/reasoning/textDelta` 계열은 **한 건도 안 왔다**(기본 설정) | §10-6-b ① — **추론 어휘를 늘릴 근거가 실측으로 안 선다.** 그리고 ★`userMessage` 아이템이 실재한다★ = §10-2 의 「우리가 보낸 것」 표시가 붙을 대상 |
| L9 | ★**`turn/interrupt` 가 18ms 에 성공하고 대화가 살아남는다**★ — `{threadId, turnId}` → `{"result":{}}`. 조각이 즉시 멈추고 `turn/completed` 가 `status:"interrupted"` 로 닫히며, **같은 `threadId` 로 다음 턴이 정상 동작했다**. ★단 끊긴 부분은 맥락에 안 실렸다★(「몇까지 셌나」에 `"0"`) | ★취소가 프로세스 종료를 요구하지 않는다★ — §5-3. `turnId: null` 은 `-32600` 으로 거부된다(**문자열 요구**) |
| L10 | ★**중단된 메시지의 `item/completed` 는 오지 않는다**★ — 그 `item/started` 가 닫히지 않은 채 남는다 | ★**「시작한 아이템은 반드시 닫힌다」를 전제한 번역기는 깨진다**★ — §6-1 이 그 전제를 세우지 않는다 |
| L11 | ★**서버→클라 요청이 0 건이다**★ — 두 실행에서 들어온 116줄 중 `method` 와 `id` 를 함께 가진 메시지가 없다. ★**조건 = `approvalPolicy:"never"` + `sandbox:"read-only"`**★ | §5-2 의 첫 행. ★**조건이 바뀌면 이 면제가 죽는다**★(선언은 11 종 — 아래 L14) |
| L12 | ★**서버 응답은 `jsonrpc` 필드를 생략한다**★ — `{"id":N,"result":{…}}` / `{"error":{…},"id":N}`. 응답·알림을 가르는 축은 **방향과 모양**(`method` 유무 · `result`/`error`)이고 ★**id 값이 아니다**★ — 서버 요청 id 와 우리 요청 id 는 값 공간이 겹칠 수 있다 | §4-4 의 판정 축이 실측으로 확인됐다 |
| L13 | ★**stdout 에 비-JSON 줄이 0, stderr 0 바이트**★(두 실행 모두) | §4-6 을 **면제하지 않는다** — 결말이 「데몬이 죽는다」라 §4-5 와 같은 판정이다(D6) |
| L14 | **로컬에서 스키마를 생성할 수 있다** — `codex app-server generate-json-schema --experimental --out <dir>`, 종료코드 0, ★**네트워크·모델 호출 없음**★. 선언 규모 = `ClientRequest` **155종** · `ServerNotification` **81종** · `ServerRequest` **11종** · `ClientNotification` 1종 | ★**번역 표의 빈 행을 추측 없이 채울 경로가 이것이다**★ — §6-1 · §9-1. ★**선언 ≠ 관측**★: 관측된 것은 알림 11 종(L7)뿐이고 서버 요청은 0 건(L11) |
| L15 | ★**PATH 의 `codex` 는 shim 이라 프로그램에서 직접 띄우면 `ENOENT` 다**★(Node `spawn('codex')` 실패). 실측은 실 바이너리 경로로 했다 | ★**우리 코드가 이미 `cmd.exe /c` 로 감싸는 것이 같은 원인의 처리이고, 그 감싸기를 걷으면 이 통로가 죽는다**★(`crates/engram-dashboard-agent/src/backend/mod.rs` 의 `console_command`) |
| L16 | ★**우리가 보내는 요청은 셋이 아니라 넷이다**★ — `initialize` · `thread/start` · `turn/start` · `turn/interrupt`. ADR-0187 본문이 「세 종」으로 적는 것은 **`initialize` 핸드셰이크를 빼고 센 것**이다 | §5-1 의 대기 맵이 다뤄야 하는 왕복 수. 핸드셰이크가 **약 130ms**(L1)라 §4-10 의 큐가 그 창을 진다 |
| L17 | ★**`-a`(`--ask-for-approval`)는 최상위에만 있고 `codex exec` 에는 없다**★ — `codex exec --ask-for-approval` 은 `error: unexpected argument` 로 죽는다. `-s` 는 양쪽에 다 있다 | 오늘 PTY 경로가 `-s workspace-write -a on-request` 를 대화형 `codex` 에 붙이는 것은 **그래서 맞는 조합**이다(`backend/codex/mod.rs:104-107`, 상수 `:46-51`). ★백엔드가 통로마다 다른 인자 집합을 갖는다는 사실 자체가 ADR-0188 의 값 설계 대상이다★ |
| L18 | 로컬 벤더 파일 판독(왕복 없이 생긴 실측) — `state_5.sqlite` 의 Windows 저장형이 **UNC 확장**(`\?\I:\…`)이라 정규화가 필요하다 | 2a 는 이 파일들을 읽지 않는다(ADR-0008). **진단 지식으로만 갖는다** |

### `[스키마]` — codex 가 내보내는 JSON 스키마·생성 TS 를 읽은 것

| # | 스키마가 말하는 것 | 2a 에 미치는 영향 |
|---|---|---|
| S1 | 프로토콜은 JSON-RPC **모양**의 NDJSON 이고 `jsonrpc` 필드가 없다(상류 `rpc.rs` 주석도 그렇게 적고 실 핸드셰이크 응답에도 없었다). 한 줄 = 한 JSON 객체. 요청은 선택적 W3C `trace` 를 싣는다 | 「JSON-RPC 2.0 라이브러리를 꽂으면 된다」가 **아니다.** 봉투를 우리가 정의한다 |
| S2 | **스레드 id 는 서버가 발급한다.** `ThreadStartParams` 27개 속성에 id·threadId·sessionId·name·title 이 하나도 없다 | ADR-0185 결정 1 의 스키마 근거. **claude 패턴(우리가 발급)이 재현 불가** |
| S2b | `Thread.id` 가 **UUIDv7** 이라는 것 — ★★**이 줄은 이제 `[스키마]` 도 `[문서]` 도 아니라 `[실측]` 이다**★★. 1·2 판은 「스키마는 타입만 주고 UUIDv7 은 문서 문장이다」로 `[문서]` 를 붙였고 3판이 그 지시를 그대로 이고 있었는데, ★**실측 정본이 그것을 실측으로 뒷받침한다**★ — 받은 값의 **버전 니블 · RFC4122 variant · 48비트 ms 접두**를 디코드해 확인했다(`.claude/handoff/attachments/codex-measurements-2026-09-09.md:21`·`:51` · 이 표의 L4). **§6-3·§9-1·§11 도 `[실측]` 로 읽는다** — 한 주장에 두 등급을 붙이지 않는다 | `backend_session_id: Option<Uuid>`(`crates/engram-dashboard-agent/src/profile.rs:165`) 의 타입이 이 한 줄에 매달렸는데, ★그 다리가 문서에서 실측으로 바뀌었다★ |
| S3 | **id 는 생성 응답으로 온다** — `thread/start` → `ThreadStartResponse { thread, … }` 이고 `Thread.id` 가 그 값 | §6 끝의 순서 제약이 걸리는 자리 |
| S4 | `Thread` 는 별도 `sessionId` 도 싣는다(같은 세션 트리에 속한 스레드들이 공유하는 id). 문서는 파생하지 말고 그 필드를 읽으라고 적는다 | ★resume 키로 쓰지 말 것 — 아래 「뒤집힌 자리」 HIGH 3★ |
| S5 | 서버→클라 알림 **81종**(실험 opt-in 유무와 동일). ★**조사 보고서는 이 자리에 83 을 적었고 1·2·3 판이 그 숫자를 옮겼는데, 로컬 스키마 생성이 돌려준 선언 수는 `ServerNotification` 81 종이다**★(L14) — **실측이 이긴다**(§2 머리의 우선순위). 델타 넷 = `item/agentMessage/delta` · `item/reasoning/textDelta` · `item/reasoning/summaryTextDelta` · `item/commandExecution/outputDelta`. 완결 블록 = `item/started`/`item/completed` + **20 변형 `ThreadItem` union**. 턴 수명 = `turn/started`/`turn/completed`(`TurnStatus` 네 값 = completed · interrupted · failed · inProgress). `thread/status/changed` 가 `ThreadActiveFlag`(waitingOnApproval · waitingOnUserInput)를 싣는다. 토큰 = `thread/tokenUsage/updated` | §6 번역 표의 왼쪽 열 전부. ★**20 변형의 목록을 이 세션은 읽지 않았다**★ `[미확인]` |
| S6 | 입력: `turn/start`(idle) · `turn/steer`(턴 중, `expectedTurnId` 선행조건) · `turn/interrupt` (threadId + turnId, Esc 등가, **턴 범위** — 스레드는 살고 턴이 interrupted 로 착지). `thread/queue/*` 는 실험 게이트. ★**「셋 다 stable」은 `[문서]` 다**★ — 상류 소스의 stable 표식·주석을 읽은 것이고(F10) 스키마가 내보내는 값이 아니다. ★단 `turn/start`·`turn/interrupt` 는 **실제로 돌았다**★(§2 L5 · L9) | 입력 인코딩이 `Raw` 가 아니게 된다(§5). ★**`control.interrupt` 신고는 새 통로 구현체가 진다**★ — `StdioTransport` 가 그 칸을 false 로 박아 신고하고(`crates/engram-dashboard-agent/src/transport/stdio.rs:363-384`) `interrupt()` 가 `Unsupported` 인 것은 그대로지만, **codex 세션은 그 파일을 안 지난다**(ADR-0189 · §5-5). ★그 정직한 값을 **누가 만들어 주입하나**는 ADR-0191 이 닫았다 — 백엔드 폴더 안의 통로 생성 코드다(§5-5)★ |
| S7 | **shutdown 메서드가 없다.** 정리는 전송 계층 = stdin 닫기 | §4-8. L2 가 그 관측이다 |
| S8 | ★**프로토콜 버전이 없다**★ — `InitializeParams`/`InitializeResponse` 양쪽에 버전 칸이 없다. 협상은 capability 플래그뿐(`experimentalApi` · `optOutNotificationMethods` · `requestAttestation` · 열린 `extensions` 맵). **깨지는 변경은 핸드셰이크에서 못 잡고 런타임에 드러난다** | §4-7 이 그 대신 무엇을 하나 |
| S9 | 영속: `state_5.sqlite` 의 `threads` 표(id PK · rollout_path · created_at · updated_at · cwd NOT NULL · title · name), 내용 정본은 `~/.codex/sessions/<Y>/<M>/<D>/rollout-<ts>-<uuid>.jsonl`. ★**뒤 두 주장은 등급이 갈린다**★ — wire 의 `ThreadListParams.cwd` 가 **정확 일치** 필터라는 것은 `[문서]`(상류 소스 판독)이고, Windows 저장형이 UNC 확장(`\?\I:\…`)이라 정규화가 필요하다는 것은 `[실측]` 이다(이 PC 의 로컬 sqlite 를 열어 본 것 — L4 의 「왕복」 범위와 다른 축) | 2a 는 이 파일들을 읽지 않는다(ADR-0008 「추적 파일로 기능 확장 금지」). **알아 두는 것은 진단 목적뿐** |
| S10 | 스레드 **이름**은 키가 못 된다 — `thread/name/set` 은 생성 후에만 되고, 이름 칸에 UNIQUE 제약이 없으며 중복이 관측됐다 | 이름을 복원 키로 쓰자는 제안이 나오면 이 줄로 닫는다 |

### `[문서]` — 공식 문서·README·상류 소스·이슈를 읽었을 뿐 돌려 본 적 없음

| # | 문서가 말하는 것 | 왜 위험한가 |
|---|---|---|
| D1 | **승인은 서버→클라 요청 다섯**이다 — `item/commandExecution/requestApproval` · `item/fileChange/requestApproval` · `item/permissions/requestApproval` · `item/tool/requestUserInput` · `mcpServer/elicitation/request`. 명령 승인 payload 에 `availableDecisions` 가 실려 **서버가 버튼 목록을 지정한다** | ★**우리 클라이언트도 JSON-RPC 서버여야 한다**★. 답하지 않으면 에이전트가 멈춘다 — 단방향 이벤트 피드로 설계하면 그 자리에서 죽는다 |
| D2 | `deprecationNotice` 알림 메서드가 있고 일부 타입에 인라인 deprecated · UNSTABLE 표식이 있다 | 버전 게이트가 없는 상태(S8)의 유일한 조기 신호 |
| D3 | 0.153.4 README 가 **bounded queue** 와 `-32001` **과부하 응답**을 문서화한다 | 읽기를 멈추면 그 응답이 돌아온다. ★실제로 언제 나오나·큐 깊이는 `[미확인]`★ |
| D4 | 재부착 — `thread/resume` doc-comment 가 「thread_id 가 도는 스레드를 가리키면 app-server 가 그 스레드에 **재합류(rejoin)** 한다」고 적는다. 구독은 연결 단위 집합이라 여러 클라이언트가 붙을 수 있다. 구독자가 0 이 되면 **30분** 유지 후 `thread/status/changed` → notLoaded + `thread/closed` | ★그 30분 시계는 `thread/unsubscribe` 뒤에 걸리고 **stdio EOF 가 아니다**★ |
| D5 | v1 은 동결(레거시 메서드 4개 잔존), 상류 AGENTS.md 가 신규 API 는 v2 라 적는다. 피어들이 날짜 박힌 호환 게이트를 들고 있다(paseo 의 legacy tool-call 호환 주석 + 제거 예정일 2027-01-09, 버전 하한 상수, `tool/requestUserInput` 과 `item/tool/requestUserInput` 이중 핸들러). Vibe Kanban 이 `newConversation` → `thread_start`/`thread_fork` 로 이주했다 | **버전 프로빙은 선택이 아니다.** 그런데 프로토콜 버전이 없어서(S8) 프로빙 대상은 CLI 버전 문자열뿐이다 |
| D6 | Windows 교훈 — paseo 변경기록: 「Codex 가 비-JSON 출력을 내도 데몬이 더는 죽지 않는다 — 지역화된 stdout 라인을 데몬 워커를 쓰러뜨리는 대신 무시한다(#866)」 | ★**모든 stdout 라인이 JSON 이라고 가정하지 말 것**★. §4-6 의 근거 |
| D7 | 이슈 #40766 — Windows 에서 **MCP OAuth 만료가 app-server 를 죽여** rollout 이 잘리고 resume 이 streaming 에서 멈춘다 | §3 EOF 처분의 근거. 「프로세스가 죽는다」가 이론이 아니라 **관측된 시나리오**다. ★그리고 그 처분이 「즉석에서 다시 붙지 않는다」인 이유이기도 하다 — 잘린 rollout 위에 새 턴을 얹으면 그 손상이 우리 것이 된다(ADR-0192)★ |
| D8 | 이슈 #22393 Windows 큐 포화 · #33241 두 app-server 프로세스가 **동시에** 같은 rollout 에 쓰기 핸들을 들어 이력이 병합됨(0.144.2) · #25914 프로세스 경계에서 thread not found / no active turn to steer | 셋을 뭉개지 말 것(아래 정정 B2) |
| D9 | 「실험적」 딱지는 현행 공식 문서에서 **app-server 명령과 WebSocket 전송**을 가리킨다 — ★**stdio 도 그 명령 안에 든다**★. 0.153.4 README 만 WebSocket 으로 좁게 적혀 있다(`.claude/handoff/attachments/codex-app-server-survey.md:66`·`:120`) | **문서 간 불일치이지 stdio 면제가 아니다.** ★**S8(프로토콜 버전 없음)과 곱해서 읽어야 하는 행이다**★ — 실험적 표면인데 **깨지는 변경이 핸드셰이크에서 안 걸리고 런타임에 터진다**(같은 파일 `:53`). ★**그리고 이 위험이 ADR-0187 본문에는 없다**★ — §11 이 그 결말을 진다 |
| D10 | `codex mcp-server` 는 공식적으로 app-server 를 권하며 폐기됐다 | app-server 가 상류가 미는 방향이라는 신호. ★단 그것이 「유일한 경로」는 아니다(HIGH 5)★ |

### `[미확인]` — 어느 등급도 못 붙이는 것

★**1·2 판의 이 목록에서 셋이 걷혔다**★ — 스레드 생성 응답 모양 · 조각이 흐르나 · 비-JSON 줄이 나오나(마지막 것은 「나오지 않았다」로 답이 났고, 그것이 §4-6 을 면제하지는 않는다). 남은 것:

- ★**도구를 실제로 실행하는 턴을 안 돌렸다**★ — 읽기 전용·승인 없음·자잘한 프롬프트로 돌려서 명령 실행이 일어나지 않았다. 그래서 **① 명령 출력이 조각으로 흐르나**(`item/commandExecution/outputDelta`) **② ★승인 요청 11 종이 실제로 오나★** 가 둘 다 미측정이다. ★**§10-6-b ② 가 이것에 매달린다**★.
- **승인 왕복을 해 본 적이 없다** — payload 모양·`availableDecisions` 의 실제 값·응답 봉투 전부 미상. ★관측된 서버 요청이 0 건인 것은 「그런 게 없다」가 아니라 **그 조건에서 안 왔다**는 뜻이다★(L11).
- **cold resume(껐다 켜고 잇기)을 해 본 적이 없다** — `thread/resume` 자체를 안 불렀다. 문서가 지원한다고 적고 적대 리뷰가 「우리 용례는 지원되는 쪽」이라 판정했으나 **우리 관측이 아니다.**
- ★**턴 도중에 stdin 을 닫으면 어떻게 되나**★ — 측정한 것은 「턴 없을 때 닫으면 46ms 에 종료 0」이다(L2). ★§10-10 이 그 미관측 위에서 「안 한다」를 고른 결정이다★.
- ★**재기동 뒤 `thread/resume` 한 서버가 진행 중이던 턴 상태를 다시 알려 주나**★ — ★3판은 여기에 「§3 조정 규칙 4 와 §10-12 의 재관측이 이것 위에 선다」를 달았는데 **그 둘이 없어졌다**★(ADR-0192). 지금 이것을 요구하는 것은 **Phase 2b 의 resume 배선**뿐이다.
- ★**살아 있는 대화의 승인 정책을 바꿀 수 있나**★ — `ClientRequest` 155 종에 그런 메서드가 있는지 안 봤고 claude stream-json 쪽도 안 봤다. **못 바꾸면 「모드 변경 = 다시 띄우기」가 되어 사용자가 체감하는 동작이 달라진다**(ADR-0188 「후속으로 남는 것」 ③).
- ★**claude `--permission-mode` 6 값이 실제로 무엇을 하는지**★ — `acceptEdits`·`auto`·`bypassPermissions`·`manual`·`dontAsk`·`plan` 의 **값별 설명이 도움말에 한 줄도 없다**`[실측]`. ★**ADR-0188 이 값 설계를 후속으로 미룬 이유가 이것이다**★ — 재고 나서 정한다.
- **추론 텍스트를 어떻게 켜나** — 기본 설정에서 `reasoning` 아이템이 빈 내용으로 왔다(L8). 어떤 설정이 `item/reasoning/textDelta` 를 내는지 안 봤다.
- 두 연결이 한 스레드의 **턴을 몰 수 있나**(관측만이 아니라 구동) — 조사 보고서 자신이 미결로 남겼다.
- `-32001` 이 실제로 언제 나오나 · 큐 깊이가 얼마인가.
- `ThreadItem` 20 변형의 목록 — 읽지 않았다. §6 번역 표의 그 행이 비어 있는 이유다. ★**스키마를 로컬에서 생성할 수 있으므로**(L14) 이것은 codex 를 돌릴 필요 없는 판독이다★.
- **`extra_args` 로 같은 플래그를 또 넣으면 뒤 값이 이기나** — 양쪽 백엔드 다 미검증(우리 소스도 그렇게 적어 뒀다 — `backend/claude/mod.rs:574`).
- app-server 경로에 **첫 방문 폴더 신뢰 확인 모달이 있나** — PTY 경로엔 있고(`trd.md` §6-1 실측) 그것이 오늘 LLM 표면을 닫아 둔 유일한 사유다(`crates/engram-dashboard-agent/src/commands.rs:306-315`). ★단 §10-9 는 그 관측을 기다리지 않고 「지금 열지 않는다」로 닫혔다 — 사유가 **권한 축**이라 다른 축이다★.
- 0.153.4 와 우리가 읽은 상류 `main` 사이의 드리프트.
### 조사 보고서가 뒤집힌 자리 — ★다음 독자가 오류를 다시 들이지 않게★

★**보고서 본문은 적대 리뷰를 반영하지 않은 상태다**★(그 파일 자신이 그렇게 적는다). 아래가 그 정정이고 **보고서 본문보다 이쪽이 우선한다.**

| 원 보고서 | 정정 | 출처 |
|---|---|---|
| resume 「4안」(우리 id → codex id 매핑을 새로 만든다) | ★**철회됐다 — 통째로 무시할 것**★. ADR-0185 가 대체했다: 그 매핑은 **이미 있다**(우리 id = `AgentId`, 백엔드 id = `backend_session_id`, 둘의 매핑 = 프로필 레코드 자체). 새로 만들면 이중 출처가 되고 어느 쪽이 정본인지 다음 세션이 못 가린다 | ADR-0185 「거부한 대안」 |
| F6 「프로세스가 다르면 재부착이 **깨진다**」(확실) | **과장이고 셋을 뭉갰다.** ① **cold resume** = 0.153.4 README 가 지원한다고 문서화 ② **동시 소유 거부** = #33241, 두 프로세스가 *동시에* 같은 rollout 에 쓴 사고 ③ **live stdio 재부착 불가** = #25914, 활성 턴 인수인계. ★**우리 용례(껐다 켜고 잇기)는 ①이라 지원되는 쪽이다**★. 순차 재시작 후 resume 실패의 증거는 어디에도 없다. 30분 unload 도 EOF 가 아니라 `thread/unsubscribe` 뒤 | BLOCKER B2 |
| F4 「fork 는 원래 루트의 session id 를 유지한다」 | **틀렸다.** 0.153.4 README 의 `thread/fork` 예시는 새 스레드의 `id` 와 `sessionId` 가 **둘 다 새 루트**다. → ★**resume 키는 `thread.id` 를 쓰고 `sessionId` 로 계보를 추론하지 말 것**★ | HIGH 3 |
| F3 「그 요청이 닫혔으니 클라이언트 지정 id 는 앞으로도 안 온다」 | **과대해석.** upvote 부족으로 닫힌 것은 설계 거부도 로드맵 약속도 아니다. 말할 수 있는 것은 「**0.153.4 에 클라이언트 지정 id 가 없다**」까지 | HIGH 4 |
| F13 「app-server 가 다중 백엔드 호스트의 사실상 유일한 선택」 | **표본 편향이었다** — `codex exec --json` 으로 resume 까지 붙인 프로젝트가 실재하고, OpenAI 비대화형 문서는 스크립트·CI 에 `exec` 를 권한다. ★**그래서 세 후보를 실제로 돌렸고, ADR-0187 이 그 위에서 app-server 를 골랐다**★ — 기각 사유는 「생태계」가 아니라 **실측**이다: `exec --json` 은 글자를 흘리지 않고(도착 이벤트 넷 · 중단 시점까지 두 줄) 공식 SDK 는 존재하지 않는다. ★**즉 이 정정이 요구한 「후보로 올려서 재라」가 실제로 수행됐고 그 결과가 같은 답이었다**★ | HIGH 5 → ★**ADR-0187 이 닫음**★ |
| F13 반대신호 「실험적 딱지는 WebSocket 만 가리킨다」 | **좁게 읽었다.** 현행 공식 문서는 app-server **명령** 과 WebSocket 전송을 함께 가리켜 **stdio 도 포함**된다. 문서 간 불일치이지 stdio 면제가 아니다 | HIGH 6 |
| 수명·복구 명세 | **통째로 빠져 있었다.** 프로세스·연결·스레드·턴 **각각의 상태기계**를 정의하고, EOF 시 대기 중 RPC 를 실패시키고 활성 턴을 미확정으로 표시한 뒤 **재시도 전에 조정**할 것. 로그인·MCP·서브에이전트 복구도 명세 | HIGH 7 → §3. ★**단 그 요구의 마지막 조각(「재시도 전에 조정」)은 이 저장소에서 성립하지 않아 기각됐다**★ — EOF 는 세션을 **끝내고 수거**하므로 조정할 세션이 안 남는다(ADR-0192 · §3 「왜 걷었나」). **앞 조각 둘(상태기계 분리 · 대기 RPC 오류로 깨우기)은 그대로 채택됐다** |
| 와이어 동시성 | **빠져 있었다.** 구체 실패 = **우리와 서버가 동시에 request id `0` 을 보내는데 대기표가 하나면 승인 응답이 엉뚱한 데로 간다.** 읽기는 항상 비우고, 하류 큐는 유계로, stdin 쓰기는 하나로 직렬화, **인바운드/아웃바운드 id 공간 분리**, 승인 생명주기와 재시도 규칙 명시 | HIGH 8 → §4 |

**리뷰가 반박하지 않은 것(그대로 유효):** Windows stdio 기동 · 구조화 이벤트 목록 · 승인이 서버→클라 요청이라는 것 · `turn/start`/`turn/steer`/`turn/interrupt` 가 stable 이라는 것 · 프로토콜 버전 부재 · shutdown 메서드 부재 · `deprecationNotice` 존재 · paseo 의 비-JSON 라인 교훈.

★**ADR-0185 의 근거 한 축이 HIGH 4 와 부딪힌다 — 여기 적어 둔다**★. 그 ADR 「근거」 첫 항목은 「『아직 없다』가 아니라 **『넣지 않기로 했다』**라서 기다리는 선택지가 없다」로 적혀 있다. HIGH 4 는 그 독법을 과대해석으로 판정한다. **결론은 안 바뀐다 — ★단 「스키마 하나만으로 선다」로 줄여 적지 말 것★.** 그 ADR 의 「근거」 첫 항목은 **다리 둘**을 함께 적는다: 「**스키마 + 상류 거절**」 — `ThreadStartParams` 에 id 계열 속성 0개(codex 자신이 내보내는 JSON 스키마) **+** 그 파라미터를 넣자는 issue 가 메인테이너 판정으로 닫힘(openai/codex#15767). HIGH 4 가 때리는 것은 **둘째 다리에 얹힌 세기**(「『아직 없다』가 아니라 『넣지 않기로 했다』라서 기다리는 선택지가 없다」)이지 **그 다리의 존재가 아니다.** 그래서 결정 1 은 여전히 두 다리 위에 서고, 약해지는 것은 그 둘째를 「상류가 영원히 안 준다」로 읽는 대목뿐이다. ★**이 문서는 §9-1 에서도 두-다리 형태를 인용한다**★(「결정 1은 스키마·상류 거절 위에 서므로 그대로다」 — ADR-0185 「미검」) — 두 자리를 갈라 적지 말 것. 그리고 상류가 언젠가 클라이언트 지정 id 를 준다면 그것은 결정 1 을 뒤집는 게 아니라 **claude 와 같은 갈래가 하나 더 열리는 것**이다 — 결정 1 이 이미 「발급 주체는 백엔드가 정한다」라 그 경우를 품는다. ★그러니 그 근거를 「상류가 영원히 안 준다」로 인용하지 말 것★. 이 문서는 그 예측을 지지하지 않는다.

---

## 3. 네 상태기계 — 프로세스 · 연결 · 스레드 · 턴

★**하나로 합치지 말 것**★ — 적대 리뷰 HIGH 7 이 요구한 것이 정확히 이 분리다. 합치면 「프로세스가 살아 있다」가 「턴이 진행 중이다」와 같은 뜻이 되고, 그 둘이 갈리는 순간(#40766: 프로세스가 죽고 rollout 이 잘렸는데 저장된 스레드는 streaming 으로 남는다`[문서]`)에 어느 상태를 믿어야 할지 알 수 없어진다.

★**넷 다 오늘 우리 세 층(session · core · transport)의 소유권 분할 위에 얹힌다 — 그 분할을 바꾸지 않는다**★(CLAUDE.md 「핵심 불변식」 소유권 분할: transport = master/writer/child/shutdown/job · core = subscribers/replay/seq/status/finalized · session = id/cwd/epoch/cols/rows).

★**그리고 넷 중 셋의 소유자에게 이제 이름이 있다 — `backend/codex/` 안의 `AgentTransport` 구현체다**★(ADR-0189 · 그것을 **만들어 조립점에 넘기는 자리**는 ADR-0191 · §5). 1·2 판은 이 자리를 「codex 세션 드라이버」라는 **미확정 이름**으로 부르며 그 거처를 §10-1 에 매달아 두었는데, 그 항목이 닫혔다. ★**그것이 `StdioTransport` 가 아니라는 것은 그대로다**★ — 그 파일 헤더의 「transport 층은 stdout 바이트의 스키마를 모른다」는 **그 범용 파이프 구현체 자신**에 대한 서술이고 지금도 참이다(`crates/engram-dashboard-agent/src/transport/stdio.rs:7-9` · 그 재독의 근거 = §5-4). **아래 표에서 「codex 통로 구현체」는 전부 그 struct 하나를 가리킨다.**

★**기계는 넷이고 표는 다섯 줄이다 — 스레드가 직교하는 두 축을 갖기 때문이다**★. 옛 초안은 그 축 둘을 한 줄로 늘여 적었는데(`생성됨 → persist 됨 → 구독 중 → …`) **지속성과 구독은 서로를 함의하지 않는다** — persist 됐지만 연결이 끊긴 상태, cold resume(신원은 살아 있고 런타임은 0), 재구독이 그 선형 나열에서 표현되지 않았다. 그래서 축으로 갈랐고, **소유자도 그 축에서 갈린다.**

| 상태기계 | 상태 | 누가 소유 | EOF · 크래시가 무엇을 하나 | 관측 등급 |
|---|---|---|---|---|
| **프로세스** | 미기동 → 기동 → 살아있음 → 종료(관측) / 종료(우리 kill) | **통로 구현체** (child + Job Object). ★그 둘을 갖는 것이 오늘 `StdioTransport` 와 같은 모양이다★(`transport/stdio.rs:39` child · `:55` job_handle) | 파이프는 자식(및 자식 트리)이 write 핸들을 모두 닫으면 read 가 EOF 로 깬다 — 자연 종료든 kill 이든 같은 경로다(그 파일 헤더 `:11-14`). 읽기 스레드가 깨고 `core.finish` 가 terminal 전이를 **1회** 낸다(ADR-0005) | 기동·stdin 닫힘·**exit 0 을 46ms 에** = `[실측]`. **비정상 죽음은 `[미확인]`** |
| **연결** | 미초기화 → `initialize` 왕복 중 → 준비됨 → 끊김 | **codex 통로 구현체** — 그 안의 대기 맵 + 아웃바운드 id 카운터(§5-1) | ★**대기 중인 모든 RPC 를 오류로 깨운다**★. 영구 hang 을 남기지 않는다 — 그 사고의 선례가 이 저장소에 있다(겹친 `request_id` 가 옛 대기자를 영구 hang 시킨 GUI 실측 2026-08-18, 회귀망 = `cargo test -p engram-dashboard --test daemon_client_pending`) | `initialize` 왕복 **약 130ms** = `[실측]`. **끊김 처리는 우리가 짜는 것** |
| **스레드 — 신원 축** | 없음 → 생성 요청 중 → id 수령 → **persist 됨** | **프로필 레코드**(`backend_session_id`, `crates/engram-dashboard-agent/src/profile.rs:165`) — ★**세션 경로에서 화신을 넘겨 사는 유일한 그릇이다**★(바로 아래 절이 그 전수 열거). ★기록하는 손은 조립점이 준 콜백이고 backend 가 레지스트리를 직접 부르지 않는다★(§5-6) | 프로세스가 죽어도 **신원은 디스크에 남는다**. 그것이 우연이 아니라 명시 설계다 — 런타임 종료는 세션만 맵에서 거두고 프로필은 시체로 보존하며(`crates/engram-dashboard-agent/src/reaper.rs:104-106`), 항목 삭제의 트리거는 **명시 delete 동사 하나뿐**이다(`profile.rs:479` ← `crates/engram-dashboard-agent/src/manager.rs:690-692` — TTL·크래시 정리·셧다운 스윕 전부 없고 `impl Drop` 도 없다) | ★**id 수령까지 `[실측]` 로 바뀌었다**★ — `thread/start` 응답의 `result.thread.id` 를 실제로 받았고 `thread.id == thread.sessionId`, **UUIDv7** 이라 `Option<Uuid>` 칸에 그대로 든다. **`thread/resume` 은 여전히 `[미확인]`**(§11) |
| **스레드 — 런타임 축** | 미부착 → 구독 중 → 미부착(연결이 끊겨서) | **codex 통로 구현체** — 그 안의 `thread.id` + 준비 상태 | 연결이 끊기면 이 축이 0 으로 돌아가고 **이 화신에서는 다시 안 붙는다**(ADR-0192 — 아래 EOF 절). **신원 축은 건드리지 않는다** — 그것이 두 축을 가르는 실익이다(cold resume = 신원은 디스크에서 살아 오고 런타임은 **다음 화신**이 0 에서 다시 붙는 것) | 첫 부착(스레드를 만든 그 연결이 알림을 받는다) = `[실측]`. ★**다음 화신의 재부착(`thread/resume`)은 `[미확인]`**★ |
| **턴** | idle → 진행 중(turnId) → completed / interrupted / failed / **미확정** | ★**상태기계는 통로 구현체 것이다**★. 코어가 갖는 것은 그 기계가 아니라 **사실 셋**뿐이다 — `{epoch, in_turn, last_signal}`(`crates/engram-dashboard-agent/src/turn.rs:47-64`). 신호는 `turn_classifier` 가 `TurnSignal`(`Progress`/`Ended` 둘)로 내고(`crates/engram-dashboard-agent/src/backend/mod.rs:143` · ADR-0113) 코어는 그것만 소비한다 | ★**활성 턴은 completed 로 낙관하지 않고 failed 로 단정하지도 않는다 — 미확정으로 남긴다**★. ★단 그 값의 소비자는 **로그 한 줄뿐**이다★ — 게이트가 아니고 화신을 넘지도 않는다(아래 두 절) | `completed`·`interrupted` 착지 = `[실측]`(★턴 id 는 `result.turn.id` 이고 `result.turnId` 가 아니다★). `failed` 는 `[스키마]` |

### ★우리 기계에서 뺀 세 상태 — 「구독 해제 → notLoaded → closed」★

옛 초안의 스레드 기계는 그 셋을 끝에 달고 있었다. **뺐다.** 셋 다 `thread/unsubscribe` **뒤에** 걸리는 전이인데(정정 B2 — 30분 시계는 stdio EOF 가 아니라 그 호출 뒤다), ★**우리가 그 메서드를 부르는 자리도, 부를 사유도 이 문서 어디에도 없다**★. 조사 보고서도 그 호출자를 적지 않는다`[미확인]`.

- **그래서 셋은 「서버 쪽 상태」로만 남긴다** — 관측하면 로그에 남기고(§4-7 의 2 와 같은 취급), 우리 기계의 전이로 쓰지 않는다.
- ★**근거 없는 상태를 기계에 남기면 구현자가 트리거를 발명한다**★ — 그것이 정확히 이 문서가 §3 을 넷으로 가른 이유(합치면 어느 상태를 믿을지 알 수 없어진다)의 반대편 실패다.
- 우리 용례에서 스레드를 놓는 계기는 **프로세스 종료(= 연결 EOF)** 하나이고, 그때 움직이는 것은 위 표의 **런타임 축**뿐이다.

★**「미확정」은 codex 어휘에 없다 — 우리가 더하는 상태다**★. `TurnStatus` 는 네 값뿐이고(S5) 그중 어느 것도 「모른다」가 아니다. 그래서 이것을 codex 값의 번역으로 적으면 안 되고, **우리 쪽 관측 실패 상태**로 세워야 한다. 오늘 이 저장소에 그 자리에 가장 가까운 것은 `resume_failure_kind` 의 「None = 이 텍스트만으로는 종류를 단정할 수 없다」(`crates/engram-dashboard-agent/src/backend/mod.rs:147-164` · ADR-0172) — **fail-open 으로 모름을 표현하는 같은 규율**이다.

### ★그 「미확정」이 사는 곳 — 통로 구현체의 상태이고 코어의 사실 계층이 아니다★

**어느 상태가 누구 어휘이고 어디 사나:**

| 상태 | 누구 어휘인가 | 어디 사나 |
|---|---|---|
| `completed` · `interrupted` · `failed` · `inProgress` | **codex** (`TurnStatus`) | wire 에서 받아 통로 구현체가 읽는다 |
| `idle` | 우리 — codex 는 「턴 없음」을 상태 값으로 주지 않는다(`turn/started` 이전이 그것이다) | **통로 구현체** |
| **미확정** | ★우리★ — 관측 실패 | ★**통로 구현체**★ |

★**세션마다 하나씩 있는 객체라 이 넷이 실제로 놓일 자리가 있다**★ — 1·2 판이 이 상태들을 「드라이버」라는 이름으로만 부르고 그 거처를 미결로 남겼던 것이 ADR-0189 가 닫은 갭이다.

★**코어의 사실 계층에 상태를 더하지 않는다 — 더할 자리가 없다**★. 그 계층이 갖는 것은 `{epoch, in_turn, last_signal}` 셋뿐이고(`crates/engram-dashboard-agent/src/turn.rs:47-64` — 신호 어휘도 `Progress`/`Ended` 둘뿐), **거기에 「미확정」을 넣으면 그것을 지우는 세 번째 호출자가 필요해진다.** ADR-0127 결정 5 가 그 자리를 못 박아 두었다: 지우는 호출 지점은 **의도적으로 둘**(`OutputCore::finish` + `emit` 의 finalize 재확인)이고 「세 번째 호출자를 늘리면 인과가 갈라진다」. 그 둘이 **같은 표 뮤텍스**를 타는 것이 종료 후 지각 삽입과의 경쟁을 닫는 논증 자체다.

- ★**EOF 시 코어의 삭제는 그대로 둔다**★ — `core.finish` 가 `turn.table.forget(self.id, self.epoch)` 로 자기 항목을 지운다(`crates/engram-dashboard-agent/src/output_core.rs:327`, finalize 승자 경로라 정확히 1회 · 다른 지점 하나는 `:251` 의 emit 재확인). **이 문서는 그 두 지점을 늘리지도 옮기지도 않는다.**
- 그래서 EOF 이후 코어의 답은 「미관측」이 되고, **「미확정」은 통로 구현체가 죽기 전에 자기 안에서 마지막으로 적는 값이다.** 둘은 같은 뜻이 아니다 — 코어의 미관측은 소비자가 「즉시 배달」로 흡수하는 fail-open 이고(`turn.rs` 모듈 헤더), 구현체의 미확정은 **「이 턴의 결말을 못 봤다」는 관측 실패의 이름**이다. ★**그리고 그것이 하는 일은 로그 한 줄이 전부다**★ — EOF 뒤에 그 값을 읽고 행동할 다음 턴이 이 화신에 없기 때문이다(ADR-0192 · 아래 EOF 절).
- ★**만약 구현에서 코어 변경이 정말 필요해진다면 그것은 이 문서가 고를 일이 아니다**★ — ADR-0127 재론이고 `[미확인]` 이다. 지금 판단으로는 **필요 없다**(위 표의 세 상태가 전부 통로 구현체 안에서 성립한다). 그 판단을 뒤집을 조건 하나 = 코어 밖에서는 볼 수 없는 소비자(우편 파킹 idle 게이트 등)가 「미확정」과 「미관측」을 **구별해야** 한다는 것이 실측으로 드러나는 것. 그때 §10 에 행을 세운다.
- ★**§4-10 의 입력 큐는 이 계층에 **아무것도 더하지 않는다** — 그러나 「읽기만 한다」도 절반만 맞다**★. **claude json** 의 해제 조건만 이 계층을 읽고(소비자가 하나 늘 뿐 — ADR-0127 저촉 없음), ★**codex 는 이 계층을 아예 안 읽는다**★ — 코어의 답이 codex 에서는 fail-open 이라 자기 턴 기계를 읽는다(ADR-0193 · §4-10 규율 4). **어느 쪽이든 지우는 호출 지점은 그대로 둘이다.**

### 그 「미확정」이 프로세스를 넘겨 사나 — ★나르지 않는다. 그리고 **나를 곳도 없어졌다**★

**통로 구현체 안에 사는 상태는 그 구현체와 함께 죽는다.** ★**3판은 이 절을 「조정 규칙 4·5 가 재기동 후에 그 값을 읽어야 하니 그릇을 찾자」로 열었는데, 그 규칙들이 없어졌다**★(ADR-0192 — 예상 못 한 EOF 뒤에 같은 통로가 다시 붙는 설계 자체를 걷었다). 즉 지금은 **읽을 소비자가 아예 없어서** 나르지 않는 것이고, 아래 두 표는 「그때도 나를 수 없었다」는 **두 번째 이유**로 남는다 — 판정이 두 다리 위에 서므로 다음 세션이 한쪽만 보고 되꺼내지 않게 남긴다.

**먼저 화신을 넘겨 사는 그릇이 애초에 있었나 — 세션 경로 전수:**

| 후보 | 수명 | 화신을 넘기나 | 복구 상태를 담을 수 있나 |
|---|---|---|---|
| **`ProfileRegistry` + `AgentProfile` 항목** | 데몬 프로세스 / **디스크 파일**. `impl Drop` 없음 · 항목 삭제 트리거 = 명시 delete 동사 하나(`profile.rs:479` ← `manager.rs:690-692`) | ★**예**★ | ★**예 — 유일한 자리다.** 단 아래 「그런데 칸이 없다」 참조★ |
| `TurnObservations`(사실 표) | 데몬 프로세스(`manager.rs:417`). 항목은 다음 `register` 가 무조건 갈아치우거나(`crates/engram-dashboard-agent/src/turn.rs:125-133`) `forget` 이 지운다 | 표는 예, **항목은 아니오** | ★아니오★ — serde derive 가 **하나도 없다**. 그리고 그 포트는 「읽기 전용이 계약」이라 「지워라/표시해라」 추가를 명시로 금한다(`crates/engram-dashboard-messaging/src/busy.rs:86`) |
| `SessionTracker` | 데몬 프로세스. 폴링 스레드 하나 | 예 | 아니오 — 배운 `resolved_pid` 가 화신과 함께 죽고, `unwatch` 의 유일한 호출자가 `kill_agent`(`manager.rs:1778`)라 자연사한 항목은 **데몬 수명 동안 남는다**(reaper 는 tracker 를 모른다, `manager.rs:1777`) |
| `BusyPolicy.stale` 원장 | 데몬 프로세스 | 예 | 아니오 — 스윕마다 통째로 교체된다 |
| `AgentSession` · `OutputCore` · **통로 구현체** · decoder · `intent` 셀 · 제어 채널 토큰 | **화신 하나** | 아니오 | 아니오 — `output_core.rs` 에 serde derive 가 하나도 없고, decoder 는 「epoch 교체 = 새 transport = 새 decoder 라 리셋이 자동」이 설계다(`crates/engram-dashboard-agent/src/transport/mod.rs:28-29`) |

★**결론: 「이 턴이 미확정으로 끝났다」를 재기동 뒤까지 나르려면 자리는 `AgentProfile` 이거나 없다.**★

**그런데 그 그릇에 그 칸이 없다 — 그리고 옆칸들이 왜 없는지를 설명한다:**

- 화신 스코프 필드 둘은 **serde 속성으로 디스크에 안 닿게** 되어 있다 — `epoch` 은 `skip_deserializing` + 항상 `0` 을 써 내보내고(`profile.rs:190-191`), `last_failure` 는 `#[serde(skip)]` 이라 데몬과 함께 죽는다(`:207-208`). 즉 「이 관측이 어느 화신 것이냐」를 **디스크가 표현할 수 없다**(그 비대칭은 의도이고 CLAUDE.md 「화신 표식」이 그 범위를 `agents.json` 디스크 serde 로 한정한다).
- 예약만 돼 있고 ★**초기화·wire 미러 외에 값을 바꾸는 생산 지점이 0** 인★ 칸 넷 = `restart_policy` · `restart_count` · `failed_reason` · `last_start_at`(선언 `profile.rs:213-230` · 초기화 `:256-261` · wire 미러 `crates/engram-dashboard-daemon/src/connection_core.rs:492-498` — 실측). ★그 넷을 「빈 칸이니 쓰면 된다」로 읽지 말 것★ — 제거가 명시로 금지된 예약 칸이고(`profile.rs:79-84`), ★**거기에 뜻을 새로 부여하는 것은 디스크 계약 변경이면서 동시에 wire 계약 변경이다**★ — 그 주석이 세는 다섯 층을 그대로 통과한다.

★**판정 = 「나르지 않는다」**★ — 첫째 다리는 **소비자 부재**(ADR-0192 뒤로 읽을 코드가 없다), 둘째 다리는 **persistence 쪽 위험**이다. 둘째의 값이 아래 셋이다:

| 무엇이 바뀌어야 하나 | 무엇을 위험에 넣나 |
|---|---|
| `AgentProfile` 에 칸 하나 추가 + 그 칸의 serde 기본값 | `agents.json` 은 `schema_version: 1` 이고 **버전 불일치는 백업 없이 빈 목록으로 떨어진다**(`crates/engram-dashboard-agent/src/persistence/mod.rs:122-129` — 파싱 오류 경로만 `.corrupt-<ms>` 로 rename 한다). 즉 스키마를 올리면 그 순간 **보호되지 않는 데이터 손실 경로**를 지난다 |
| 그 관측이 **어느 화신 것이냐**를 함께 적을 것 | 위 대로 `epoch` 이 디스크에 `0` 으로만 나가므로 **화신 귀속을 적을 어휘가 디스크에 없다.** 새 축을 만들면 ADR-0163 의 「읽기를 건너뛰는 것이 의미의 일부」와 정면으로 부딪힌다 |
| 쓰는 시점 | 프로필 저장은 **뮤텍스 안에서 파일 전체를 동기 재작성**한다(`profile.rs:395` · `:406`). 그리고 ★**저장 실패는 `tracing::error!` 로 삼켜져 호출자·UI·LLM 에 아무것도 안 간다**★(`persistence/mod.rs:99-106`, 계약 = `profile.rs:310-311`). 즉 「미확정을 적었다」를 **확인할 수 없다** |

★**그래서 2a 는 나르지 않는다**★ — 미확정은 통로 구현체 안에만 살고, 그 구현체가 죽으면 로그 한 줄만 남는다. **잃었을 때의 결말이 무엇인지도 이제 짧다**: 그 화신은 끝났고(EOF = 세션 종료 1회 + 수거), 다음 화신은 **저장된 `thread.id` 로 새로 붙는 것**이지 잃어버린 턴 표식을 이어받는 것이 아니다. ★**그 새 부착(`thread/resume`)과 그 뒤 서버가 진행 중이던 턴을 다시 알려 주나는 둘 다 `[미확인]` 이고, 둘 다 Phase 2b 의 resume 배선이 지는 질문이다**★(§0 · §9-1 · §11) — 2a 는 그 위에 아무 설계도 얹지 않는다. → §10-12.

### EOF 를 관측했을 때 — ★같은 통로가 다시 붙지 않는다★ (ADR-0192)

★**3판은 여기에 「조정(reconcile) 규칙 0~5」를 두고 그 넷째 규칙이 「새 프로세스를 띄우고 `initialize` → `thread/resume`」 하게 했다. 그 설계를 통째로 걷는다**★ — 그 규칙이 서 있던 전제(예상 못 한 EOF 뒤에 **같은 세션이 살아남아** 재부착한다)가 이 저장소의 수명 모델과 양립하지 않기 때문이다. 사유는 아래 「왜 걷었나」.

**그래서 EOF 에서 실제로 하는 일은 넷이고, 그중 새로 짜는 것은 둘(1·2)뿐이다:**

1. **대기 중 모든 RPC 를 오류로 깨운다.** 하나도 남기지 않는다. ★**새로 짜는 것 첫째**★ — 영구 hang 을 남기지 않는다(선례 = 겹친 `request_id` 가 옛 대기자를 영구 hang 시킨 GUI 실측 2026-08-18, 회귀망 `cargo test -p engram-dashboard --test daemon_client_pending`).
2. **진행 중이던 턴은 미확정으로 로그에 남긴다.** ★**새로 짜는 것 둘째 — 그리고 이것이 전부다**★. 결과를 추측하지 않는다. ★**게이트가 아니다**★ — 이 화신에 다음 턴이 없으므로 읽을 소비자가 없고, 값어치는 사후 진단 하나다(위 절).
3. **그 뒤는 오늘 인과 그대로다** — pump break → `core.finish` → **terminal 전이 정확히 1회**(ADR-0005) → reaper 가 세션을 맵에서 거두고 프로필은 시체로 보존한다(`crates/engram-dashboard-agent/src/reaper.rs:46-71` · `:104-113`). **이 문서는 그 경로에 한 줄도 더하지 않는다.**
4. **재기동은 새 화신뿐이다.** 그 화신이 저장된 `backend_session_id` 로 `thread/resume` 하는 배선은 **Phase 2b** 이고(§0), 2a 는 그 값을 만들어 두는 데까지다.

★**「모르겠으니 새 대화를 파자」는 그대로 금지다**★ — ADR-0082 가 fresh fallback 을 폐지했고 그 금지는 회귀 테스트가 진다(`crates/engram-dashboard-agent/tests/activation.rs` 의 `activate_resume_early_exit_ends_failed_no_fresh_fallback`). 이 금지는 재연결이 없어져도 죽지 않는다 — 거는 자리가 **다음 화신의 활성화**이기 때문이다.

#### ★왜 걷었나 — 세 불변식과 정면으로 부딪혔다★

★**되꺼내지 말라고 여기 남긴다.** 「EOF 뒤 같은 통로가 `thread/resume` 한다」를 글자대로 구현하면 셋이 동시에 깨진다:★

- **ADR-0005(finalize 1회)** — pump 가 EOF 에서 이미 `core.finish` 를 내 terminal 전이를 1회 냈다(`crates/engram-dashboard-agent/src/output_core.rs:299-341`). 그 뒤에 통로가 자식을 다시 띄우면 그 세션은 **이미 끝난 것으로 신고된 채** 살아 있다.
- **ADR-0019(reaper 단일 소비자)** — `finish` → done_tx → reaper 가 그 세션을 **맵에서 지운다**(`reaper.rs:46-71`). 재부착이 성공해도 붙을 세션이 명부에 없다.
- **Job Object 경계** — 새 자식은 **수거된 세션의 Job Object 밖**에 남는다. 즉 아무도 그것을 죽일 수 없다.

**그리고 그 설계엔 소유자도 없었다.** 「새 프로세스를 띄운다」의 실물은 `activate_profile`(`crates/engram-dashboard-agent/src/manager.rs:1113`)이고 그것은 통로가 부를 수 있는 동사가 아니다. 예약 칸 `restart_policy` 는 **생산 소비자가 0** 이라(위 「그 「미확정」이 프로세스를 넘겨 사나」 절의 예약 칸 넷) 그쪽에도 자리가 없었다.

★**codex 쪽 근거도 같은 방향이다**★ — #40766 `[문서]`: Windows 에서 MCP OAuth 만료가 app-server 를 죽여 rollout 이 잘리고 resume 이 streaming 에서 멈춘다. **잘린 rollout 위에 즉석에서 새 턴을 얹으면 그 손상이 우리 것이 된다.** #33241 `[문서]`: 두 프로세스가 **동시에** 같은 rollout 을 쥐면 이력이 병합된다 — 「죽은 것 같으니 하나 더 띄운다」가 정확히 그 사고 모양이다. ★**claude 도 프로세스가 죽으면 스스로 다시 붙지 않는다 — 백엔드 기본값을 claude 와 맞춘다는 것이 ADR-0192 다.**★

#### ★「우리 kill 이냐」는 여전히 갈라야 한다 — 단 **계약은 안 늘린다**★

EOF 를 `Killed` 로 접을지 `Exited{code}` 로 접을지는 오늘도 갈리고 있고, codex 통로도 같은 구별이 필요하다(우리가 죽인 EOF 에 오류 로그를 쏟지 않기 위해). ★**그런데 그 구별에 공용 계약은 필요 없다 — 묻는 쪽과 답하는 쪽이 같은 객체이기 때문이다.**★

- 그 비트는 **통로 사유물**이다 — `StdioTransport.shutdown: Arc<AtomicBool>`(`crates/engram-dashboard-agent/src/transport/stdio.rs:46`)를 `shutdown()` 이 `Release` 로 세우고(`:340`) **그 구현체 자신의 pump** 가 `Acquire` 로 읽어 분류한다(`:251` · `:271`). codex 통로 구현체도 **자기 읽기 스레드를 자기가 소유**하므로(§5-1) 같은 모양을 그대로 쓴다 — 자기 사유 필드를 자기가 읽는다.
- ★**그래서 `AgentTransport` 는 여섯 메서드 그대로다**★ — ADR-0189 의 「공용 계약에 한 줄도 더하지 않는다」가 지켜지고, 3판이 그 문장과의 긴장을 적어 둔 자리는 **없어졌다**(긴장 자체가 사라졌다).

★**3판이 여기 세웠던 「읽기 함수를 하나 더한다」 판정은 철회한다 — 그 근거 문장이 거짓이었다**★:

| 3판이 적은 것 | 실제 |
|---|---|
| 「세 구현체 전부가 **이미 그 비트를 갖고 있다**(`stdio.rs:46` · pty · api)」 | ★**`ApiTransport` 는 필드가 하나도 없다**★ — `pub struct ApiTransport;`(`crates/engram-dashboard-agent/src/transport/api.rs:14`), `fn shutdown(&self) {}`(`:50`). 그 구현체는 그 비트를 **갖고 있지 않고**, 계약에 함수가 생기면 거기서 거짓말을 하거나 `Unsupported` 를 늘려야 한다 — ADR-0189 가 기각한 바로 그 모양이다 |
| 「조정 규칙 4 가 돌 때 물을 수 없으면 죽는 길에 새 프로세스를 띄운다」 | ★**그 규칙이 없어졌다**★(위) — 물어서 막을 사고가 남아 있지 않다 |

★그리고 3판이 적은 나머지 사실들은 그대로 참이고, 이제 **아무것도 요구하지 않는다**★ — 위로 새는 분류가 사후·손실적이라는 것(`TerminalReason::Killed`/`Exited{code}` → `AgentStatus::Killed` 한 값으로 붕괴, `output_core.rs:306-315`), `TerminationIntent` 에 getter 가 없다는 것(`crates/engram-dashboard-agent/src/types.rs:96-101` · setter `session.rs:111-113` · 유일 독자 `manager.rs:1284-1288` · `reaper.rs:105-111`). ★**이 셋을 「그러니 계약을 넓혀야 한다」의 근거로 다시 쓰지 말 것**★ — 구별이 필요한 층이 자기 필드를 갖고 있으므로 공용 계약이 그것을 나를 이유가 없다. → §10-13.

### 이 넷과 우리 화신 표식(epoch)의 관계 — ★같은 축이 아니다★

화신 표식은 **우리 쪽 재부착 축**이다(ADR-0163/0164): 화신마다 새로 뽑는 32비트 난수이고 비교는 일치/불일치만, 재부착 계기는 권위 명부 관측 단독이며 구독 effect deps 는 `[viewId, agentId]` 다(표식을 넣지 않는다). codex 스레드 id 는 **백엔드 쪽 축**이고 프로세스를 넘겨 산다. ★둘을 섞지 말 것★ — 표식이 바뀌었다고 스레드가 바뀐 것이 아니고(재기동 후 같은 스레드를 잇는 것이 우리 목표다), 스레드가 그대로라고 화면 스트림이 이어지는 것도 아니다.

### 로그인 · MCP · 서브에이전트 복구

HIGH 7 이 셋을 함께 요구했는데 ★**이 문서는 그 셋을 설계하지 않는다**★ — 근거: MCP 주입 자체가 2a 범위 밖(§0)이고, 로그인 만료의 관측(#40766)은 `[문서]`이며 서브에이전트는 우리 스키마 판독 범위 밖(`ThreadItem` 20 변형 미판독 — §2 `[미확인]`)이다. **대신 셋 다 위 EOF 처분 하나로 흡수된다** — 어느 사유로 죽었든 우리가 하는 일은 같다(대기 RPC 오류로 깨우기 → 턴 미확정 로그 → 세션 종료·수거 → 재기동은 새 화신). ★**그리고 ADR-0192 뒤로는 그 흡수가 더 강해졌다**★ — 사유별로 갈릴 여지가 있던 자리(「어느 사유면 다시 붙나」)가 통째로 없어졌다. ★단 그것은 「사유를 구별할 필요가 없다」가 아니라 「지금은 구별할 근거가 없다」다★ — 구별이 필요해지면 `resume_failure_kind`(ADR-0172)가 그 자리다. §11 에 남긴다.

---

## 4. wire 규율

★**아래 열 중 앞 아홉은 전부 적대 리뷰 HIGH 7·8 이 요구한 것이고, 하나도 선택 사항이 아니다**★ — 열째(§4-10, 입력 큐)는 그 뒤에 **ADR-0190** 이 더한 것이다. ★**「어디에 둘지」는 더 이상 §5 에 매달려 있지 않다**★ — ADR-0189 가 그 답을 닫았고, 아래 열 중 쓰기·읽기·상관에 닿는 것은 전부 **그 통로 구현체 안**이다(§5-1). ★**개수를 세는 다른 자리를 만들지 말 것**★ — 이 줄의 수는 절 제목(`### 4-N`)이 정본이다.

### 4-1. stdin writer 는 정확히 하나 — 직렬화한다

한 줄 = 한 JSON 객체(S1)인데 writer 가 둘이면 **한 줄이 쪼개져 끼어든다.** 그 조합에서 서버가 받는 것은 두 개의 깨진 객체다.

- `AgentTransport` 는 `Send + Sync` 이고 `send_input(&self, …)` 이 불변 참조라 **여러 스레드에서 동시에 불릴 수 있다**(`crates/engram-dashboard-agent/src/transport/mod.rs:41-45`). 즉 직렬화는 **구현체 안의 뮤텍스**가 진다.
- 오늘 그 모양이 이미 있다 — `StdioTransport { stdin: Mutex<Option<ChildStdin>>, … }`(`transport/stdio.rs:40`).
- ★**승인 응답이 이 규율의 실제 시험대다**★ — 승인은 서버가 먼저 물어 우리가 답하는 것이라(D1) 그 쓰기는 **사용자 입력과 다른 스레드에서** 온다. 두 경로가 같은 뮤텍스를 통과해야 한다.

### 4-2. 읽기는 항상 비운다

소비자가 느려도 read 를 멈추지 않는다. 근거 = bounded queue + `-32001`(D3) 과 Windows 큐 포화(#22393 `[문서]`) — **읽기를 멈추면 서버 쪽 큐가 차서 과부하 응답으로 돌아온다.**

★**이 규율이 §5 의 선택을 제약한다**★ — 오늘 pump 스레드가 decoder 를 `&mut` 로 **배타 소유**한다(`transport/mod.rs:26-29`: 「decoder 는 …가변 상태를 들고, pump 스레드(단일)가 `&mut` 로 배타 소유한다」). 응답 대기표를 그 스레드가 들고 기다리면 **기다리는 동안 읽기가 멈춘다** = 이 규율 위반. 그래서 대기표는 **pump 밖**에 있어야 한다.

★**그리고 그 규율에는 두 번째 얼굴이 있다 — 읽기 스레드는 stdin 락을 blocking 으로 잡지 않는다**★

읽기 스레드가 **쓰기도 해야 하는** 순간이 실제로 있다: 인바운드 요청에 응답하는 것(§5-2 첫 행 · §6-2 의 모르는-요청 오류 응답 · 나중의 승인 응답). 그런데 모든 쓰기는 하나의 stdin 뮤텍스를 타고(§4-1) 그 blocking 쓰기에 상한을 둘 수단이 없다(§4-9 — `std::sync::Mutex` 에 시한부 lock 이 없고 std 파이프에 마감이 없다). ★**그 둘을 곱하면 자기잠금이 된다**★: 사용자 입력 쓰기가 파이프 backpressure 로 뮤텍스를 쥔 채 블록한 상태에서 읽기 스레드가 같은 뮤텍스를 기다리면 → **읽기 정지** → 서버 큐 포화 → ★**EOF 가 오지 않아 §3 의 EOF 처분이 영영 발화하지 않는다**★. 프로세스는 살아 있고 신호는 하나도 없다.

- ★**규율:** 읽기 스레드는 stdin 뮤텍스를 **직접 잡지 않는다.** 자기가 내야 하는 응답도 §4-9 가 세우는 **유계 큐 + 전용 writer 스레드**에 넣고 즉시 돌아온다★ — 그러면 **이 통로 안에서** stdin 을 blocking 으로 잡는 스레드가 그 writer 하나뿐이고, 읽기 스레드는 그것과 절대 경합하지 않는다.
  - ★**「워크스페이스에서 유일한 스레드」로 적지 말 것 — 거짓이다**★. claude json 경로의 `StdioTransport::send_input` 은 **오늘도** std `write_all` + `flush` 를 `stdin` 뮤텍스를 쥔 채 돈다(`crates/engram-dashboard-agent/src/transport/stdio.rs:295-304`). ★즉 이 정지 모양은 이 통로가 새로 들여오는 것이 아니라 **claude json 경로에 이미 있다**★ — §4-9 가 그것을 그대로 인정한다. 이 규율이 하는 일은 그 모양을 **없애는 것이 아니라 새 통로에서 읽기 경로와 갈라 두는 것**이다.
- ★**그 큐가 차면 응답도 거절된다**★. 거절은 §4-9 규칙 1 대로 **`Err` 로 즉시 돌아오고 로그에 남는다** — 무신호였던 것이 관측 가능한 거절이 된다. ★**단 그것이 세션 종료로 착지하지는 않는다**★ — 상대는 여전히 답을 못 받아 멈출 수 있고, 우리는 그 사실을 로그로만 안다(§4-9 의 「알려진 한계」 · §11). ★「거절 = 관측 가능한 종료」로 적었던 4판 문장은 걷었다 — 그 종료를 실행할 동사가 없다★(같은 절).
- ★**이 규율을 「승인 응답은 급하니 먼저 쓰게 하자」로 되돌리지 말 것**★ — 우선순위를 주려면 큐 안에서 주는 것이고, 뮤텍스를 직접 잡는 갈래는 위 자기잠금을 되살린다.

### 4-3. 하류 큐는 유계

읽은 것을 무한 버퍼에 쌓지 않는다. 우리 쪽 정본은 이미 유계다 — `OutputCore` 의 replay 링 버퍼. ★새 무계 큐를 만들지 말 것★.

### 4-4. ★인바운드 / 아웃바운드 request id 공간을 분리한다★

**구체적으로 무엇을 막나:** 우리가 낸 요청 id `0`(예: `thread/start`)과 **서버가 낸 요청 id `0`**(예: 명령 승인 요청)이 한 대기표를 공유하면, 승인 요청의 응답이 **우리 요청의 대기자에게 배달된다.** 우리는 `ThreadStartResponse` 자리에서 승인 결정을 받고, 서버는 답을 영영 못 받아 에이전트가 멈춘다.

- **아웃바운드** = 우리 카운터. 우리 대기표(id → 대기자)에 들어간다.
- **인바운드** = 서버 카운터. **우리 표에 넣지 않는다.** 받은 id 를 그대로 응답에 되돌려 주기만 한다.
- ★**판정 축은 id 가 아니라 「방향 + 봉투 모양」이다**★ — id 로는 원리상 가를 수 없다. 서버가 낸 id 는 **글자 그대로 되돌려 줘야** 하므로(바로 위 줄) 두 값 공간은 **겹칠 수밖에 없고**, 그것이 결함이 아니라 프로토콜 요구다.
  - **받은 줄에 `method` 가 있으면** 그것은 서버→클라 **요청/알림**이다 → 우리 대기표를 **조회하지 않는다**. 요청이면 그 id 를 그대로 실어 응답한다.
  - **받은 줄에 `result` 또는 `error` 가 있으면** 그것은 우리 요청의 **응답**이다 → 그때만 우리 대기표를 조회한다.
  - **두 카운터 규율은 그대로다** — 아웃바운드 카운터는 우리 것이고 서버 id 를 그 카운터에 섞지 않는다. 인바운드 id 는 **표에 넣지 않고 그 응답 한 번에만 쓴다.**
- 판정 한 줄(고쳐 적는다): ★**대기표 조회를 `method` 유무로 게이트하지 않는 구조면 그 구조가 결함이다.**★ 「id 만 보고 가른다」로 적힌 옛 문장은 만족 불가능한 수락 조건이었다 — 지운다.
- ★**JSON-RPC 2.0 라이브러리가 이것을 대신 해 주지 않는다**★ — 봉투를 우리가 정의한다(S1: `jsonrpc` 필드가 없다).

### 4-5. `-32001`(과부하) 처리

**재시도 대상이지 실패가 아니다** — 단 재시도는 **그 요청 하나의 재전송**이고 프로세스·연결을 다시 세우는 것이 아니다(★재연결은 안 한다 — ADR-0192 · §3★). 백오프 값은 구현 상수다. ★**실제로 언제 나오는지는 `[미확인]` 이므로 「안 나올 것이다」로 처리를 생략하지 말 것**★ — 생략하면 그 응답이 「모르는 오류」로 흘러 턴이 조용히 멈춘다.

### 4-6. 비-JSON stdout 라인은 무시하고, 죽지 않는다

근거 = paseo 의 Windows 사고(D6 `[문서]`). ★**파싱 실패를 fatal 로 두지 말 것**★. 단 **조용히 버리지도 않는다** — 로그에 남긴다. 마스킹 경로가 이미 있다(`transport/stdio.rs:24` 가 `engram_dashboard_base::logging::mask_secrets` 를 import 해 쓴다).

★**이것을 「관용적 파서」로 확대하지 말 것**★ — 규율은 **JSON 이 아닌 줄을 흘린다** 하나다. JSON 인데 모르는 이름이면 그건 §4-7 의 축이고, JSON 인데 모양이 다르면 그건 결함이다.

### 4-7. 프로토콜 버전이 없다(S8) — 그 대신 무엇을 하나

셋을 한다.

1. **기동 시 CLI 버전을 한 번 기록한다.** 선례가 이미 있다 — `crates/engram-dashboard-agent/tests/backend_contract.rs` 의 `record_version_once` 가 **운영 argv 로** `--version` 을 물어 찍고 ★값을 단언하지 않는다★(이 CLI 는 스스로 업데이트하므로 박아 두면 업데이트마다 빨개진다 — 그 파일 헤더가 그 판정을 적는다).
2. **`deprecationNotice`(D2)를 받으면 로그에 남긴다.** 알림 하나가 유일한 조기 신호다.
3. ★**모르는 알림 이름은 로그에 남기고 버린다**★ — 이름별 1회, 마스킹 경로를 타고, 카운터를 진단에서 볼 수 있게. ★**초판은 이것을 `Structured{kind,json}` 탈출구로 흘리라고 적었는데 걷었다**★ — 그 탈출구는 프론트에서 **codex 메서드 이름과 JSON 원본을 화면에 그린다**(§6-2 가 그 두 줄을 짚는다). 유실 방지의 값은 로그가 지고, 화면은 중립 어휘만 받는다.

★**「깨지면 알 것이다」로 두지 말 것**★ — 알림 **이름**이 조용히 바뀌면(D5 의 `newConversation` → `thread_start` 가 그 선례다) 우리가 아는 이름이 하나도 안 와서 **화면이 비고 그것이 유일한 신호**가 된다.

**그 침묵을 무엇이 깨나 — 셋을 겹친다(초판은 화면 하나에 걸었다):**

1. ★**번역 표 골든**★(§9-3) — 「아는 이름 전부가 중립 이벤트를 낸다」를 단언하므로 상류가 이름을 **바꾸면** 빨개진다. **화면 관찰보다 이르고 CI 에서 돈다.**
2. **위 3 의 로그** — 모르는 이름이 실제로 도착하는 것을 이름별 1회로 남긴다.
3. **위 1 의 CLI 버전 기록** — 사후에 「언제부터 바뀌었나」를 가른다.

★**그래도 남는 구멍 하나: 이름이 「사라지지 않고 새로 생긴」 경우**★ — 골든은 우리가 아는 이름만 덮으므로 신설 알림은 2 의 로그가 유일한 신호다. 그것을 화면으로 옮기지 않는 것이 §6-2 의 판정이고, 그 대가가 이 줄이다.

### 4-8. teardown — kill 인과는 그대로다. ★transport 안에는 한 줄도 더하지 않는다★

shutdown 메서드가 없다(S7 `[스키마]`). 그래서 「정리 = stdin 닫기」로 읽고 그것을 `transport.shutdown()` **안에** 끼워 넣고 싶어지는데, ★**그것이 이 저장소가 이름 붙여 금지해 둔 순서다.**★ 옛 초안이 그 순서를 그대로 처방했고(「① stdin 을 닫는다 → ② 짧게 기다린다 → ③ kill」), 그 문장을 걷었다.

**오늘 그 자리에 박혀 있는 불변식 — 그대로 유효하고 이 문서가 손대지 않는다:**

- ★**「순서 불변 — stdin close 는 kill 보다 절대 먼저 오면 안 된다(데드락, FIX 1)」**★(`crates/engram-dashboard-agent/src/transport/stdio.rs:330-337`). 인과가 그 주석에 적혀 있다: `send_input` 은 stdin 뮤텍스를 **blocking `write_all` 내내** 쥐고, 자식이 stdin 을 안 읽으면(파이프 backpressure) 그 write 가 영원히 블록해 락을 놓지 않는다 → kill **전에** `stdin.lock()` 을 시도하면 그 락을 영영 못 얻어 **kill 에 도달조차 못 하고** → pump 가 깨지 못해 → `core.join_pump` 가 **영구 hang** 한다(ADR-0001 인과가 멈춘다).
- 같은 주석이 그 유혹까지 미리 닫아 두었다 — 「※graceful-exit-via-stdin-close 는 필요 없다 — 어차피 여기서 kill 하므로」(`:337`).
- 오늘의 stdin 정리는 kill **뒤**의 `try_lock` + skip 이다(`:356-360`) — 못 얻으면 그냥 넘기고, 미정리 `ChildStdin` 은 transport drop 시 OS 가 회수한다.
- ★**회귀망이 이미 있다**★ — `shutdown_completes_even_if_send_input_blocks_on_full_pipe`(`:457`, `#[cfg(windows)]`). 순서를 뒤집으면 그 테스트가 타임아웃으로 잡는다.
- ★**§4-1 이 그 방아쇠를 더 자주 당긴다**★ — 승인 응답이 사용자 입력과 **같은 stdin 뮤텍스**를 통과하므로(D1), 쓰기를 쥔 채 블록할 수 있는 경로가 하나에서 둘로 는다. **codex 경로는 claude json 경로보다 이 데드락에 더 가깝다.**

**그리고 첫째 동사 안에서 기다리면 5초 계약이 늘어난다 — 옛 초안의 회계가 틀렸다:**

- `AgentSession::kill` 은 `transport.shutdown();` 다음에 `core.join_pump(timeout);` 을 부르는 **순차** 코드다(`crates/engram-dashboard-agent/src/session.rs:243-246`). 그래서 shutdown 안의 양수 대기는 5초 예산 **안에 드는 게 아니라 그 앞에 더해진다.**
- 계약 자신이 그것을 금한다 — 「자원 강제 종료(멱등). pump 종료 대기는 여기서 안 함(core.join_pump 몫)」(`crates/engram-dashboard-agent/src/transport/mod.rs:53`).
- ★그러므로 옛 초안의 「②의 값은 사용자 결정이 아니라 구현 상수다 … `join_pump(5s)` 예산 안에 든다」는 **두 군데가 틀렸다**★ — 예산 안에 들지 않고, 상수도 아니다(아래).

**그래서 2동사는 오늘 모양 그대로다:**

```
transport.shutdown()                     ← 첫째 동사 (오늘 그대로 — 이 문서가 아무것도 더하지 않는다)
   ① shutdown 플래그 → child.kill + wait                  (stdio.rs:340 · :342-347)
   ② TerminateJobObject — 손자까지                        (:349-354)  ← write 핸들을 닫는 것이 이것
   ③ stdin try_lock + take (best-effort, 못 얻으면 skip)   (:356-360)
core.join_pump(5s)                       ← 둘째 동사 (오늘 그대로)
```

- **EOF → pump break → `core.finish` → done_tx** 인과 그대로. 파이프는 자식 트리가 write 핸들을 모두 닫으면 read 가 EOF 로 깨므로 PTY 처럼 watcher 가 필요 없다(`transport/stdio.rs:11-14`).
- **finalize 1회**(ADR-0005) 그대로 — terminal 전이·알림은 pump 단독이고 이 변경이 그 소비자를 늘리지 않는다.

**codex 를 정상 종료시키고 싶다면 그 자리는 transport 밖이다 — 세션 층, 첫째 동사 「앞」:**

```
(선택) 통로 구현체의 graceful close 시도   ← 유계. 어떤 결말이든 아래로 떨어진다
      ↓
transport.shutdown() → core.join_pump(5s)   ← 위 그림 그대로, 무변경
```

조건 넷을 다 지켜야 이 배치가 성립한다:

1. **유계** — 상한을 넘으면 결과를 기다리지 않고 내려간다.
2. **실패·타임아웃이 no-op** — 예외도 특별 경로도 만들지 않고 그대로 평소 kill 경로로 떨어진다. 즉 최악의 경우가 **오늘의 동작**이다.
3. **transport 계약을 한 줄도 안 바꾼다** — `shutdown()` 은 여전히 「기다리지 않는 강제 종료」다.
4. 그 시도가 stdin 을 쓴다면 그 쓰기도 §4-1 의 뮤텍스를 타므로 ★**여기서도 blocking lock 금지**★ — 쓰기 자체가 유계여야 한다(§4-9).

**등급을 정직하게 적는다:**

| 주장 | 등급 |
|---|---|
| app-server 에 shutdown 메서드가 없다 | `[스키마]`(S7) |
| stdin 을 닫으면 종료코드 0 으로 끝난다 | `[실측]`(L2) — ★**단 그 관측은 활성 턴이 없는 상태였다**★. ★1·2 판이 여기 달았던 「이 PC 의 app-server 왕복은 `initialize` 하나뿐이다」는 **낡았다 — 지웠다**★: 왕복은 넷이다(`initialize` · `thread/start` · `turn/start` · `turn/interrupt` — L16). 이 행이 좁은 이유는 왕복 수가 아니라 **stdin 닫기를 턴 도중에 해 본 적이 없다**는 것 하나다 |
| **턴 도중에 stdin 을 닫아도 곧 exit 한다** | ★`[미확인]`★ — 어디서도 관측한 적이 없다. **위 `[실측]` 에서 유도한 추론일 뿐이다.** 그러니 graceful 대기값을 「보통 금방 끝난다」로 정당화하지 말 것 |
| 정상 종료면 rollout 이 flush 되고 hard kill 이면 잘린다 | `[미확인]` — 이 갈림이 곧 §10-10 이 묻는 것이다 |

#### ★그런데 그 「유계 graceful 시도」에 오늘 **동사가 없다** — 초판이 그것을 안 적었다★

**S7 이 말하는 정리 = stdin 닫기**인데, ★**stdin 을 「죽이지 않고 닫는」 수단이 이 저장소에 하나도 없다**★. 줄로 적는다:

- `AgentTransport` 여섯 동사에 `close_stdin` 이 없다(`crates/engram-dashboard-agent/src/transport/mod.rs:41-57`, 전문 판독). `shutdown()` 은 계약상 「자원 **강제** 종료(멱등)」다(`:53`) — 그 정의 안에 「부드럽게」가 없다.
- stdin 이 실제로 drop 되는 유일한 자리는 `StdioTransport::shutdown` **4단계**이고 그것은 ★**kill 뒤**★이며 `try_lock` best-effort 라 **경합하면 그냥 건너뛴다**(`transport/stdio.rs:356-360`).
- ★**`ControlCaps.graceful_shutdown` 은 세 구현체 전부 `false` 이고 그 값을 읽는 분기가 워크스페이스에 하나도 없다**★ — `stdio.rs:381` · `pty.rs:355` · `crates/engram-dashboard-agent/src/transport/api.rs:70`. 유일한 비-테스트 소비는 wire 로 그대로 복사하는 한 줄이다(`crates/engram-dashboard-daemon/src/connection_core.rs:348`). **즉 그 칸은 신고만 하고 아무 동작도 켜지 않는다** — 「이미 그 개념이 계약에 있으니 쓰면 된다」로 읽지 말 것.
- 그리고 **동사를 새로 내는 것이 값이 싸지 않다.** blocking `stdin.lock()` 을 하는 `close_stdin` 은 **위에 인용한 데드락 불변식을 그대로 되살린다**(`stdio.rs:330-337`) — 자식이 stdin 을 안 읽는 상태에서 그 락은 영영 안 잡히고, 그 시도가 kill **앞**에 서면 kill 에 도달조차 못 한다. 그래서 새 동사는 「계약 변경 + 명시된 락 순서 위험」 둘을 함께 진다.

**그래서 §10-10 의 (가)를 값과 함께 다시 적는다 — 갈래 셋이다:**

| 갈래 | 동사 | 값 |
|---|---|---|
| **(가-1) transport 계약에 `close_stdin` 을 더한다** | 새 trait 메서드. 구현체 셋 전부 | ★**`try_lock` + 실패하면 no-op 이어야 한다**★ — blocking 이면 `stdio.rs:330-337` 의 hang 이 돌아온다. 그런데 `try_lock` 이면 **경합 시 조용히 아무 일도 안 하는 graceful close** 가 되어, 「보냈다」와 「못 보냈다」를 호출자가 구별할 수 없다(반환형에 그 축이 필요해진다) |
| **(가-2) 통로 구현체가 stdin 을 안 만지고 프로토콜로만 시도한다** | 없음 — 기존 `send_input` | ★**app-server 에 shutdown 메서드가 없다**(S7)★. 즉 보낼 요청이 **없다.** `turn/interrupt` 는 턴 범위라 프로세스를 안 내린다(S6) · `thread/unsubscribe` 는 §3 이 호출자 없음으로 뺐다. **그러니 이 갈래는 오늘 스키마로는 실행 불가다** |
| **(나) 안 받는다 — 오늘 claude 와 같은 hard kill** | 없음 | 변경 0. 대가 = rollout flush 여부가 codex 손에 남고 그 결말이 `[미확인]` 이다 |

★**정직한 판정: (가)를 「유계 대기값 하나만 고르면 되는 일」로 적은 초판 프레이밍은 실행 가능한 경로를 가리키지 않았다.**★ (가-2) 는 스키마에 동사가 없어 불가이고, (가-1) 은 **계약 변경 + 반환 축 추가 + 데드락 불변식 옆에서 작업**이다. 그러니 §10-10 은 「값을 얼마로 하나」가 아니라 ★**「그 문을 낼 값을 낼 것인가」**★를 묻는다. ★어느 답이든 `transport.shutdown()` **안**은 한 줄도 안 바뀐다★ — 그 안의 순서는 명시된 불변식이고 §8 의 「손대지 않는 것」 1번이다.

★**그래서 「유계 graceful 시도를 넣나」는 구현 상수가 아니라 결정이다**★ — 사용자가 체감하는 것이 걸려 있다(codex 가 죽기 전에 자기 rollout 을 flush 하나). 오늘 claude 는 그 시도 없이 hard kill 을 받는다. **§10-10.**

### 4-9. ★쓰기 쪽 정지(stall)에도 탐지 수단이 있어야 한다★

§4-2 는 **읽기**를 비우게 하고 §3 의 EOF 처분은 **EOF** 에서 발화한다. 그 사이에 아무도 안 보는 창이 하나 있다.

**구체적으로 무엇이 일어나나:** 서버가 stdin 을 안 읽기 시작하면(큐 포화 · 서브프로세스에 매달림 · #40766 류의 반죽음) 파이프 backpressure 로 `send_input` 의 `write_all` 이 뮤텍스를 쥔 채 블록한다. 그 상태의 신호는 **하나도 없다** — 프로세스는 살아 있고, EOF 는 안 오고, 대기 중 RPC 는 마감이 없으면 실패하지 않고, `-32001` 도 오지 않는다(그것은 서버가 **읽은 뒤** 거절하는 응답이다). 그리고 §4-1 때문에 **사용자 입력과 승인 응답이 함께** 그 뮤텍스 뒤에 갇힌다 — 승인이 막히면 §6-1 의 승인 행 그대로 에이전트가 멈춘다.

**규율 둘 — ★둘 다 「기다리는 쪽을 깨우는 것」이고, 멈춘 쪽을 죽이는 것이 아니다★:**

1. ★**아웃바운드 쓰기의 *호출자* 대기는 유계다**★ — 유계 큐 + 전용 writer 스레드를 `send_input` 앞에 세우면, 큐가 찰 때 **호출자는 `Err` 로 즉시 돌아온다.** 무한정 블록하는 **호출**을 남기지 않는 것이 이 규율이고, ★**정지 그 자체를 없애는 것이 아니다**★(아래 「대가」).
2. ★**대기 중 RPC 에는 마감(deadline)이 있다**★ — 「응답이 안 온다」와 「서버가 죽었다」를 EOF 하나로만 가르면 EOF 가 안 오는 이 창에서 대기자가 영구히 남는다. 그 사고의 선례가 이 저장소에 있다(겹친 `request_id` 가 옛 대기자를 영구 hang 시킨 GUI 실측 2026-08-18 — 회귀망 `cargo test -p engram-dashboard --test daemon_client_pending`). ★**마감이 깨우는 것은 그 대기자 하나뿐이고 세션을 내리지 않는다**★.

#### ★그 정지가 오래가면 무엇이 푸나 — **강제 종료 감시를 만들지 않는다**★ (사용자 결정)

★**4판은 여기에 「그 실패가 **연결 = 끊김**으로 올라가 §3 의 EOF 처분과 같은 착지를 탄다」를 적었다. 그 문장을 걷는다 — 실행할 주어가 없다.**★ 막힌 write 는 **자식을 살려 둔 채** 멈춘 것이라 EOF 를 만들지 않고, 그러니 「연결이 끊겼다」로 올릴 사건 자체가 발생하지 않는다. 그 상태에서 세션을 내리려면 **타이머로 산 에이전트를 죽이는 동사**를 새로 만들어야 하는데, ★그 동사의 소유자가 이 문서 어디에도 없다★ — 그리고 §3 이 그 부류를 이름 붙여 금해 두었다(「근거 없는 상태를 기계에 남기면 **구현자가 트리거를 발명한다**」).

**그래서 실제로 서는 것은 셋이다:**

- ★**우편 쪽은 기존 30분 상한이 풀어 준다**★ — 메시징 커널에 `BUSY_MAX_TURN`(30분) fail-open 안전 밸브가 이미 있고(`crates/engram-dashboard-messaging/src/busy.rs:44-57`), 주기 sweep 이 그 시간을 넘긴 「턴 중」 관측을 **잔해로 판정하고 그 주인을 도어벨로 깨운다**(`:136-146`). ★**그것이 이 저장소가 「너무 오래 멈췄다」에 취하는 자세다 — 관측을 낡은 것으로 선언하지, 죽이지 않는다.**★ 즉 정지가 길어져도 그 수신자 앞 우편이 영구히 막히지는 않는다.
- ★**화면에는 아무 신호가 없다 — 알려진 한계다**★. 프로세스는 살아 있고 EOF 는 안 오고 세션 상태도 안 바뀐다. 사용자가 보는 것은 **응답이 안 오는 에이전트** 하나뿐이고, 그 원인이 「쓰기가 막혔다」인지 「모델이 오래 생각한다」인지 구별할 표면이 오늘 없다. → §11.
- ★**겪거나 재고 나면 그때 값과 함께 만든다**★ — 지금은 이 정지가 실제로 일어나는지조차 `[미확인]` 이고(§11), 실측 0 위에서 「몇 분이면 죽인다」를 고르면 그 수가 지어낸 것이 된다. **탐지 수단(위 규율 둘 + 로그)을 남기는 데까지가 이 절이 정하는 전부다.**

**대가를 적어 둔다:**

- ★**막을 수 있는 것과 못 하는 것을 갈라 적는다 — 초판이 이 둘을 섞었다**★.
  - **못 한다:** `write_all` 자체에 상한을 두는 것. `AgentTransport::send_input` 은 동기이고(`crates/engram-dashboard-agent/src/transport/mod.rs:45`) 구현체의 쓰기는 std 블로킹 `write_all` + `flush` 를 **`stdin` 뮤텍스를 쥔 채** 돈다(`crates/engram-dashboard-agent/src/transport/stdio.rs:295-304`). std 파이프에 마감을 붙이는 수단이 없고, `std::sync::Mutex` 에는 시한부 lock 이 없다.
  - **할 수 있다:** **호출자의 대기**에 상한을 두는 것. 유계 큐 + 전용 writer 스레드를 `send_input` **앞에** 세우면, 그 스레드 하나만 영영 블록하고 나머지 호출자는 큐가 찰 때 **즉시 실패로** 돌아온다. 정지 자체는 안 사라지지만 **무신호가 관측 가능한 거절로 바뀐다** — 이 절이 요구하는 것이 그것이다.
- ★**그리고 그 자리는 이제 정해졌다 — codex 통로 구현체 자신의 `send_input` 안이다**★(ADR-0189 · §5-1). 1·2 판은 이 자리를 seam 안 넷에 걸어 재고 있었는데, ★그때도 결론은 「네 안 모두에 소유자가 있어 결정이 아니다」였다★. 계약은 안 바뀐다 — 블로킹 `send_input` 앞에 큐를 세우는 데는 **새 trait 메서드가 필요하지 않다.**
  - ★**claude 를 건드리나 — 이 절만 보면 ✗ 다**★. codex 통로 구현체는 **새 객체**라 `StdioTransport::send_input` 을 한 줄도 안 건드린다. ★**단 §4-10 의 입력 큐는 다르다**★ — 그쪽은 **json 모드 공통**이라 claude json 의 쓰기 타이밍을 바꾼다(ADR-0190). **두 큐를 뭉개지 말 것**(§4-10 의 대조 표).
- **유계 큐가 차면 그때는 입력을 거절한다** — 무계로 늘리면 §4-3 위반이다. 거절은 **사용자에게 보이는 실패**라야 한다(조용히 버리면 「전송 실패」와 「모델이 무시했다」가 구별되지 않는다 — ADR-0088 이 세운 그 구별).
  - ★**그 거절은 반드시 `Err` 여야 한다 — 짧은 `Ok` 로 표현할 수 없다**★. `WriteOutcome` 의 두 바이트 칸은 **독립 측정값이 아니라 `bytes.len()` 을 구성상 그대로 복사한 값**이고(`crates/engram-dashboard-agent/src/session.rs:178-184`), 그 타입의 주석이 그것을 명시로 못 박는다 — 「완결성 신호 = Ok-vs-Err 이지 바이트 비교가 아니다 … `bytes_written` 은 독립 계측값이 아니라 `bytes_requested` 를 구성상 그대로 복사한 값이다(short-write 탐지 불가 — 비교하면 항상 같다, 동어반복)」(`crates/engram-dashboard-agent/src/types.rs:663-671`, 같은 판정이 `session.rs:151-154` 에도 있다). ★그러니 「일부만 받았다」를 그 칸으로 신고하려 하지 말 것★ — 그 순간 ADR-0088 이 세운 구별이 **동어반복 위에 서게 된다.**
  - ★**그리고 그 축은 §4-10 에서도 **글자 그대로 같다**★ — 두 절이 ADR-0088 을 다르게 적용하지 않는다. **영수증은 하나이고 뜻도 하나다**: `Ok` = 「요청 바이트를 우리가 전량 맡았다」 · `Err` = 「안 맡았다」. ★**「담겼다」와 「실제로 썼다」를 두 등급으로 쪼개 신고하는 갈래는 기각됐다**★(사용자 결정 — §4-10 · §10-20).
- 상한 **값**은 구현 상수다(§4-5 의 백오프와 같은 부류). ★단 그 값의 근거를 주석에 적는다 — 실측이 0 이라 지금 고르는 수는 지어낸 것이다.★
- ★**이 규율은 「app-server 가 실제로 그렇게 멈추나」에 매달려 있지 않다**★ — 멈추지 않는다는 증거도 없고(`-32001` 이 언제 나오나·큐 깊이가 얼마인가가 `[미확인]`, D3), 멈췄을 때의 결말이 **영구 무신호**라 §4-5 와 같은 판정을 받는다: 「안 나올 것이다」로 생략하지 않는다.


### 4-10. ★json 모드 입력은 큐에 담고, 보낼 수 있게 되면 순서대로 흘린다★ (ADR-0190)

★**이 규율은 codex 전용이 아니다 — json 모드 공통이다.**★ 그래서 §5(통로) 옆이 아니라 여기, 쓰기 축 규율들과 나란히 둔다: §4-1(writer 하나)과 §4-9(유계 쓰기)가 **같은 창구**를 다루고 이 절은 그 **앞에** 붙는다.

★**단 「json 모드 공통」이라는 이름을 그 정책의 *근거* 로 읽지 말 것 — 정책이 매달린 것은 wire 형식이 아니라 백엔드다**★

이 절이 세우는 정책의 알맹이는 **「한 번에 한 턴, 턴 중에는 안 흘린다」**(직렬 턴)이다. 그것이 지금 참인 이유는 **오늘 우리가 가진 json 백엔드 둘이 그렇기 때문**이다 — claude json 은 한 호출이 완결된 유저 턴 하나이고(`crates/engram-dashboard-agent/src/session.rs:136-141` 의 호출 계약), codex 는 턴 중 반영 동사(`turn/steer`)를 이번 범위에서 뺐다(아래 규율 5). ★**JSON 이라는 형식이 직렬 턴을 함의하지는 않는다**★ — **JSON 이면서 동시 턴이나 steer 를 지원하는 셋째 백엔드**가 오면 이 정책은 그 백엔드에 안 맞고, 그때 갈라야 하는 축은 「wire 가 JSON 이냐」가 아니라 ★**「그 백엔드가 턴을 겹쳐 받나」**★다. **지금 그 축을 만들지 않는다**(그런 백엔드가 없고 만들 근거가 실측 0이다) — 대신 그 사실을 §11 에 남긴다. ★이 절의 이름을 근거 삼아 「JSON = 직렬」을 셋째 백엔드에 강요하지 말 것★.

**왜 필요한가:** codex app-server 는 프로세스가 뜬 뒤에도 **바로 말을 걸 수 없다** — `initialize` 왕복(★약 130ms★`[실측]`)과 `thread/start` 왕복이 끝나야 `thread.id` 를 알고, 그것 없이는 `turn/start` 요청을 **만들 형태 자체가 없다**. 그 구간에 입력이 오면 보낼 곳이 없다. ★**그리고 그 구간에 입력이 실제로 온다**★ — 사람이 130ms 창에 칠 일은 드물지만 LLM 경로는 그렇지 않다. 이 저장소에 이미 **T-28**(갓 스폰된 터미널 수신자가 배달된 편지를 제출하지 않는다)이 있고 원인이 같은 축이다 — 수신자가 준비되기 **492ms 전에** 제출이 쓰였다(데몬 debug 로그 실측).

★**「준비 전 입력」은 특수 케이스가 아니다**★ — **턴이 도는 중에 온 입력과 같은 상황**이고(둘 다 「지금 못 보낸다」) 다른 것은 **언제 흘려보내나**뿐이다. 그래서 둘을 하나로 합친다.

**규율 여섯:**

1. ★**「준비된 뒤에 쏘라」는 규칙을 만들지 않는다**★ — 그 규칙을 두면 부르는 쪽마다 준비 여부를 알아야 해서 로직이 복잡해진다. **부르는 쪽은 그냥 보내고, 큐가 알아서 담는다.**
2. ★**입력을 버리지 않는다**★ — 상한을 넘거나 준비가 끝내 실패하면 **그때 오류로 돌린다.** `send_input` 이 이미 `Result` 다(`crates/engram-dashboard-agent/src/transport/mod.rs:45`). ★조용히 버리는 갈래는 기각됐다 — 그것이 T-28 이 기록한 사고 부류다★.
3. ★**「지금 보낼 수 있나」를 각 통로 구현체가 신고하는 축을 하나 만든다**★ — **판정은 각 구현체 안에서** 하고(codex = `thread/start` 응답을 받았고 턴이 안 돈다 / claude json = 턴이 안 돈다) **축은 공용이다.** 백엔드 이름으로 가르지 않는다.
4. ★**해제 판정의 주인은 각 통로 구현체다 — ADR-0193 이 그렇게 정했다**★. ADR-0190 결정 4 는 원래 「턴이 도는가는 **코어가 이미 안다** — 큐는 읽는다」였는데, ★**ADR-0193 이 그 결정을 개정해 판정을 각 구현체 안으로 옮겼다**★. ★**이 문서가 그 결정을 다시 해석하지 않는다 — 사유의 정본은 그 ADR 이고 아래는 결과만 옮긴 것이다**★:
   - **claude json** — 해제 조건 「턴이 안 돈다」를 **코어에서 읽어도 된다.** claude 는 `input_echo_event` 를 선언해 **쓰는 순간** 턴 신호를 만들어 미관측 창이 없다(`crates/engram-dashboard-agent/src/session.rs:175-176`). 접근자도 이미 있다(`AgentManager::turns()` — `crates/engram-dashboard-agent/src/manager.rs:537`). ★**ADR-0127 저촉 없음**★ — 그 결정이 고정한 것은 턴 관측을 **지우는 호출 지점 둘**이고, 여기서 느는 것은 **읽는 소비자** 하나다(기존 소비자 = 메시징 busy 게이트).
   - **codex** — ★**코어를 읽으면 그 자리에서 무너진다**★. codex 는 `input_echo_event` 를 선언하지 않아 trait 기본값 `None` 을 타고(`crates/engram-dashboard-agent/src/backend/mod.rs:209-211`), 그래서 `turn/start` 를 쓴 순간부터 첫 알림 도착까지 **미관측**이며, ★**미관측은 코어에서 idle 로 흡수된다**★(`crates/engram-dashboard-messaging/src/busy.rs:177-180` — `turn_fact` 가 `None` 이면 곧장 `false`). 그 창에 큐가 다음 항목을 흘리면 **`turn/start` 두 개가 겹친다.** ★그래서 codex 는 **자기 턴 상태기계**를 읽는다★(§3) — 요청을 **보낸 그 자리에서** 진행 중으로 넘어가므로 그 창이 없다. **이 둘이 ADR-0193 이 든 근거 그대로다.**
   - ★**그 「보낸 자리에서 전이」는 판정과 전이가 **한 락 안**에 있어야 성립한다 — 이 문서가 그것을 규율로 못 박는다**★. `AgentTransport::send_input(&self, …)` 은 불변 참조라 **동시 호출이 구조적으로 가능하고**(`crates/engram-dashboard-agent/src/transport/mod.rs:41-45` · §4-1 이 같은 사실 위에 선다), 「지금 보낼 수 있나」와 「진행 중으로 전이」가 갈리면 두 호출이 **둘 다 idle 을 보고** 각자 `turn/start` 를 낸다 — 겹침을 닫으려고 세운 규율이 그 자리에서 새는 것이다. **check-and-set 을 원자적으로 한다.** ★**새 락을 만들지 않는다**★ — §4-1 이 이미 요구하는 구현체 안 뮤텍스가 이 전이를 함께 진다. ★그리고 **그 락을 쥔 채 blocking write 를 하지 않는다**★ — 흘려보내기는 §4-9 의 유계 큐에 **넣고 즉시 놓는** 것까지다(ADR-0006 락 순서: 락 보유 중 외부 호출 금지).
   - ★**§7-4 ④가 이 줄로 닫히지 않는다**★ — 여기서 닫은 것은 **큐의 해제 조건**이고, 우편 게이트는 **여전히 코어를 읽는다.** codex 의 미관측 창은 큐에서는 없어지고 **우편에서는 그대로 남는다** — 그것을 닫는 것은 §7-4 의 ①②④ 선언이다.
   - ★**턴 중에 흘려보내지 않으므로 `turn/steer` 는 이 설계에 안 든다**★ — 큐는 **턴이 끝난 뒤** 흘리고 그때 동사는 `turn/start` 다. 나중에 「턴 중 즉시 반영」을 고르면 그때의 동사는 `turn/steer`(+`expectedTurnId` 선행조건 — S6)이지 ★둘째 `turn/start` 가 아니다★. 그 조작은 규율 5 가 이번 범위에서 뺐다.
5. ★**이번에 만드는 것은 큐뿐이다**★ — 항목 빼기 · ESC 로 대기분 전부 밀기 · 순서 바꾸기 같은 **조작은 만들지 않는다.** 흘려보내는 룰도 「보낼 수 있게 되면 **순서대로**」까지다. 근거 = **큐가 있으면 어떤 조작 규칙도 나중에 얹을 수 있다**(사용자 보류 — 「다른 claude·codex 앱들을 보고 편한 쪽을 판단한다」).
6. **PTY 통로는 이번 범위 밖이다** — 그쪽 증상이 **T-28** 이고, 그것은 스스로 「수신자 준비됨 신호가 없어 조사부터 시작하는 단독 주제」로 표시돼 있다. 끌어들이면 2a 가 커진다.

★**claude json 모드의 동작이 바뀐다 — 이것이 이 규율이 codex 전용이 아닌 이유이자 사용자 확인을 받은 이유다**★. 지금 claude json 은 입력을 **즉시** 밀어 넣는다. 큐가 생기면 **턴 중 입력이 즉시 나가지 않고 대기한다.** ★§8 의 「claude 실행 경로는 한 줄도 안 바뀐다」를 이 항목에 적용하지 말 것★ — 그 문장은 ADR-0185 의 **spawn·복원 경로**에 대한 것이고, 여기는 **쓰기 타이밍**이라 다른 축이다.

#### ★영수증은 하나로 퉁친다 — `Ok` 의 뜻은 안 쪼갠다★ (사용자 결정 · §10-20)

**적대 리뷰가 이 자리를 BLOCK 했다: 담긴 직후 `Ok` 면 사용자 에코와 배달 성공이 기록된 뒤 writer 가 막혀도 호출자가 못 듣는다. ★그 지적을 「영수증을 둘로 쪼갠다」로 풀지 않는다★ — 사용자 결정이고, 사유는 「우편은 보냈고 받는 쪽이 소화한다」다.**

- ★**오늘도 그 바 높이다 — 큐가 낮추는 것이 아니다**★. 터미널 경로의 `send_input` 은 `write_all` + `flush` 를 **호출 안에서** 끝내고 그 `Ok` 의 뜻은 「요청 바이트 **전량 수용**」이다(`crates/engram-dashboard-agent/src/session.rs:150-156` · ADR-0088). ★**그런데 받는 CLI 가 그것을 읽었는지·행동했는지는 오늘 아무도 확인하지 않는다**★ — 즉 **「전달했다, 소화는 상대 몫」이 이미 이 저장소의 자세**이고, 큐는 그 바를 내리지 않는다.
- **그래서 뜻을 하나로 유지한다** — `Ok` = 「요청 바이트를 **우리가 전량 맡았다**(순서 확정 포함)」 · `Err` = 「안 맡았다」. ★**「담겼다」와 「실제로 썼다」를 두 등급으로 쪼개 신고하지 않는다**★. 우편 계층도 그대로 「전달했다」를 적는다(ADR-0088 배달 관측 레코드). ★반환형에 축을 더하는 갈래는 그래서 아예 안 열린다 — `WriteOutcome` 의 바이트 칸이 **독립 계측이 아니라 `bytes_requested` 의 구성상 복사**라 그 축을 나를 수도 없다★(`crates/engram-dashboard-agent/src/types.rs:663-671`).
- **여전히 원래 호출이 `Err` 로 돌아오는 것 셋**(ADR-0190 그대로 — 입력을 버리지 않고, 상한 초과·준비 실패는 오류로 돌린다): ① **큐 상한 초과** ② **준비가 끝내 실패**(연결이 끊겨 이 화신에서 보낼 길이 없어짐) ③ **준비 상태에서 즉시 흘려보낸 write 가 실패.** ★셋 다 맡기 전에 갈리므로 `Ok` 로 위장할 자리가 없다★(§4-9 의 「짧은 `Ok` 금지」와 **같은 문장**).
- ★★**한 가지가 실제로 바뀐다 — 그리고 그것은 해결이 아니라 한계다**★★. 오늘은 write 가 **호출 도중** 일어나 실패가 `Err` 로 **그 호출에** 돌아간다. 큐가 앞에 서면 ★**호출이 돌아간 뒤에 실패한 write 는 그 호출자에게 돌아갈 길이 없다**★. ★**이 문서는 그 길을 만들지 않는다 — 정정 채널을 발명하지 않는다**★. → §11 의 한계 행.
  - 남는 것은 **보이는 것**이지 **돌아가는 길**이 아니다: 통로 구현체는 `start(core)` 로 `core` 를 쥐므로 그 실패를 출력 스트림에 `Error{message}` 로 낼 수 있고(이미 있는 중립 어휘 — §6), 프론트는 그것을 항목으로 그리며 `turnDone` 을 올려 대기 인디케이터를 닫는다(`src/components/slot/structuredAccumulator.ts:101`). ★**그러나 그것은 화면·LLM 이 뒤늦게 보는 것이고, `Ok` 를 받은 호출자도 이미 적힌 배달 관측 레코드도 고쳐지지 않는다.**★
  - ★**「그 실패가 연결 = 끊김으로 올라가 세션이 착지한다」로 적었던 4판 문장은 걷었다**★ — 그 착지에 **실행할 주어가 없다**(§4-9 의 「강제 종료 감시를 만들지 않는다」).
- ★**에코가 먼저 나간다 — 그리고 이 결정 아래에서는 그것이 옳다**★. 합성 유저 에코는 `send_input` 이 `Ok` 로 돌아온 뒤에 발화하는데(`session.rs:163` → `:175-176`), 그 `Ok` 의 뜻이 **「우리가 맡았다」 하나**이므로 에코가 가리키는 사실이 정확히 그것이다. ★**그래서 「에코를 큐 방출 뒤로 미룬다」 갈래는 없어졌다**★ — 미룰 사유였던 「`Ok` 가 두 뜻」이 사라졌다. 남는 화면 질문은 ★「대기 중」을 표시하나★ 하나이고, 그것이 §10-17 이다.

**§4-9 의 유계 writer 와 같은 것인가 — 갈라 적는다:**

| | §4-9 의 유계 큐 | 이 절의 입력 큐 |
|---|---|---|
| 무엇을 흡수하나 | **쓰기 자체가 블록**하는 것(파이프 backpressure — 서버가 stdin 을 안 읽는다) | **지금은 보낼 형태가 없는 것**(준비 전 · 턴 진행 중) |
| 언제 풀리나 | 서버가 다시 읽기 시작할 때 | **준비 완료** 또는 **턴 종료** |
| 차면 | `Err` 로 거절(§4-9) | `Err` 로 거절(위 2) |

★**둘이 같은 창구 앞에 선다 — 그래서 구현이 한 덩어리가 될 수는 있어도 「같은 문제」로 뭉개지 말 것**★. 한쪽만 있으면 다른 쪽 증상이 그대로 남는다: 유계 writer 만 있으면 준비 전 입력이 여전히 갈 곳이 없고, 입력 큐만 있으면 「준비됐다고 신고했는데 쓰기가 영영 안 풀리는」 창에서 **호출자가 무한정 매달린다.** ★**단 유계 writer 가 있어도 그 정지가 사라지지는 않는다**★ — 바뀌는 것은 **호출자가 `Err` 로 돌아온다**는 것뿐이고, 화면 신호는 여전히 없다(§4-9 의 「강제 종료 감시를 만들지 않는다」).

**열린 채로 두는 것 넷 — ★구현 시 정하고 근거를 코드 주석에 남긴다★:**

- ★**큐 *길이* 상한과 초과 시 동작 — 규율은 확정이고 값만 미정이다**★. 초과는 **`Err`**(위 셋 중 ①). ★**여기엔 ADR-0190 과의 긴장이 없다**★ — 넘칠 때 호출자가 **그 자리에서** 듣기 때문이고, 그것은 「입력을 버리지 않는다」에 어긋나지 않는다(버림 = 무신호). 값은 실측이 0 이라 지금 고르는 수가 지어낸 것이다(§4-9 의 같은 판정).
- ★**대기 *시간* 상한 — 위와 같은 항목이 아니다. 4판이 둘을 한 줄에 묶은 것을 가른다**★. ★**시간 상한은 ADR-0190 의 「입력을 버리지 않는다」와 정면으로 부딪힌다**★: 큐가 안 풀리는 이유는 대개 **정당하게 긴 턴**이고, 시간으로 자르면 **멀쩡히 순서를 기다리던 항목을 `Err` 로 떨어뜨린다** — 길이 상한과 달리 「넘쳤다」가 아니라 「기다렸다」를 벌하는 것이다. 그리고 「긴 턴」과 「영영 안 풀리는 턴」을 구별할 수단이 오늘 없다(§4-9 의 정지가 `[미확인]`). ★**그래서 이 문서의 기본은 「시간 상한을 두지 않는다」이고, 두려면 그 충돌을 먼저 푼다**★ — 연결이 끊기는 쪽은 이미 위 셋 중 ②가 덮으므로 **「영구히 매달린다」의 나머지 경우는 살아 있는 연결의 긴 턴뿐**이다. → §11.
- ★**화면에 「대기 중」임을 알릴 수단**★ — **오늘 프론트에 그 개념이 0 이다.** 에코는 즉시 뜨므로 입력이 사라져 보이지는 않지만, ★**담겨서 아직 안 나간 것과 이미 나간 것을 화면에서 구별할 수 없다**★. 범위·모양은 후속이고 이 문서가 정하지 않는다. ★**§10-17 이 이제 이 질문 하나만 진다**★ — 함께 묶여 있던 에코 순서 갈래는 사용자 결정으로 닫혔다(위 절).
- ★**우편 파킹과 기능이 겹친다 — 그리고 그 중첩을 지금 풀지 않는다**★. 우편은 수신자가 바쁘면 편지를 파킹해 두고 상한(30분 fail-open)까지 기다린다. 큐가 생기면 **대기 자리가 둘**이 된다. **지금은 둘을 따로 둔다**(사용자 결정 — 나중에 우편 쪽을 고칠 것을 전제로 한다). ★**그 판단 전까지 「편지가 두 번 대기한다」가 실재할 수 있음을 알고 간다.**★

---

## 5. 통로 — ★ADR-0189 가 자리를, ADR-0191 이 **만들어 넘기는 법**을 확정한 모양★

★**이 절은 더 이상 선택지를 재지 않는다.**★ 1·2 판은 여기에 안 넷(가·나·다·라)을 놓고 「어느 안이 맞나」를 물었고, 두 판의 권고가 **둘 다 없는 경로를 가정해** 무너졌다(그 두 실패의 모양은 §5-8 이 보존한다 — 다음 세션이 같은 방식으로 틀리지 않게). **ADR-0189 가 그 갈림을 닫았고, 3판이 남겨 둔 마지막 구멍(「그 통로를 누가 만들어 조립점에 꽂나」)을 ADR-0191 이 닫았다.** 아래는 두 결정의 모양과, 그것들이 **바꾸지 않는** 것들이다.

### 5-1. 확정된 모양 — 한 문단

**codex app-server 통로는 `crates/engram-dashboard-agent/src/backend/codex/` 안의 struct 이고 `AgentTransport` 를 구현한다. 그것을 만드는 코드도 같은 폴더 안에 있고, ★조립점은 그것을 받아 쓸 뿐 구체 타입을 이름으로 모른다★**(ADR-0191).

- ★**공용 계약(`AgentTransport` 여섯 메서드)에 이 결정이 더하는 것은 없다**★ — 세션은 지금과 똑같이 그 여섯만 부른다(`crates/engram-dashboard-agent/src/transport/mod.rs:41-56`). ★3판이 여기에 「읽기 함수 하나」를 더하려 했던 것은 철회됐다★(§3 — 근거가 거짓이었고 필요도 없었다).
- **통로 종류 축이 셋이 된다** — 터미널(`PtyTransport`) · 파이프(`StdioTransport`) · **파이프 + 양방향 JSON**(이번 것). ★**백엔드 이름으로 가르지 않는다**★ — 축은 「우리 쪽 통로에 무엇을 요구하나」이지 「누가 쓰나」가 아니다(그 판정의 정본 = `crates/engram-dashboard-agent/src/backend/mod.rs:113-123` 의 doc).
- **Phase 1 의 PTY codex 경로는 그대로 산다** — 이 결정은 통로를 **더하는** 것이고 기존 것을 걷지 않는다. 같은 백엔드가 통로 둘을 갖게 되고, 그 둘의 권한 정책이 갈리는 문제는 **ADR-0188** 이 진다. ★단 그 「둘」이 **기동 argv 부터 갈린다** — §8 이 그 행을 진다★.

#### ★그 통로를 누가 만드나 — 가르는 switch 는 한 곳뿐이다★ (ADR-0191)

**이 저장소에는 「어느 backend 냐」를 가르는 switch 가 이미 하나 있다** — `backend_for(c)`(`crates/engram-dashboard-agent/src/backend/mod.rs:282`). ★**통로도 그 switch 가 정하고, 조립점은 한 번의 호출로 그 결과를 통째로 받는다.**★

- **오늘 조립점은 그 switch 를 국면마다 다시 탄다** — `spawn_agent` 이 `backend_caps` · `transport_shape` · `input_encoder` · `output_decoder` · `turn_classifier` · `reads_messages` 여섯을 각각 뽑는다(`crates/engram-dashboard-agent/src/manager.rs:1033-1043`). ★**그 여섯이 한 번의 호출로 접히고, 통로가 그 묶음에 함께 실린다**★. 「이 백엔드로 세션 하나를 세우는 데 필요한 것」이 한 자리에서 나오는 모양이다.
- ★**`select_transport` 의 중앙 `TransportShape` match 에 셋째 갈래를 더하지 않는다 — 기각됐다**★(`manager.rs:79-99`). 사유 둘: ① 가르는 자리가 **둘로 쪼개진다**(`backend_for` 와 그 match) ② 자기 주석이 ★「이 함수는 어느 backend 도 이름으로 모른다(ADR-0004)」★ 라고 적어 둔 함수가(`manager.rs:75-76`) **백엔드 타입을 이름으로 알게 된다.** 오늘 그 파일에 `codex` 라는 낱말은 **0회**다.
- ★★**없어지는 것은 그 match 이지 enum 이 아니다 — 셋째 `TransportShape` 변형은 그대로 선다**★★(ADR-0189 「영향」이 적은 그대로). ★**「갈래를 안 더하니 변형도 못 더한다」는 틀렸다**★ — ADR-0191 은 그 shape-keyed 생성 match 를 **codex 만이 아니라 모든 백엔드에서** 걷어 내므로, 비-exhaustive 가 될 match 자체가 **남지 않는다.** 즉 변형 하나의 값이 **0** 이다.
- ★**그래서 shape 는 ADR-0191 뒤로 「생성자를 고르는 스위치」가 아니라 순수한 선언 축이다**★ — 그 백엔드가 「내 통로는 무엇을 나르나」를 중립 어휘로 신고하는 값이다. ★★**단 「그 값에서 caps·렌더 판정이 내려온다」로 적지 말 것 — 4판의 그 문장은 인과를 지어낸 것이다**★★: ADR-0191 뒤로 **caps 주입값을 만드는 것도 같은 백엔드의 통로 생성 코드**이고(§5-5 의 세 문장), 프론트의 렌더 판정이 읽는 것은 **`output.structured` 한 칸**이지 shape 가 아니다(`src/components/slot/renderMode.ts:24` — ★프론트는 shape 를 아예 못 본다★). ★**즉 shape 와 caps 는 같은 백엔드가 **나란히 내는 선언 둘**이지 앞엣것이 뒤엣것을 낳는 관계가 아니다.**★ 그 둘이 어긋나게 선언돼도 컴파일러도 런타임도 안 잡는다 — ★**그것이 선언 표 트립와이어가 남아야 하는 이유이고**★, 「shape 만 맞추면 caps 는 따라온다」로 읽으면 그 그물이 지키려는 것을 놓친다. **오늘 그 축의 실물 소비자가 정확히 둘이고**(`rg transport_shape crates/ src-tauri/src/`, 비-테스트) 그중 생성 match 쪽(`crates/engram-dashboard-agent/src/manager.rs:1235` 인자 · `:1241` 호출)이 ADR-0191 로 은퇴하므로, ★**남는 유일한 독자가 선언 표 트립와이어다**★(`backend/mod.rs:800-802` — 「transport_shape 불일치 — 위 `expected_codec_axis` 를 따라 **의식적으로 선언할 것**」).
- ★★**그러니 변형을 지우면 그 트립와이어를 잃는다 — 그것이 이 판이 한 번 잘못 갈 뻔한 자리다**★★. 변형이 없으면 codex 는 `Pty` 를 계속 신고하고 **선언 표가 초록인 채로 남아**, 「이 백엔드의 통로 축을 의식적으로 선언했나」를 강제하던 그물이 사라진다. **enum 값 하나를 아끼고 회귀망을 파는 거래**라 성립하지 않는다. ★그리고 그것은 ADR-0189 의 「영향」 문장(「`transport_shape` 에 셋째 변형이 생긴다」)을 뒤집는 일이라 **TRD 가 임의로 할 일이 아니다**★.
- ★**선례가 이미 있다 — 새 모양이 아니다**★: `backend::output_decoder`(`backend/mod.rs:477`)가 `backend_for(c).output_decoder(c)` 한 줄이고, 그 doc 이 규율을 적는다 — 「판정도 decoder 실물도 [`AgentBackend::output_decoder`] 가 소유하고 이 함수는 dispatch 뿐이다 … **새 backend 는 자기 폴더에서 그 메서드를 구현하면 되고 이 함수는 손대지 않는다(교체성)**」. 통로도 같은 모양을 탄다.
- ★★**claude 경로의 값·순서는 그대로다 — 동작 변화 0 이 이 리팩터의 수락 조건이다**★★. 접는 것은 **뽑는 자리**이지 뽑히는 값이 아니다: 파이프 통로를 열며 `structured: true` 를 주입하는 것(`manager.rs:89`)도, 터미널 통로를 여는 것도, **하던 일이 그대로** 각 백엔드의 통로 생성 코드 안에서 일어난다.
  - ★★**단 「그 match 가 살아남아 이제 백엔드가 그것을 부른다」로 읽지 말 것 — 그러면 기각된 갈래가 되살아난다**★★. **shape 로 생성자를 고르는 match(`manager.rs:79-99`) 자체가 은퇴한다.** 백엔드는 **자기 통로를 shape 로 고르지 않는다** — 자기가 무엇을 여는지 이미 알기 때문이다(그것이 애초에 「가르는 switch 는 `backend_for` 하나」의 뜻이다). ★그 함수를 남겨 두고 백엔드가 shape 를 넘겨 부르면 **가르는 자리가 도로 둘이 되고**, 「이 함수는 어느 backend 도 이름으로 모른다」(`manager.rs:75-76`)를 지키려던 이유도 함께 사라진다★. **옮겨 가는 것은 그 두 갈래가 하던 *일*이지 그 match 가 아니다.**

**그 struct 가 소유하는 것 일곱:**

| 소유물 | 왜 여기인가 |
|---|---|
| 자식 프로세스 핸들 + Job Object | 프로세스 상태기계(§3)의 소유자가 통로라는 오늘 분할 그대로다. `StdioTransport` 가 같은 둘을 갖는다(`crates/engram-dashboard-agent/src/transport/stdio.rs:39` child · `:55` job_handle) |
| `Mutex<Option<ChildStdin>>` — **단일 writer** | §4-1 의 직렬화를 구현체 안의 뮤텍스가 진다. 오늘 그 모양이 이미 있다(`stdio.rs:40`) |
| **자기 읽기 스레드** | `AgentTransport::start(core)` 가 pump 를 **자기 안에서** 띄우는 것이 오늘 계약이다(`stdio.rs:209`). 그래서 writer 와 reader 가 한 객체 안에 있다 |
| ★**유계 큐 + 전용 writer 스레드**★ — §4-9·§4-10 | ★**4판이 §8 에만 적고 이 표에서 빠뜨린 소유물이다**★. 여기여야 하는 이유 = stdin 을 blocking 으로 잡는 스레드가 **이 구현체 안 하나**라야 읽기 스레드가 그것과 경합하지 않는다(§4-2 의 자기잠금). ★**그 종료 회계는 §5-7 이 진다 — 셋째 동사를 만들지 않는다**★ |
| 보낸 요청의 **응답 대기 맵** | §4-4 의 아웃바운드 축. 밖으로 나가지 않는다(아래 5-2) |
| 아웃바운드 **request id 카운터** | 같은 곳. 서버 id 를 이 카운터에 섞지 않는다(§4-4) |
| `thread.id` · **준비 상태** | 스레드 런타임 축과 연결 축(§3)이 여기 산다. 준비 상태는 §4-10 의 큐가 읽는 축이기도 하다 |

★**세션마다 하나씩 만들어지는 객체라는 것이 이 배치의 실질이다**★ — `AgentBackend` 는 codex 를 아는 유일한 층인데 **크기 0 짜리 unit struct 를 전 세션이 공유하는 함수 모음**이라 세션마다 기억할 자리가 없었고(`backend/mod.rs:277-279`), `AgentSession` 은 상태를 갖지만 codex 를 알면 ADR-0004 위반이었다. **통로 구현체는 그 둘 사이에 이미 있던 자리다** — 세션마다 새로 만들어지고, `backend/` 폴더 안에 살 수 있다.

### 5-2. 인바운드 분기는 그 struct 안에서 끝난다 — 네 경우

읽기 스레드가 받은 줄 하나를 **모양으로** 가른다. ★판정 축은 id 값이 아니다★ — 서버 요청 id 와 우리 요청 id 는 값 공간이 겹칠 수 있고(§4-4), 서버 응답은 `jsonrpc` 필드조차 생략한다`[실측]`.

| 받은 줄 | 무엇인가 | 무엇을 하나 |
|---|---|---|
| `method` **있고** `id` **있다** | 서버 → 클라 **요청** | 2a 실측 **0 건**(★조건 = 읽기 전용 + `approvalPolicy:"never"` · 선언은 11 종★)`[실측]`. ★**운영값은 그 조건과 반만 같다**★ — ADR-0192 가 고른 값이 **작업폴더 쓰기 + 안 묻기**라 승인 축은 그대로지만 **샌드박스 축이 넓어졌다**(§10-5). 그래서 이 행은 「안 온다」에 기대지 않는다: **오면 method-not-found 로 응답한다** — ★**버리면 상대가 멈춘다**★. 그 응답이 **성공을 위장하면 안 된다**(자동 승인이 되어 정책을 우회한다 — §6-2) |
| `method` **만** 있다 | notification | **번역 후 `core.emit`** — 오늘 pump 가 하는 그 일이다 |
| `id` **만** 있다(`result` 또는 `error` 동반) | 우리 요청의 **응답** | 대기 맵에서 꺼내 대기자를 깨운다. ★**밖으로 나가지 않는다**★ |
| JSON 파싱 실패 | 비-JSON stdout 라인 | **로그만 남기고 죽지 않는다**(§4-6). ★단 이 버전·이 조합에서는 stdout 에 비-JSON 줄이 **0** 이었다★`[실측]` — 그래도 fatal 로 두지 않는 것이 §4-6 의 판정이다 |

★**이 표가 1·2 판의 「인바운드를 어느 경로로 보나」 항목을 통째로 소멸시킨다**★ — 그 항목이 재던 셋((ㄴ) decoder 사설 채널 · (ㄷ) `OutputCore::subscribe` sink · (ㄹ) 세션 대기표 + 새 pump 간선)은 전부 **상관을 구현체 밖으로 내보내야 할 때만** 필요한 값이었다. 상관이 구현체 안에서 끝나면 **층간 간선이 0** 이다. → §10-14 는 닫혔다.

- ★**그래도 남는 배치 제약 하나**★ — **읽기 스레드가 응답을 기다리지 않는다.** 그 스레드가 유일한 reader 라(§4-2) 거기서 기다리면 읽기가 멈추고, 멈추면 서버 쪽 bounded queue 가 차서 `-32001` 로 돌아온다(§4-5). 그래서 읽기 스레드는 **대기자를 깨우기만** 하고 기다리는 쪽은 다른 스레드다. 이 제약은 통로 선택과 무관하게 성립하던 것이고 지금도 그대로다.

### 5-3. 앞서 막혀 있던 셋이 사라졌다 — 그리고 안 해도 되는 변경 하나

1·2 판이 「막힌 자리」로 적었던 셋은 **통로 구현체가 상태를 갖는 순간 전부 해소된다.**

| 막혀 있던 것 | 왜 사라졌나 |
|---|---|
| **쓰기 경로** — 「요청 봉투를 누가 만드나」 | 구현체가 **stdin 을 직접 소유**한다. 세션 층도 정적 인코더도 거치지 않는다 |
| **「응답이 안 왔다」를 표현할 자리** | `send_input` 이 **이미 `Result` 를 돌려준다**(`transport/mod.rs:45`). 새 반환 축이 필요 없다 |
| **취소** | `interrupt()` 가 **이미 공용 인터페이스에 있고**(`:51`) 이 구현체가 그것을 구현한다. ★`turn/interrupt` 는 실측 18ms 에 성공하고 같은 대화가 그대로 살아남는다★`[실측]` — 즉 취소가 프로세스 종료를 요구하지 않는다 |

★**그리고 파장이 가장 넓던 변경 하나가 통째로 불필요해진다**★ — 「입력 인코더를 세션마다 상태 갖는 객체로 바꾼다」. 그 안이 필요했던 이유는 `turn/start` 봉투에 셋(**세션별 상태** · **아웃바운드 id 카운터** · **오류 경로**)이 필요한데 오늘 인코더 seam 에 셋 다 없기 때문이었다:

| 사실 | 줄 |
|---|---|
| 계약이 나르는 것은 언제나 바이트다 — `InputEvent` 는 변형이 하나뿐이다(`Raw(Vec<u8>)`) | `crates/engram-dashboard-agent/src/types.rs:73-75` |
| `encoder` 는 `Copy` **태그**(`InputEncoder`)이고 정적 `dyn AgentBackend` **유닛 구조체 싱글턴**으로 dispatch 된다 | `backend/mod.rs:382-387`(enum) · `:296-301`(표) |
| 그 싱글턴의 시그니처가 전부다 — `fn wrap_input_turn(&self, text: &str, msg_uuid: Uuid) -> Vec<u8>` | `backend/mod.rs:198`(기본값) · `backend/claude/mod.rs:354`(claude 구현) |

★**그 셋을 인코더에 넣으려면 선언·호출 12 곳 + 다른 crate 의 실행 파일 하나 + 트립와이어 테스트를 함께 고쳐야 했다**★(ADR-0189 「거부한 대안」의 실측). **통로 구현체는 원래 세션마다 만들어지는 객체라 그 변경이 통째로 불필요하다.** ★이 표를 「인코더 seam 이 아직 문제다」로 읽지 말 것★ — 여기 남긴 이유는 **왜 그 seam 이 아니었나**를 다음 세션이 다시 재지 않게 하는 것이다.

- ★**단 `InputEncoder` 축이 아예 안 늘어난다는 뜻은 아니다**★ — 셋째 태그를 세울지, 아니면 이 통로에서는 인코더를 안 태울지가 §8 의 행으로 남는다. **컴파일러가 그것을 강제한다**: `InputEncoder::submit_sequence()` 의 match 가 exhaustive 다(`backend/mod.rs:443-446`). 태그를 더하면 그 arm 이 요구되고, 안 더하면 그 자리가 안 늘어난다. ★이 갈림은 통로 결정이 닫지 않았다 — §10-7 ②★. ★**`TransportShape` 쪽과 헷갈리지 말 것**★ — 그쪽은 **셋째 변형을 세우는 것으로 닫혔고**(ADR-0189 「영향」 · §5-1), 이쪽은 **세울지 말지가 열려 있다.** 두 enum 이 같은 자리에 있지 않다: shape 는 ADR-0191 뒤로 **선언 축 하나**라 변형을 더해도 요구되는 arm 이 없고(생성 match 가 은퇴한다), 인코더는 여전히 `Copy` 태그로 세션까지 흘러 `submit_sequence()` 의 exhaustive match 를 만난다(`session.rs` 가 그 태그로 `encode`·`input_echo_event` 를 부른다).

### 5-4. ★2 판 리뷰가 이 모양을 BLOCK 했던 이유와, 그 판정이 뒤집힌 근거★

**남겨 적는다 — 이 재해석이 이 절 전체를 세운 단 하나의 근거이기 때문이다.**

2 판 리뷰는 「codex 를 아는 transport 구현체」를 **TRD 가 정할 게 아니라 별도 결정이 필요한 행위**로 막았다. 근거로 인용된 것이 `transport/mod.rs:19-23`(이 문서 1·2 판의 인용은 `:20-24`)이었고, 그 문장이 **codex 를 이름으로 들어** 금지하는 것처럼 읽혔다.

> transport(StdioTransport)는 **바보 파이프**라 자식 stdout 바이트가 무슨 스키마인지(claude stream-json / codex 프로토콜 / 평문) 몰라야 한다.

★**원문의 주어는 `StdioTransport` 다**★ — **범용 파이프 구현체 하나**에 대한 서술이고, codex 는 그 파이프가 몰라야 하는 **스키마의 예시**로 나열됐을 뿐이다. 같은 취지가 `stdio.rs:7-9` 에도 있고 역시 **그 파일 자신**을 가리킨다. 그래서 두 규칙이 동시에 선다:

- **범용 파이프는 그대로 바보로 남는다** — `StdioTransport` 는 이번 결정에서 한 줄도 안 바뀐다.
- **스키마 지식은 `backend/codex/` 안에 있다** — ADR-0004 그대로다.

★**이 재해석은 리뷰 판정을 뒤집는 것이라 메인이 임의 확정하지 않고 사용자 판정을 받았다(2026-09-09).**★ ★그러니 이 절을 「세션이 주석을 다시 읽어 그렇게 판단했다」로 인용하지 말 것★ — 근거의 등급은 **사용자 판정**이고, 그것이 ADR-0189 의 상태 줄에 그대로 적혀 있다.

### 5-5. `transport_shape` 셋째 변형 — ★이제 **선언**이지 생성 스위치가 아니다★

**셋째 변형은 선다**(ADR-0189 「영향」). ★**바뀐 것은 그 값이 무엇을 하느냐다**★ — ADR-0191 이 shape-keyed 생성 match 를 걷어 낸 뒤로 그 값은 **생성자를 고르지 않고**, 그 백엔드가 「내 통로는 무엇을 나르나」를 중립 어휘로 신고하는 **선언**이 된다(§5-1). ★**그 선언이 caps·렌더 판정을 낳는 것이 아니다**★ — caps 도 같은 백엔드가 나란히 주입하는 **별개의 선언**이고 프론트는 `output.structured` 만 읽는다(§5-1 의 그 항). ★**shape 의 유일한 독자는 선언 표 트립와이어다**★ — 즉 이 값이 하는 일은 「그 백엔드가 통로 축을 **의식적으로** 선언했나」를 강제하는 것 하나다.

- **codex 백엔드는 오늘 통로 축을 하나도 선언하지 않는다** — `transport_shape` 는 trait 기본값 `Pty`(`backend/mod.rs:126`), `output_decoder` 는 기본값 `None`(`:219-221`). app-server 모드가 서면 **번역기가 붙고 통로 모양이 갈린다** — 두 축이 함께 움직이고, **둘 다 선언해야 한다.**
- ★**선언 표 트립와이어가 그것을 강제한다 — 그리고 셋째 변형이 서는 이유의 절반이 이것이다**★ — `expected_codec_axis` 의 codex 행이 오늘 `(InputEncoder::Raw, false, TransportShape::Pty)` 로 적혀 있고(`backend/mod.rs:779`), 그것을 확인하는 테스트가 `codec_axis_is_consciously_declared_for_every_backend` 다(같은 파일 · shape 단언은 `:800-802`, 메시지가 「**의식적으로 선언할 것**」이다). ★**같은 커밋에서 그 행을 갱신하지 않으면 그 테스트가 빨개진다**★ — 사고가 아니라 **의도된 마찰**이다(선언 축을 조용히 바꾸지 못하게). ★변형을 안 세우면 codex 가 `Pty` 를 계속 신고해 **이 그물이 초록인 채로 아무것도 안 잡는다**★.
- ★★**그런데 그 표에 구멍이 하나 있다 — 그 키가 `AgentCommand::Codex { .. }` 변형 하나이고 튜플도 하나다**★★(`backend/mod.rs:779`). 즉 **모드를 `AgentCommand::Codex` 안의 칸으로 나르면 둘째 모드는 그 검사를 아예 안 탄다** — 표는 변형당 한 줄이라 두 모드가 같은 줄로 접히고, 한 모드만 맞아도 초록이다. ★모드를 **별 변형**으로 세우면 표에 줄이 하나 늘어 검사를 탄다★. **그 갈림(칸이냐 변형이냐)이 §10-7 ①이고 데이터 위치라 사용자 결정이다** — 이 문서는 ★그 선택이 트립와이어 커버리지를 함께 정한다★는 사실만 못 박는다.
- **조립점은 shape 로 생성자를 고르지 않는다** — 백엔드가 만들어 넘긴 통로를 그대로 쓴다(§5-1 · ADR-0191). ★**그리고 「챗 표면에 그리나」의 답은 같은 자리에서 함께 정해진다**★ — codex 통로 생성 코드가 caps 에 `output.structured = true` 를 **실어 주면** 프론트가 `src/components/slot/renderMode.ts:24` 에서 `rich` 를 고른다. **프론트 코드 한 줄도 안 짜고 챗 표면에 얹힌다**(§7-3 · §10-4). ★**단 그것을 「shape 를 선언했으니 따라온다」로 읽지 말 것**★ — 따라오는 것은 **caps 를 주입했기 때문**이고 shape 는 그 옆줄이다(위 항). 없어진 것은 그 값을 읽던 **생성 match** 뿐이다.

#### capability 를 정직하게 신고하는 축 — ★레버가 어디 붙나: ADR-0191 이 닫았다★

`Capabilities` 의 `input`/`output`/`control` 은 **transport 소유**이고 backend 가 못 채운다(ADR-0030 · `crates/engram-dashboard-agent/src/types.rs:477` `Capabilities::compose`). 오늘 `StdioTransport::capabilities()` 는 `input{raw:true, message:false}` 와 `control{resize:false, interrupt:false, cancel:false, graceful_shutdown:false}` 를 **리터럴로 박아** 신고하고, 그 함수에서 주입으로 갈리는 칸은 `output.structured` 하나뿐이다(`transport/stdio.rs:363-384` · 필드 선언과 그 규율은 `:47-50`).

★**그 한 칸에는 명시 금지가 걸려 있다 — 구현체가 하드코딩하지 않는다**★. 두 자리가 그것을 적는다: 통로 필드 주석(`transport/stdio.rs:47-49`: 「★조립점 주입(ADR-0044/0030)★ … **하드코딩 금지** — 평문 stdio 엔 false」)과 조립점 doc(`manager.rs:69-73`: 「파이프 자체는 내용을 모르므로(통로 무정제 불변) StdioTransport 는 structured 를 하드코딩하지 않고 이 지점의 주입값을 받아 caps 로 신고한다」). ★**그 값이 화면 분기(`renderMode.ts:24`)와 우편 프록시(`LiveAgent.turn_signal` — `crates/engram-dashboard-daemon/src/messaging_host.rs:104`) 둘 다로 흐르므로 값싸게 다룰 칸이 아니다.**★

★**판정(ADR-0191 이 닫았다) — 세 문장이고, 이 문서의 네 자리가 전부 이 문장을 말한다**★(§5-5 = 여기 · §7-2 · §8 · §10):

1. ★**통로 구현체는 caps 를 하드코딩하지 않는다.** 생성 시 주입받아 그대로 신고한다★ — 오늘 `StdioTransport` 의 규율 그대로이고, 새 struct 도 예외가 아니다.
2. ★**그 주입값을 만드는 것은 그 백엔드의 통로 생성 코드다**★(`backend/codex/` 안). 「이 통로가 구조화 스트림을 나르나 · 키 입력 바이트를 받나 · 인터럽트가 되나」는 **백엔드/모드 지식**이고, ADR-0044/0030 이 금한 것은 **파이프가 스스로 그것을 정하는 것**이지 백엔드가 정해 주입하는 것이 아니다(조립점 doc 자신이 「claude `--output-format`(backend/mode 지식)이 정한다」로 그렇게 적는다).
3. ★**조립점은 그 값을 만들지도, 통로의 구체 타입을 알지도 않는다.**★

- ★**3판이 §10-11-a 로 열어 둔 「레버가 어디 붙나」는 이것으로 닫힌다**★. 3판의 어긋남은 「생성자 인자를 넓힌다」가 `StdioTransport::open` 을 가리키는지 새 struct 의 생성자를 가리키는지 모호했던 것인데, ADR-0191 뒤로는 **둘 다 답이 아니다** — 레버는 **백엔드 폴더 안의 통로 생성 코드**이고, `StdioTransport::open` 은 claude json·평문 경로 것으로 **한 글자도 안 바뀐다**(codex 세션은 그 파일을 지나지 않는다).
- **그래서 codex 세션의 신고는 이렇게 선다** — `input{raw:false, message:true}` · `output{structured:true, …}` · `control{interrupt:true}`. ★값이 무엇인지는 정직성 문제이고, 그것을 **누가 만드나**가 위 셋이다.★
- ★**「caps 한 칸을 바꾸려고 `TransportShape` 변형을 또 세우지 않는다」는 그대로 유효하다**★ — 셋째 변형이 서는 것은 **통로 종류가 실제로 다르기 때문**이고(ADR-0189), 그 위에서 값이 갈리는 것은 **주입**으로 흡수한다. ★넷째를 부르는 사유가 「caps 한 칸」이면 그건 주입으로 가야 한다★.
- ★**`interrupt()` 가 처음으로 실제 동작하는 구현체가 생긴다**★ — 오늘 `StdioTransport::interrupt` 는 `Unsupported` 다(`transport/stdio.rs:316-321`). 그 반대편(사람이 누를 표면)은 **여전히 없다** — 트리 노드가 `canInterrupt` 를 만들지만 그 값을 읽는 화면 코드가 하나도 없다(`src/components/agent/mergeTreeNodes.ts:55` 선언 · `:94` 대입 · `:111` 예약 기본값 세 줄이 전부, 비-테스트 실측). ★그 절반은 이 문서의 범위가 아니고 **T-34** 가 진다★(`docs/tracking.md`).

### 5-6. 대화 id 는 조립점이 준 콜백으로 프로필에 기록한다

★**backend 가 `ProfileRegistry` 를 직접 부르지 않는다.**★ 오늘 그 레지스트리는 `backend/` 트리에서 보이지 않고(`rg "ProfileRegistry|profiles" crates/engram-dashboard-agent/src/backend/` = **0줄**, 비-테스트) 밖에서 얻을 수도 없다(`AgentManager` 에 `pub fn profiles()` 가 없다 — `turns()` `:537` · `presets()` `:541` 은 있다).

- **선례가 그대로 있다** — claude sid 기록 경로가 데몬 조립점에서 클로저 하나를 만들어 내려보낸다: `Arc::new(move |agent_id, new_sid| { profiles_cb.observe_session_id(agent_id, new_sid); })`(`crates/engram-dashboard-daemon/src/lib.rs:284-290`). ★**「이 에이전트의 백엔드 세션 id 를 기록한다」 동사 하나만 든 콜백**이고 레지스트리 자체가 아니다★.
- **그래서 §6-3 의 「그 구현체가 프로필에 손이 닿나」는 이 콜백으로 닫힌다** — 통로 구현체를 만드는 자리가 그 콜백을 함께 건넨다. ★단 **읽는 쪽**(「이미 persist 됐나」)까지 그 콜백이 덮는지는 이 결정이 정하지 않았다★ — 선례는 **쓰기 한 방향**뿐이다. §10-16 이 그 잔여를 진다.
- ★**콜백 이름이 중립이어야 한다**★ — 「codex 드라이버용」이면 조립점이 백엔드를 알게 되어 ADR-0004 가 샌다. 위 선례의 이름이 그 규율의 실물이다.

### 5-7. 이 결정이 **안 건드리는** 것 — 불변식 점검

ADR-0189·0191 「근거」의 점검 그대로이고, 이 문서가 그것을 확인한다.

- **kill 인과 2 동사**(ADR-0001) — `transport.shutdown()` → `core.join_pump(5s)`. ★**읽기 스레드를 그 struct 가 소유하므로 `shutdown()` 이 그것까지 정리한다 — 셋째 동사를 만들지 않는다**★.
  - ★★**그런데 이 통로의 스레드는 둘이다 — writer 스레드가 그 회계 어디에 드나를 4판이 안 적었다**★★. 줄로 적는다.
    1. ★**둘째 동사가 기다리는 것은 읽기 스레드 하나뿐이다**★ — `OutputCore` 는 pump 핸들과 done 채널을 **하나씩만** 갖고(`crates/engram-dashboard-agent/src/output_core.rs:68-69`, 적재는 `attach_pump` `:395-397`), `join_pump` 는 그 done 채널 하나를 기다린다(`:383-392`). ★**거기에 둘째 핸들을 끼워 넣지 않는다**★ — 그 자리는 한 칸이고, 늘리는 것은 코어 변경이다(§3 이 금한 부류).
    2. ★**writer 스레드는 첫째 동사가 인과로 끝낸다**★ — `shutdown()` 의 kill + `TerminateJobObject` 가 **파이프를 깨서 블록된 `write_all` 이 에러로 풀린다**. 그 인과는 이 저장소가 이미 문장으로 적어 둔 것이다: 「자식을 죽이면 파이프가 깨져 블록된 write_all 이 에러로 풀리고 락이 해제된다」(`crates/engram-dashboard-agent/src/transport/stdio.rs:330-337`, 회귀망 `:457`). 그 뒤 writer 는 shutdown 플래그를 보고 루프를 끝낸다.
    3. ★**`shutdown()` 은 그것을 기다리지 않는다 — 기다리면 계약 위반이다**★. 계약이 「자원 강제 종료(멱등). **pump 종료 대기는 여기서 안 함**」이다(`crates/engram-dashboard-agent/src/transport/mod.rs:53`), 그리고 §4-8 이 그 안의 양수 대기가 5초 예산 **앞에 더해진다**는 회계를 이미 적었다.
    4. ★**선례가 이미 있다 — 새 모양이 아니다**★: 오늘 `StdioTransport` 도 pump 말고 **stderr drain 스레드**를 하나 더 띄우는데(`stdio.rs:167-194`) 그것을 기다리는 동사가 어디에도 없다. **「transport 소유 스레드가 둘 이상이고 둘째는 인과로만 끝난다」가 이미 이 저장소의 모양이다.**
    - ★**대가 하나 — 아무도 그 스레드를 기다리지 않으므로 큐에 남아 있던 미전송 입력은 조용히 사라진다**★. kill 경로에서는 그것이 맞는 결말이지만(세션이 끝난다), ★그 사실 자체는 어디에도 안 보인다★ → §11.
- **finalize 1회**(ADR-0005) · **락 순서**(ADR-0006) · **턴 관측 정리 두 지점**(ADR-0127 결정 5) — 소비자를 늘리지 않는다.
- ★**백엔드 이름 격리**(ADR-0004) — 이 판에서 그 확인이 **실물로 바뀌었다**★. 3판은 「스키마·메서드 이름·승인 어휘가 전부 `backend/codex/` 안에 남는다」로 적었는데, 그때 함께 처방한 「`select_transport` 에 셋째 갈래 한 줄」이 **바로 그 격리를 깨는 줄**이었다: 그 함수는 자기 주석에 ★「이 함수는 어느 backend 도 이름으로 모른다(ADR-0004)」★ 라고 적어 두었고(`crates/engram-dashboard-agent/src/manager.rs:75-76`) 오늘 그 파일에 `codex` 는 **0회**인데, 셋째 갈래는 그 자리에서 codex 통로 타입을 이름으로 부르게 된다. ★**ADR-0191 이 그 줄을 걷었으므로 이제 이 항목은 점검이 아니라 사실이다**★ — 통로 타입 이름이 `backend/codex/` 밖으로 나가지 않는다.
- **소유권 분할** — transport = master/writer/child/shutdown/job · core = subscribers/replay/seq/status/finalized · session = id/cwd/epoch/cols/rows. ★이번 결정들은 그 분할을 바꾸지 않고 **transport 칸 안에서** 소유물이 늘 뿐이다★.
- ★**claude·shell 경로의 동작**★ — ADR-0191 이 접는 것은 **조립점이 값을 뽑는 자리**이지 값 자체가 아니다. 파이프 통로를 열며 `structured: true` 를 주입하는 일도, 터미널 통로를 여는 일도 **값·순서 그대로** 백엔드 쪽에서 일어난다(§5-1 의 그 항). ★**단 shape 로 생성자를 고르는 match 는 남지 않는다 — 「그 함수를 백엔드가 부른다」가 아니다**★(같은 항). ★**동작 변화 0 이 수락 조건이라는 것을 §9-3 이 단언으로 진다**★.

### 5-8. ★1·2 판이 틀린 방식 — 남겨 둔다★

**두 번 같은 방식으로 틀렸다. 그 방식을 여기 남겨 다음 세션이 반복하지 않게 한다.**

| 판 | 권고 | 그것이 서 있던 사실 문장 | 어디서 무너졌나 |
|---|---|---|---|
| 초판 | 통로를 재사용하고 decoder 에 되쓰기 seam 을 단다 | 「`turn/start` 는 그 seam 에 그대로 맞는다」 | **그 seam 에 세션 상태·아웃바운드 id·오류 경로가 셋 다 없다**(§5-3 의 표). 승인만 막히는 게 아니라 **어떤 요청도** 못 낸다. 그리고 되쓰기 핸들은 **출처가 없다** — decoder 실물을 만드는 유일한 자리가 상태 없는 싱글턴이고 값을 하나만 돌려준다(`backend/mod.rs:219-221` · 조립점 `manager.rs:1039`) |
| 2 판 | transport 위에 층 셋(RPC 피어 · 드라이버 · 순수 번역기) | 「§4-9 가 요구하는 유계 writer 를 그 피어가 **이미** 갖는다」 + 「통로를 재사용하므로 값이 싸다」 | 그 writer 는 **네 안 모두에** 소유자가 있어 한 안의 이점이 아니었고(§4-9 의 표), 「재사용이라 싸다」는 **아웃바운드 축에서만** 참이었다 — 인바운드에서는 그 안이 값을 새로 냈다 |

★**두 실패의 공통 모양 = 「층 이름」 수준에서 그림을 그리고 그 위에 「이 경로는 이미 있다」를 얹은 것.**★ ADR-0189 가 그것을 깬 방법은 그림이 아니라 **소유권 추적**이었다 — 어느 객체가 무엇을 쥐고 언제 죽나를 줄로 세어(정본 = `docs/reference/structure/session-path-ownership.md`, ★쓸모 있는 부분은 PART B·C★) 「codex 를 알면서 세션마다 상태를 갖는 것」이 놓일 자리가 **이미 있었다**는 것을 확인했다.

★**그리고 이 절이 닫지 않는 것 하나**★ — 이 값들 전부가 **승인 왕복의 실측 앞에서 다시 매겨진다.** 승인이 「우리가 서버 요청에 응답」이라는 것까지가 `[문서]`이고(D1) 그 왕복을 돌려 본 적이 없다 — ★2a 실측에서 서버 요청은 **0 건**이었고, 그 0 은 「읽기 전용 + `approvalPolicy:"never"`」 조건이 만든 것이라 **조건이 바뀌면 그 면제가 죽는다**★(ADR-0187 「영향」). §11.

## 6. 번역기 — codex 이벤트 → 중립 어휘

**우리 중립 어휘는 이미 있다** — wire `StructuredEvent`(정본 = `crates/engram-dashboard-protocol/bindings/StructuredEvent.ts`, agent `OutputEvent` 의 충실한 미러, ADR-0045): `TextDelta{text,turn_id,message_id}` · `ToolCall{name,args_json,id,turn_id,message_id}` · `Usage{input_tokens,output_tokens,turn_id}` · `MessageDone{turn_id,message_id}` · `Error{message}` · `Structured{kind,json}`. 여섯이 전부다.

★**어휘를 늘리기 전에 이 표를 채운다**★ — 늘리는 것은 wire 변경이고 §10-6 에 매달린다.

### 6-1. 번역 표 — 무엇을 잃고 무엇을 지어내나

★**1·2 판의 이 머리글은 「왼쪽 열 전부가 `[스키마]` 이고 오른쪽 결말은 아직 아무것도 실측이 아니다」였다. 두 문장 다 죽었다**★ — 2026-09-09 실측 뒤 왼쪽 열의 **여러 행이 `[실측]`** 이다(`item/agentMessage/delta` = L6 · 턴 중 알림 11 종 = L7 · `turn/interrupt` 와 그 뒤 대화 생존 = L9 · 중단된 `item/started` 가 안 닫히는 것 = L10). ★**아래 표의 등급 열이 정본이고 이 머리글은 개수를 세지 않는다**★.

★**그래도 표 전체가 실측 위에 서지는 않는다**★ — 등급이 `[스키마]`·`[미확인]` 인 행은 여전히 **판독뿐**이고, 그 행들에는 1·2 판의 판정이 그대로 걸린다: **표가 틀리면 번역기가 아니라 표부터 다시 짠다**(§11). 그리고 ★**오른쪽 「우리 어휘」 열은 아직 어느 행도 실측이 아니다**★ — 번역기를 아직 안 짰기 때문이다.

★**그리고 `[스키마]` 행의 출처는 조사 보고서 F8 하나다 — 그 보고서가 적지 않은 것은 이 표도 모른다**★. 아래에서 「보고서가 열거하지 않았다」로 적힌 칸은 추측으로 채우지 않았다.

| codex 이벤트 | 등급 | 우리 어휘 | 잃는 것 · 지어내는 것 |
|---|---|---|---|
| `item/agentMessage/delta` | `[스키마]` | `TextDelta` | 손실 없음. ★단 `turn_id`/`message_id` 를 **채울 수 있다**★ — codex 는 turnId 를 갖고 있다. **claude 는 그 칸이 항상 None 이다**(`src/components/slot/structuredAccumulator.ts:130-132`) → 채우면 프론트가 처음으로 그 칸에 값을 보게 된다(§7 갭 1) |
| `item/reasoning/textDelta` · `item/reasoning/summaryTextDelta` | `[스키마]` | ★**대응 variant 가 없다**★ | 우리 어휘에 「추론」 축이 없다. 갈림 셋: ① `TextDelta` 로 접어 본문에 섞는다(추론과 답을 화면이 구별 못 한다) ② 어휘를 늘린다(§10-6) ③ **버린다**(추론은 진단 가치가 있고 사용자도 보고 싶어 하므로 이것이 공짜가 아니다). ★**「`Structured` 로 흘린다」는 §6-2 가 닫았다**★ |
| `item/commandExecution/outputDelta` | `[스키마]` | ★**없다**★ | 명령 **결과** 스트림. `ToolCall` 은 **호출**이지 결과가 아니다 — 우리 어휘에 도구 결과 축이 없다. 위와 같은 갈림(§10-6) |
| `item/started` / `item/completed` (`ThreadItem` **20 변형 union**) | `[스키마]`(개수) + ★`[미확인]`(목록)★ | 일부 → `ToolCall`, 나머지 미정 | ★**조사 보고서는 그 20 변형을 열거하지 않는다 — 「20-variant union」이라고만 적는다**★(F8). 이 세션도 상류 스키마를 다시 열지 않았다. **그래서 이 행은 지금 채울 수 없고, 비운 채로 두지도 않는다** → ★`ThreadItem` 변형 열거를 §9-1 첫 게이트의 항목으로 올린다★. 열거 없이 번역기를 짜면 어느 변형이 조용히 버려지는지 아무도 모른다 |
| `turn/started` | `[스키마]` | **턴 시작 어휘가 없다** | claude 에선 「새 유저 턴」이 `Structured{kind:"user"}` 로 와서 프론트가 그것으로 `turnDone` 을 내린다(`structuredAccumulator.ts:110-123`). codex 는 그 모양으로 오지 않는다 → **대기 인디케이터가 안 뜬다**(§7 갭 1). ★그 관례를 계약으로 박는 것이 §10-2 (가)다★ |
| `turn/completed` — `TurnStatus = completed` | `[스키마]` | `MessageDone` | 맞는 짝이다. ★**턴마다 정확히 1회**라는 계약을 여기서 지켜야 한다★ — `item/completed` 마다 내면 한 턴이 여러 경계로 쪼개진다(§7-1 의 3) |
| `turn/completed` — `TurnStatus = failed` | `[스키마]` | 부분: `Error{message}` | `MessageDone` 으로만 접으면 **화면이 「정상 종료」로 읽는다.** `Error` 를 함께 내면 표시는 되지만 **턴 경계와 실패가 두 이벤트로 갈려** 순서 계약이 하나 더 생긴다 |
| `turn/completed` — `TurnStatus = interrupted` | `[스키마]` | ★**없다**★ | 「사용자가 끊었다」를 나타낼 variant 가 없다. `Error` 로 접으면 실패로 보이고, `MessageDone` 으로 접으면 완료로 보인다. **둘 다 거짓이다** → §10-6 |
| `turn/completed` — `TurnStatus = inProgress` | ★`[미확인]`★ | — | 네 값 중 이것이 `turn/completed` 에 실려 오는 조합이 **무슨 뜻인지 모른다**(그 enum 은 상태 값이라 다른 자리에서도 쓰이는 것으로 보이지만 보고서가 갈라 적지 않았다). ★**추측해서 매핑하지 않는다**★ — §9-1 에서 실물을 보고 채운다 |
| `thread/status/changed` — `ThreadActiveFlag`(`waitingOnApproval` · `waitingOnUserInput`) | `[스키마]` | **화면 어휘 없음 — 코어가 읽는다** | 이 둘은 **이벤트가 아니라 상태**다. ★그런데 §3 턴 상태기계의 입력이다★ — 화면에 안 그리더라도 통로 구현체는 알아야 한다(승인 대기 중인 턴을 「멈춘 턴」으로 오판하지 않기 위해). 즉 「번역해서 흘린다」와 「통로 구현체가 읽는다」가 갈리는 행이고, ★**흘리지 않는 쪽이 기본이다**★ |
| `thread/status/changed → notLoaded` · `thread/closed` | `[문서]`(D4) | **없음 — 우리 기계 밖** | §3 이 그 셋을 우리 상태기계에서 뺐다(`thread/unsubscribe` 호출자가 없다). 관측하면 **로그만** 남긴다 |
| `thread/tokenUsage/updated` | `[스키마]` | `Usage` | codex 가 그 두 값(입력·출력)을 그 모양으로 주는지 `[미확인]`. 캐시 토큰·추론 토큰 같은 칸이 더 있으면 **잃는다** |
| 승인 요청 다섯 (D1) | `[문서]` | **없다 — 그리고 있으면 안 된다** | 이건 출력 이벤트가 아니라 **답해야 하는 요청**이다. ★번역기가 이것을 `StructuredEvent` 로 흘려 화면에 그리기만 하면 에이전트가 멈춘다★. ★**2a 의 처분은 「안 오게 하는 것」과 「그래도 오면 답하는 것」 둘이다**★ — 전자 = `thread/start` params 의 `approvalPolicy` 를 **안 묻기**로 준다(ADR-0192 · §10-5), 후자 = `method`+`id` 가 함께 온 줄에는 **유계 오류 응답**을 돌려준다(§5-2 첫 행 · §6-2). ★1·2 판이 여기 적은 「§10-5 정책 하나로 답한다」는 낡았다 — 그 항목은 정책값을 **argv 로** 상상했고 이 통로에서는 params 다★ |
| `deprecationNotice` | `[문서]`(D2) | **없음 — 로그로만** | §4-7 의 2 가 그 처분이다. ★**초판은 이것을 `Structured` 탈출구로 보냈는데 걷었다**★ — 사유는 §6-2 |
| 모르는 알림 이름 (버전 드리프트) | `[미확인]` | **없음 — 로그 + 버림** | ★초판은 이것도 `Structured` 로 보냈다. 걷었다★ — §6-2 가 그 이유와 대가를 적는다 |
| 위에 안 든 나머지 알림 (**81 종** 중) | ★`[미확인]`★ | 미정 | 보고서가 열거한 것이 위 목록이고 **81 종 전체 명단은 우리에게 없다**. 그 나머지가 무엇인지 아는 것도 §9-1 항목이다. ★**1·2·3 판이 여기 적은 「83 종」은 조사 보고서 숫자이고 낡았다**★ — 로컬 스키마 생성이 돌려준 선언 수가 `ServerNotification` **81 종**이다(L14). 이 문서의 규율대로 **실측이 이긴다** |

★**「어휘를 늘린다」가 §10-6 이고 이 문서는 고르지 않는다**★ — 위 표에서 §10-6 로 넘어간 축은 셋이다: **추론** · **도구 결과** · **턴 결말(interrupted)**. 무엇이 늘어야 하는지는 §10-6 행이 진다.

### 6-2. `Structured{kind,json}` 탈출구 — ★기본 배출구로 쓰지 않는다★

**먼저 실측 하나 — 초판의 처방은 화면에 codex 프로토콜 낱말을 띄운다:**

| 사실 | 줄 |
|---|---|
| 누산기가 `Structured` 를 받으면 `kind` 를 **`label` 로 그대로 실어** 항목에 넣는다 — 「알 수 없는 종류(kind)도 흘려 유실 방지」 | `src/components/slot/structuredAccumulator.ts:125-126`(변형 선언 = `:27-28`) |
| 렌더러가 `thinking` 이 아닌 label 을 전부 `GenericItemRow` 로 그린다 | `src/components/slot/StructuredTextView.tsx:397-400` |
| 그 행은 **label 을 monospace 로 화면에 찍고**, 펼치면 **JSON 원본을 코드블록으로** 보여 준다 | 같은 파일 `:279-304`(label = `:295` · JSON = `:299`) |

★**그래서 「모르는 알림 이름을 `Structured` 로 흘린다」는 곧 `item/reasoning/summaryTextDelta` 같은 codex 메서드 이름과 그 payload 를 사용자 화면에 띄우는 것이다.**★ 그것이 정확히 이 프로젝트가 금한 것이고(`trd.md` §6-1 사용자 결정 — 「번역기가 덜 채워져서 원본 JSON 이 화면까지 가는 것」), 초판이 §4-7 의 3 과 §6-1 의 마지막 두 행에서 그것을 **처방**했다.

**고친 규율 — 배출구는 셋으로 갈린다:**

| 무엇이 왔나 | 어디로 | 화면에 뜨나 |
|---|---|---|
| 아는 이름 · 아는 모양 | 번역표(§6-1)의 중립 어휘 | ✓ 중립 어휘로 |
| **모르는 알림 이름**(= `method` 만 있고 `id` 가 없는 줄) | ★**로그 + 버림**★ — `deprecationNotice` 도 여기 | ✗ |
| ★**모르는 인바운드 요청 이름**(= `method` **와** `id` 가 함께 있는 줄)★ | ★**버리면 안 된다 — 유계 오류 응답을 돌려준다**★. 아래 항이 그 근거다 | ✗ (화면 어휘는 여전히 없다) |
| 아는 이름인데 **모양이 다르다** | 로그 + 버림. ★**이건 결함이다**★ — 조용히 관용하면 §4-7 이 경고한 침묵이 된다. ★**단 그 줄에 `id` 가 있으면 위 행이 이긴다 — 버림 전에 응답을 낸다**★ | ✗ |
| `Structured{kind,json}` | ★**우리가 그 `kind` 를 알고, 화면에 그리기로 정한 것만**★ | ✓ 정한 모양으로 |

★**「로그 + 버림」을 알림과 요청에 똑같이 적용하면 에이전트가 멈춘다 — 초판은 이 둘을 안 갈랐다**★

- **알림은 버려도 된다.** 아무도 답을 기다리지 않는다.
- ★**요청은 버리면 영구 정지다.**★ 승인이 그 부류라는 것을 §6-1 마지막 줄과 §5-1 이 이미 적었는데(D1 — 답하지 않으면 에이전트가 멈춘다), ★그 판정은 **우리가 아는 다섯**에만 걸려 있었다★. 상류가 여섯째 요청을 신설하면(D5 가 그 선례를 보인다 — `newConversation` → `thread_start`) 그것이 「모르는 이름」으로 들어와 위 행의 「버림」을 타고, **그 에이전트는 그 자리에서 멈춘다.** 우리 화면에는 아무것도 안 뜨고 로그 한 줄만 남는다.
- **그래서 규율:** ★**받은 줄에 `method` 와 `id` 가 함께 있으면, 그 이름을 모를 때도 그 `id` 로 오류 응답을 한 번 돌려준다.**★ 내용은 「이 메서드를 모른다」 하나로 족하다 — 우리가 정의하는 봉투이므로(S1) 오류 코드도 우리가 고른다. 그 응답이 성공을 위장하면 안 된다(자동 승인이 되어 §10-5 의 정책을 우회한다).
- ★**이것은 §4-4 의 판정 축과 같은 축이다**★ — 「`method` 유무로 대기표 조회를 게이트한다」의 짝이 「`id` 유무로 응답 의무를 게이트한다」다. 두 축을 한 곳에서 판정하지 않으면 하나는 반드시 어긋난다.
- ★**대가:** 그 오류 응답 자체가 `[미확인]` 위에 선다★ — app-server 가 모르는-메서드 오류를 어떻게 받아들이는지(그 턴을 실패로 접나 · 무시하나) 우리가 본 적이 없다. **그래도 「버림」보다는 낫다** — 버림의 결말은 영구 정지이고 §4-5 와 같은 판정을 받는다. §11 에 남긴다.

★**판정 한 줄(그대로 유효):** `Structured` 로 흘린 것을 프론트가 `JSON.parse` 해야 화면이 맞는다면, 그것은 탈출구가 아니라 미룬 번역이다.★ `Structured.json` 은 **문자열**이라 프론트가 그것을 파싱하면 **그 순간 프론트가 codex 스키마를 알게 된다.** 그것이 오늘 claude 에서 이미 일어난 일이다(`extractUserUuid` 가 claude replay 라인의 uuid 를 파싱한다 — `structuredAccumulator.ts:111`).

**그러면 §4-7 이 걱정한 침묵은 무엇이 깨나 — 대가를 정직하게 적는다:**

- 초판의 논증은 「모르는 이름이 화면에 원본으로라도 보이면 침묵이 깨진다」였다. **그 신호를 화면에서 로그로 옮긴 것이 이 변경의 대가다** — 사용자는 안 보고, 그것을 보는 사람은 로그를 여는 사람이다.
- ★**그래서 로그 한 줄로 끝내지 않는다**★: ① 모르는 이름은 **이름별로 1회만** 남긴다(스팸 방지) ② 그 카운터를 진단에서 볼 수 있게 한다 ③ 마스킹 경로를 그대로 탄다(`transport/stdio.rs:24` 가 `engram_dashboard_base::logging::mask_secrets` 를 import 해 쓴다).
- ★**그리고 §9 가 그 침묵을 자동으로 잡는다**★ — 번역 표 골든이 「아는 이름 전부가 중립 이벤트를 낸다」를 단언하므로, 상류가 이름을 바꾸면 **골든이 빨개진다.** 화면 관찰보다 이쪽이 이르다. (그 골든은 우리가 아는 이름만 덮으므로 **새로 생긴 이름**은 여전히 로그가 유일한 신호다 — 그 한계를 남긴다.)
- ★**§10-6 을 「안 늘린다」로 고르면 §6-1 의 세 축(추론·도구 결과·interrupted)이 이 규율 아래에서 「버림」이 된다**★ — 즉 그 결정의 대가가 「화면에 원본이 보인다」에서 「화면에 안 보인다」로 바뀐다. 그 갈림을 §10-6 행이 함께 진다.

#### ★그리고 「그럼 화면으로 보내면 되지 않나」의 답 — **프론트에는 미지를 받는 문이 애초에 없다.** 픽셀까지 추적했다★

이 셋은 위 결정을 **보강**한다. 「로그로 보낸다」가 화면을 포기하는 것처럼 읽히는데, ★실제로는 화면 쪽에 보낼 문이 없다.★

1. ★**미지 최상위 `StructuredEvent.type` 은 그냥 사라진다 — 그런데 스피너까지 데려간다**★. 누산기의 `consume()` switch 에 ★**`default:` arm 이 없다**★(`src/components/slot/structuredAccumulator.ts:66-138`). 안 맞는 `type` 은 항목을 안 넣고 · `nextId` 도 안 올리고 · `turnDone` 도 안 건드리고 · throw 도 warn 도 카운터도 없다. **그런데 호출자는 무언가 온 것처럼 행동한다**(`src/components/slot/RichSlot.tsx:153-167`): seq 하이워터가 먼저 전진하고(`:156`) · `setItems([...acc.snapshot()])` 가 **내용이 같은 새 배열 참조**를 만들어 리렌더 + 자동 스크롤을 발화하고(`:165` · deps `[items]` 인 스크롤 effect `:221-224`) · ★**`setAwaiting(false)` 가 「첫 토큰 대기」 인디케이터를 끈다**(`:167`)★. ★**픽셀 결말 = 「화면은 이전과 한 픽셀도 다르지 않은데 스피너만 사라졌다」**★. 그리고 릴리스 WebView2 에는 devtools 가 없어 `console` 도 못 본다(그것이 `ConnectionNotice` 가 존재하는 사유다 — `src/components/layout/ConnectionNotice.tsx:3-5`). ★**즉 「우리가 이 이벤트 모양을 모른다」와 「에이전트가 아무 출력도 안 냈다」가 화면에서 똑같이 보인다.**★
2. ★**새 프레임 태그는 클라이언트가 보기도 전에 조용히 버려진다**★ — `src/api/wsFrame.ts:35` 가 `0`·`1` 외를 전부 `null` 로 떨어뜨리고(255 는 별 경로) 두 carrier 가 `if (!f) return` 한다(`src/api/tauriTransport.ts:339` · `src/api/wsTransport.ts:294`). 로그도 카운터도 없다. ★**그 줄을 고치기 전까지 `tag=2` 스트림은 「한가한 에이전트」와 구별되지 않는다.**★ → 즉 §10-6 을 「새 태그를 만든다」로 읽는 갈래는 **프론트 한 줄을 함께 고치는 일**이고, 그 줄은 §8 「고치는 것」에 없다(§10-6 이 열릴 때 든다).
3. ★**있는 문은 하나뿐이고 그것은 `Structured.kind` 다**★ — 모르는 `kind` 는 `GenericItemRow` 로 **label + JSON 원본**을 그린다(`structuredAccumulator.ts:125-126` → `StructuredTextView.tsx:397-400` → `:279-304`). ★**그래서 「미지를 화면으로」의 유일한 실현 경로가 곧 이 절이 금지한 것이다**★ — 문이 하나뿐이고 그 문이 codex 메서드 이름을 화면에 찍는다. 둘을 동시에 가질 수 없다.

★**결론: 「로그로 보낸다」는 화면을 포기한 선택이 아니라, 화면 쪽에 미지를 안전하게 받는 문이 없어서 남는 유일한 선택이다.**★ 문을 내려면 (1)의 `default:` arm 이나 (2)의 태그 통과 경로를 새로 짜야 하고, **둘 다 이 문서의 범위 밖**(Phase 3 프론트 회수)이다. → 그 사실을 §11 에 남긴다.

**어휘 소유(그대로):** `kind` 문자열은 백엔드가 정하고, 그 해석 선언은 Rust `turn_classifier`(`backend/mod.rs:143`)가 진다. **프론트에 `kind` 별 분기를 만들지 않는다.** 그 근거는 그 함수 doc 이 이미 적는다 — 「공용 층에서 해석하면 한 백엔드의 관례가 전원에게 강제된다」(`backend/mod.rs:130-137` · ADR-0113).

### 6-3. ★순서 제약 — 받은 thread id 는 첫 턴을 허용하기 전에 persist 된다★

ADR-0185 「영향」의 Phase 2 요구사항이고, **그 ADR 이 「아직 불변식이 아니다 — 성립한 불변식으로 인용하지 말 것」으로 도장 찍었다.** 여기서 설계가 가능해지는 이유 = **2a 가 그 응답을 소유하는 유일한 단계**라는 것.

```
thread/start 요청 전송
      ↓
ThreadStartResponse 수령 → thread.id                       [스키마 S3 · 실측 0]
      ↓   ★★★ 이 구간이 위험 창 ★★★  (무엇이 위험한지는 바로 아래 — 「못 잇는다」가 아니다)
ProfileRegistry::observe_session_id(agent_id, thread.id)   profile.rs:629
      ↓   (mutate_if → 변경 즉시 persist — 그 함수 doc :626-628)
여기서부터 turn/start 를 허용한다   ← ★그 게이트는 오늘 없다. 만드는 것이 2a 의 일이다★
```

#### 그 창에서 실제로 무엇이 깨지나 — ★초판이 잘못 적었다★

초판은 「여기서 죽으면 그 대화를 영영 못 잇는다」로 적었다. **그것은 첫 대화일 때의 얘기이고, 더 나쁜 쪽을 놓쳤다.**

★**저장된 sid 가 이미 있는 경우**(그 에이전트의 두 번째 이후 대화 전부) — 창 안에서 죽으면 **옛 sid 가 그대로 남고, 다음 활성화가 조용히 옛 스레드로 재부착한다.** 새로 만든 스레드는 고아가 된다.★ 근거 = 두 활성화 입구가 **저장된 sid 유무 하나로** `SpawnMode` 를 유도한다:

- 로컬 명령 입구 — `profile.backend_session_id.is_some()` 이면 `Resume`(`crates/engram-dashboard-agent/src/commands.rs:521-526`, `activate_profile` 호출 = `:526`). ★**두 입구의 동일성을 못 박는 문장이 여기 있다**★ — 그 갈림 바로 위 주석: 「★모드 유도 규칙은 WS 경로와 같은 것을 쓴다(ADR-0076)★: 저장된 세션이 있으면 이어받기, 없으면 새로. 여기서 다른 규칙을 쓰면 같은 에이전트가 어느 입구로 깨우느냐에 따라 대화 이력을 잃는다」(`:519-520`). ★**초판은 이 인용을 아래 WS 줄에 달았는데 그 자리에 없다 — WS 쪽 주석은 다른 문장이다**★(바로 아래).
- WS 입구 — `resume || profile.backend_session_id.is_some()` 이면 `Resume`(`crates/engram-dashboard-daemon/src/connection_core.rs:1126-1130`, `activate_profile` 호출 = `:1131`). 그쪽 주석은 「★모드 = 세션 존재 여부로 유도(ADR-0076)★ … 저장된 세션이 있으면 wire `resume` 플래그(프론트는 false 로 보낸다)와 무관하게 **항상 Resume** 이다」(`:1115-1117`)이고, 이어서 ★「단 '안전하다' 고 읽지 말 것」★을 붙인다(`:1121-1123`).

★**그래서 결말이 「오류」가 아니라 「조용한 오답」이다**★ — 사용자는 이어받기가 됐다고 보고, 방금 한 대화만 사라진다. 이것이 조사 보고서 BLOCKER B1 의 **살아남은 절반**이고 ADR-0185 가 Phase 2 로 넘긴 그것이다.

#### 게이트를 어디에 두나 — ★강제할 수 있는 유일한 자리★

**오늘 강제 수단이 0 인 이유를 줄로 적는다:**

- 세션은 **pump 를 띄우기 전에 이미 명부에 오른다** — `sessions.insert` 가 `session.start_pump()` 보다 먼저다(`crates/engram-dashboard-agent/src/manager.rs:1322` → `:1337`, ADR-0019 가 그 순서를 의도로 못 박았다). 즉 **spawn 직후부터 그 세션에 쓸 수 있다.**
- 그리고 쓰기 경로에 조건이 하나도 없다 — `AgentSession::write_input_observed` 는 즉시 인코딩하고 즉시 `send_input` 한다(`crates/engram-dashboard-agent/src/session.rs:156-165`).
- `turn/start` 라는 이름이 이 저장소 **코드**에 하나도 없다(`rg "turn/start" crates/ src/ src-tauri/` = 0줄). ★**「코드·문서에 없다」로 적지 말 것 — 문서에는 있다**★: ADR-0185 자신과 이 문서가 그 이름을 쓴다(§1 이 같은 정정을 이미 적었다 — 두 자리를 갈라 두지 말 것). **강제 수단이 없다는 것은 코드 축의 사실이다.**
- `observe_session_id` 의 유일한 생산 호출자는 claude `/clear` watcher 다(`crates/engram-dashboard-daemon/src/lib.rs:288`). 그 watcher 는 codex 의 수령 경로가 될 수 없다(§1). ★**그리고 그 함수에는 화신 가드가 없다**★ — `observe_session_id(&self, id, new_sid)` 는 `incarnation` 인자를 받지 않고(`crates/engram-dashboard-agent/src/profile.rs:629`), 다른 값이면 **무조건** 덮어쓴다. 같은 레지스트리의 `set_last_failure` 는 `incarnation` 을 받는다(`:575-583`) — ★비대칭이고, 그것이 §9-3 의 「그 창에서 죽는 것」 옆에 있는 두 번째 미봉 지점이다★. 죽은 화신의 늦은 관측이 산 화신의 sid 를 덮을 수 있다. **2a 는 이 가드를 만들지 않는다**(수령 배선은 Phase 2b — §0) → §11.

★**그러므로 게이트는 codex 통로 구현체가 진다**★(ADR-0189 · §5-1) — 그 객체가 **`turn/start` 를 만드는 유일한 자리**이므로 조건을 볼 수 있는 유일한 자리다.

#### ★그 구현체가 프로필에 손이 닿나 — 쓰는 손은 닫혔고 읽는 손이 남았다★

**1·2 판은 이 문을 아예 안 봤다.** 게이트가 봐야 하는 값(`backend_session_id`)과 게이트가 불러야 하는 동사(`observe_session_id`)가 둘 다 `ProfileRegistry` 것인데, `backend/` 트리에는 그 레지스트리가 없다. 줄로:

- `rg "ProfileRegistry|profiles" crates/engram-dashboard-agent/src/backend/` = ★**0줄**★(비-테스트). 백엔드가 받는 것은 `&AgentCommand` 와 spawn 인자뿐이고 명부는 그 반대편에 있다.
- 그리고 **밖에서 얻을 수도 없다** — `AgentManager` 는 `pub fn turns()`(`crates/engram-dashboard-agent/src/manager.rs:537`)와 `pub fn presets()`(`:541`)를 내주지만 ★**`pub fn profiles()` 가 없다**★. 그 필드의 doc 이 「프로필 단일 소유자」로 그 비공개를 의도로 적는다(`:361-362`).

★**쓰는 손 = ADR-0189 가 닫았다**★ — **조립점이 준 콜백**이고 backend 가 레지스트리를 직접 부르지 않는다. 선례가 그대로 있다: `Arc::new(move |agent_id, new_sid| { profiles_cb.observe_session_id(agent_id, new_sid); })`(`crates/engram-dashboard-daemon/src/lib.rs:284-290`). ★**동사 하나만 든 클로저이지 레지스트리 자체가 아니다**★ — 그래서 조립점이 백엔드를 알게 되지 않는다(§5-6).

★**남은 것은 읽는 손이다**★ — 그 선례는 **쓰기 한 방향**뿐이라 「이미 기록됐나」를 묻는 동사가 없다. **갈래 둘**(값은 아래, 고르는 것은 §10-16):

| 갈래 | 무엇이 바뀌나 | 무엇을 위험에 넣나 |
|---|---|---|
| **(A) 콜백을 두 동사로 넓힌다** — 「읽는다 / 기록한다」 | 조립점에서 만드는 클로저가 하나에서 둘로. **선례의 확장이고 새 축이 아니다** — `spawn_agent` 이 백엔드 파생 사실 여섯을 그 자리에서 뽑아 내려보내는 줄에 붙는다(`manager.rs:1033-1043`) | ★**포트 이름이 중립이어야 한다**★ — 「codex 용」이면 조립점이 백엔드를 알게 되어 ADR-0004 가 샌다 |
| **(B) 구현체가 「자기가 방금 기록했다」만 기억한다** | 아무것도 — 디스크를 안 묻는다 | ★**그것으로 충분하다는 것이 아래 절의 결론이다**★ — 게이트 조건의 정직한 이름이 이미 「반영 호출이 돌아왔다」라 **디스크를 물을 필요가 없다.** 단 **재기동 후**에는 그 기억이 없으므로 「이미 있는 sid 로 resume 한다」 경로가 따로 서야 한다 |

★**1·2 판이 세운 갈래 (B)(세션 층이 「준비됐다」만 통보)와 (C)(게이트를 세션·매니저 층에 둔다)는 죽었다**★ — 둘 다 「조건을 볼 수 있는 층이 어디냐」를 미결로 둔 위에서만 성립했고, ADR-0189 가 그 층을 확정했다. ★되꺼내지 말 것★ — (C) 는 그 층들이 backend 를 몰라서 물을 어휘가 없다는 §6-3 의 원래 사유로도 이미 닫혀 있다.

- **규칙:** 통로 구현체는 `observe_session_id` 반영 호출이 **돌아온 뒤에만** 첫 `turn/start` 를 낸다. 그 전에 사용자 입력이 도착하면 ★**§4-10 의 큐에 담긴다**★ — **거절하지 않고, 조용히 통과시키지도 않는다**(ADR-0190 가 그 갈림을 닫았다).
- ★**게이트 조건은 하나다 — 3판의 둘째 조건이 없어졌다**★. 3판은 「조건만 둘(persist 완료 · **미확정 턴 없음**)」로 적었는데, 그 둘째는 **재기동 뒤 같은 통로가 다시 붙는 설계** 위에서만 뜻이 있었다. ADR-0192 가 그것을 걷은 뒤로 ★**새 화신의 통로 구현체는 미확정 턴을 하나도 안 갖고 태어나므로 그 조건이 항상 참**★이고, 항상 참인 조건을 게이트에 두면 다음 세션이 그것을 「무언가를 막는 것」으로 읽는다. **그래서 지운다.**
- ★**남는 조건 하나는 여전히 「같은 지점」 규율을 진다**★ — 「지금 `turn/start` 를 낼 수 있나」의 판정은 **통로 구현체 한 곳**에서만 한다. 그 판정에 사유가 더 붙어도(§4-10 의 「턴이 도는가」가 그 하나다) **자리는 늘리지 않는다** — 두 곳에 나누면 하나는 반드시 우회된다.
- ★**세션 층에는 이 조건을 두지 않는다**★ — 그 층은 backend 를 모르므로(ADR-0004) 「persist 됐나」를 물을 어휘가 없다. ★**단 「transport 에도 두지 않는다」로 적었던 1·2 판의 문장은 지운다**★ — ADR-0189 뒤로는 **그 조건을 지는 객체가 바로 transport 구현체**다. 그 문장이 견누던 것은 **범용 파이프**(`StdioTransport`)이고, 그쪽은 지금도 이 조건을 모른다.
- **새 필드·새 매핑·새 파일을 만들지 않는다**(ADR-0185 결정 2) — 우리 id ↔ 백엔드 id 매핑은 **프로필 레코드 자체**다.
- ★**`observe_session_id` 는 불린 뒤에만 원자적이다**★ — ADR-0185 「거부한 대안」이 그것을 명시한다. 위 게이트는 **창을 좁히지 못한다** — 창은 「응답 수령 → 호출」 사이이고 게이트는 그 **뒤**에 선다. ★게이트가 막는 것은 「persist 전에 턴이 나가는 것」이고, 「그 창에서 죽는 것」은 막지 못한다.★ 후자의 복구는 `[미확인]` 으로 남는다(§11).
- ★**id 가 `Uuid` 로 안 파싱되면 이 그림 전체가 막힐 뻔했다 — ★그 위험은 닫혔다★**★: 받은 값이 **UUIDv7 이고 `thread.id == thread.sessionId`** 임을 실측했다(§2 L4). `backend_session_id: Option<Uuid>`(`profile.rs:165`) 칸에 그대로 든다 — ★**§9-1 첫 게이트가 재려던 네 행 중 둘이 여기서 답을 받았다**★(id 수령 · 타입). 남은 둘 = 「id 가 턴마다 재발급되나」(안 그러했다 — 같은 `threadId` 로 둘째 턴이 돌았다) · 「`thread/resume` 이 재기동 뒤에 서나」(★여전히 `[미확인]`★).

#### ★그 게이트가 실제로 단언할 수 있는 것 — 「persist 됐다」가 아니다★

**이 게이트의 조건 이름이 「persist 완료」인데, 그 사실을 확인할 수단이 오늘 없다. 줄로 적는다.**

- `observe_session_id` 는 `bool` 을 돌려주는데 그 값의 뜻은 「**메모리 맵이 바뀌었다**」다 — `mutate_if` 가 클로저의 판정(`changed`)을 그대로 되돌려 준다(`crates/engram-dashboard-agent/src/profile.rs:400-409` · `:629-641`). ★**「디스크에 닿았다」가 아니다.**★
- 그 사이에 `store.save(&snapshot)` 가 불리지만(`:406`) ★**그 함수는 `()` 를 돌려주고 실패를 삼킨다**★ — 계약 자신이 그렇게 적는다: 「전체 스냅샷을 atomic 하게 저장. **실패는 구현 내부에서 로그만 — 호출자를 막지 않는다**」(`:310-311`), 실물은 `tracing::error!` 한 줄이다(`crates/engram-dashboard-agent/src/persistence/mod.rs:102-103`). **호출자·UI·LLM 에 아무것도 안 간다.**
- 그리고 오늘 그 `bool` 은 **버려진다** — 유일한 생산 호출자가 반환값을 안 본다(`crates/engram-dashboard-daemon/src/lib.rs:288`).

★**그래서 게이트 조건의 정직한 이름은 「persist 완료」가 아니라 「수령 → 반영 호출이 돌아왔다」다.**★ 그것이 이 저장소가 오늘 표현할 수 있는 최대치이고, 그 위에 「디스크에 있다」를 얹으면 **없는 보장을 인용하는 것**이 된다. 디스크 확인까지 요구하려면 `ProfileStore::save` 의 **반환형을 바꿔야** 하고, 그것은 그 trait 의 명시된 계약 문장(`:310-311`)을 뒤집는 일이다 — ★2a 가 할 일이 아니다★. §11 에 남긴다.

★**그리고 이 절을 「첫 턴 전 persist 는 성립한 불변식이다」로 인용하지 말 것**★ — ADR-0185 가 그 인용을 금했다. 여기 있는 것은 **요구사항 + 그것을 걸 자리**이고, 불변식이 되는 조건은 §9 가 그것을 실패할 수 있는 단언으로 잡는 것이다. ★위 절이 그 단언의 **천장**까지 정한다★ — §9-3 이 잴 수 있는 것은 「반영 호출 전에 `turn/start` 가 안 나간다」이고 「그 값이 디스크에 있다」는 어느 단언도 잴 수 없다.

---

## 7. 프론트 표현 — 갭 1·2·3 의 처리 + 턴 관측 약속의 재조정

★**두 렌더 경로는 이미 있고, 이미 백엔드 이름이 아니라 capability 로 갈린다**★ — 2a 가 만들 것이 아니라 **지킬 것**이다.

- 프레임 태그가 한 seq 공간을 공유한다: `0` = 터미널 raw 바이트 · `1` = `StructuredEvent` JSON(`src/api/agentClient.ts:26-31` · `src/api/wsFrame.ts:8-9`). 구독 입구 하나(`src/api/protocolClient.ts:744`).
- xterm 경로는 tag1 을 건너뛰고(`src/components/slot/TerminalSlot.tsx:255-261`), 구조화 챗 경로는 tag0 을 건너뛴다(`src/components/slot/RichSlot.tsx:156-162`). 누적은 `src/components/slot/structuredAccumulator.ts`, 렌더는 `StructuredTextView.tsx` + `src/components/slot/chat/`.
- 렌더러 선택 = **한 칸이고 매 렌더마다 다시 유도한다**: `agent.capabilities.output.structured ? 'rich' : 'terminal'`(`src/components/slot/renderMode.ts:24`). 오버라이드 명령도 이미 있다(`src/commands/renderModeCommands.ts`).

★★**전부를 관통하는 불변식 — `if (backend === 'codex')` 가 프론트에 한 줄이라도 생기면 그것이 위반이다**★★. 렌더 분기는 `capabilities.output.structured` 한 칸이 전부이고, 그 규율은 이미 프론트 타입 주석에 박혀 있다(`src/api/types.ts:110`: 「★화면 분기에 쓰지 말 것★ — 렌더러는 `capabilities.output.structured` 한 칸으로 갈린다」). ★아래 셋 중 어느 안을 골라도 이 줄은 안 바뀐다★.

### 7-1. 갭 1 — claude 관례가 프론트 누산기에 박혀 있다

**구체적으로 무엇이 깨지나 (셋):**

1. **user 에코 dedup** — `structuredAccumulator.ts:110-114` 가 `Structured{kind:'user'}` 의 json 에서 uuid 를 뽑아 중복을 스킵한다. codex 스트림에 `kind:'user'` 가 아예 없으면 dedup 이 안 돌고 **무해**하다. 그런데 있는데 uuid 모양이 다르면(또는 다른 자리에 있으면) **같은 내용이 화면에 두 번 뜬다.**
2. ★**대기 인디케이터가 깜빡 꺼진다 — 「idle 로 보인다」가 아니다. 초판이 이 항을 두 군데 틀렸다**★.
   - ★**틀린 사실 ①: 「`:123` 이 turnDone 을 내리는 유일한 지점」**★ — **대입 지점이 넷이다**(전부 판독): `:77`(`TextDelta`) · `:88`(`ToolCall`) · `:123`(`Structured{kind:'user'}`) · `:154`(`reset()`). 올리는 곳은 둘 = `:101`(`Error`) · `:136`(`MessageDone`).
   - ★**틀린 사실 ②: 증상**★ — codex 가 `kind:'user'` 를 안 내면 오히려 그 프레임이 아예 안 와서, 첫 실제 델타가 `:77` 로 내린다. **그래서 「응답 대기 중 화면이 idle」이 codex 의 기본 증상이 아니다.** 실제 증상은 그 소스 주석이 이름 붙여 둔 **flicker 창**이다.
   - **그 창의 기제 — 줄로:** `streaming = awaiting || (!turnDone && items.length > 0)` (`src/components/slot/RichSlot.tsx:270`). 전송 시 `setAwaiting(true)`(`:238`), 그리고 ★**tag1 프레임이 하나 오면 그것이 무엇이든 `setAwaiting(false)`**★(`:167`). 그러니 **첫 프레임이 `turnDone` 을 내리지 않으면** 그 순간 `streaming` 이 false 로 떨어지고 대기 인디케이터(WaitRow)가 꺼진다 — 첫 실제 토큰이 올 때까지. 그 인과를 소스가 그대로 적는다: 「합성 user 에코가 awaiting 을 해제하는 순간 … 파생 streaming 이 false 로 떨어져, 첫 assistant 토큰 전까지 대기 인디케이터(WaitRow)가 깜빡 꺼진다(후속 전송 flicker)」(`src/components/slot/structuredAccumulator.ts:117-122`). ★**`:123` 은 claude 에서 그 창을 닫는 패치이지 「유일한 지점」이 아니다.**★
   - ★**그래서 codex 에서 그 창이 되열리는 조건이 정확히 둘이고, 둘 다 §6-1 표에 이미 있다**★:
     - **`Usage` 가 먼저 온다** — `thread/tokenUsage/updated` → `Usage` 매핑(§6-1)인데, `Usage` arm 은 ★**`turnDone` 을 한 줄도 안 건드린다**★(`structuredAccumulator.ts:90-97`). 그것이 턴의 첫 프레임이면 스피너가 꺼진다.
     - **모르는 최상위 `type` 이 온다** — `default:` arm 이 없어 아무것도 안 하면서(§6-2 의 그 절) `setAwaiting(false)` 만 발화한다.
   - ★**설계 귀결(이 문서의 실제 산출물):** 번역기는 **한 턴의 첫 tag1 프레임이 반드시 `turnDone` 을 내리는 것**을 계약으로 져야 한다★ — 아니면 그 프레임을 아예 내지 않아야 한다. 그것이 §10-2 (가)가 「테스트로 박는다」고 할 때 박아야 하는 단언 중 하나이고, ★**초판의 (가) 서술에는 이 단언이 없었다**★(「`MessageDone` 턴마다 1회」와 「새 턴 시작 신호」 둘만 적었다).
3. **턴 경계가 쪼개지거나 사라진다** — `:129-132` 가 「decoder(backend claude.rs)가 claude 결과 한 줄·한 턴마다 `MessageDone` 을 정확히 1회 발행한다」를 전제로 그것을 ★유일하게 신뢰할 수 있는 턴 경계★로 쓴다(같은 주석이 `turn_id` 로 바꾸면 「현재 always-None 이라 경계가 사라짐」이라 재론 방지를 적어 두었다). codex 번역기가 `turn/completed` 마다 하나를 내면 맞고, `item/completed` 마다 내면 **한 턴이 여러 경계로 쪼개진다.**

★**그리고 프론트가 물어볼 capability 칸이 없다**★ — 이 관례를 신고하는 칸이 `Capabilities` 다섯 영역 어디에도 없다(`crates/engram-dashboard-protocol/src/domain.rs:23-77`). 즉 프론트는 **모르는 채로 claude 관례를 가정하는 것 말고 할 수 있는 것이 없다.**

| 안 | 무엇을 | 대가 |
|---|---|---|
| **(가)** | **중립 어휘의 계약을 엄격히 정의하고 백엔드 번역기가 그것을 지킨다** — **셋**을 주석이 아니라 테스트로 박는다: ① 「`MessageDone` = 턴 경계, 턴마다 정확히 1회」 ② 「새 턴 시작 신호」 ③ ★**「한 턴의 첫 tag1 프레임은 `turnDone` 을 내린다(또는 내지 않는다)」**★ — ③은 위 항 2 가 새로 찾은 단언이고 초판에는 없었다 | claude decoder 도 그 계약에 맞춰 **재확인**해야 한다(오늘 그 계약은 주석에만 있다). Phase 3 로 밀린 「전면 회수」와 어디서 갈리는지를 그어야 한다. ★③은 **프론트 쪽 단언**이라 그 자리가 `src/components/slot/structuredAccumulator.test.ts` 이고, 위 두 개(Rust 골든)와 **레인이 갈린다**★ — §9-3 에 그 행을 더한다 |
| **(나)** | claude 관례를 **capability 칸으로 승격**해 프론트가 읽는다(예: 턴 경계 축을 신고하는 칸) | wire 변경(§10-6) + ★**갭 2 를 키운다**★ — 읽는 사람 없는 칸이 하나 늘거나 프론트에 분기가 하나 는다. 그리고 관례 축이 늘 때마다 칸도 늘어난다 |
| **(다)** | **프론트를 두 관례 모두에 관용적으로** 만든다 — dedup 을 uuid 없이도 성립하게, 턴 경계를 여러 신호에서 유도하게 | ★**어느 백엔드에서도 정확하지 않은 코드**가 된다★. 그리고 관용은 오탐을 숨겨서, 번역기가 틀렸을 때 화면이 **조용히** 이상해진다 — 그 침묵이 §4-7 이 경고한 것과 같은 모양이다 |

★**결정 = (가) — 사용자가 골랐다**★(§10-2). 그 근거 셋: ① 문제의 뿌리가 「번역기가 덜 채워져 원본 JSON 이 화면까지 간다」이고 그 진단은 이미 사용자 결정으로 적혀 있다(`trd.md` §6-1) ② 갭 2 를 키우지 않는다 ③ 계약을 테스트로 박으면 **셋째 백엔드가 같은 질문을 다시 안 한다.** ★**그리고 사용자가 한 가지를 더 박았다 — 「걸러 낸다」가 아니다**★: 되울린 유저 항목은 **재부착 시 화면을 되살리는 재료**라 버리지 않고, **번역기가 그것을 「우리가 보낸 것」으로 표시하면 프론트는 그 표시만 읽는다.** ★claude 쪽은 「바꾸는 것」이 아니라 「이미 참인 것을 적는 것」이고, 전면 회수는 여전히 Phase 3 다★.

### 7-2. 갭 2 — capability 불린 18개 중 16개가 프론트 독자 0

**실측:** 읽는 것은 둘뿐이다 — `output.structured`(`src/components/slot/renderMode.ts:24`) · `control.interrupt`(`src/components/agent/mergeTreeNodes.ts:94`). 나머지 16 칸은 wire 를 타고 프론트까지 가서 **아무도 안 본다**.

★**왜 이것이 2a 의 문제인가**★ — codex 가 claude 와 **다르게 신고해야 하는** 칸이 그 16 안에 있다: `input.raw`(app-server 는 키 입력 바이트를 안 받는다) · `input.message` · `control.resize`(파이프엔 개념이 없다) · `session.resume` · `output.markdown`/`tool_events`/`usage`. **신고해도 읽는 사람이 없으면 그 차이를 화면에 반영할 유일한 길이 백엔드 이름 분기가 된다.** ★갭 2 는 프론트 백엔드 분기가 태어나는 자리다.★

**2a 가 실제로 독자를 필요로 하는 칸 — 하나씩 재 봤다:**

| 칸 | 2a 에 독자가 필요한가 | 근거 |
|---|---|---|
| `control.resize` | ★**아니오**★ | ★**초판의 실측 서술이 틀렸고, 2판이 그 자리에 **또 다른 틀린 측정**을 놓았다 — 둘 다 고친다**★. ① 초판의 「호출자 하나뿐」이 틀렸다. ② 2판의 「`rg resizePty src/` = 호출 넷 + 선언 둘」도 **그 명령이 돌려주는 것이 아니다** — 그 명령은 **24줄**을 낸다(테스트 목·주석·죽은 `src/lab/` 포함). ★열거는 맞고 측정 문장이 틀렸다★. **운영 호출 넷** = `src/components/slot/TerminalSlot.tsx:118` · `:213` · `:287` · `:305` · **선언** = `src/api/agentClient.ts:187` · **구현** = `src/api/protocolClient.ts:852`. ★**그래도 결론은 선다 — 넷이 전부 같은 컴포넌트 안이기 때문이다**★(터미널 슬롯). `RichSlot.tsx` 에는 한 줄도 없다 → codex 를 챗 표면에 그리면 그 경로에 애초에 안 들어온다. **판정 근거는 「호출자 개수」가 아니라 「운영 호출자가 전부 터미널 슬롯 안」이다** |
| `control.interrupt` | ★**신고는 고칠 수 있게 됐다. 그러나 화면에 독자가 없다**★ | ★**1·2 판이 이 칸을 「독자가 이미 있다」로 적은 것은 틀렸다**★ — `src/components/agent/mergeTreeNodes.ts:94` 는 그 값을 트리 노드에 **쓰는** 자리이고, 그 노드의 `canInterrupt` 를 **읽는 화면 코드가 하나도 없다**(비-테스트 실측 — 같은 파일 세 줄 `:55` 선언 · `:94` 대입 · `:111` 예약 기본값이 전부다). ★그 절반(사람이 누를 표면)은 **T-34** 가 진다★(`docs/tracking.md`). 신고 쪽은 닫혔다 — 새 통로 구현체가 `interrupt()` 를 실제로 구현하고(★`turn/interrupt` 18ms 성공 · 대화 생존 — §2 L9★) 자기 caps 를 정직하게 낸다(그 값을 만드는 자리 = ADR-0191 · §5-5). ★**그래서 「동작 없는 참」 위험은 없고, 대신 「참인데 누를 곳이 없다」가 남는다**★. **§10-3·11 · T-34** |
| `input.raw` / `input.message` | ★**갭 3 의 답에 매달림**★ | 챗 표면이면 입력창이 텍스트 한 턴을 보낸다 = `message` 축. 터미널이면 키 바이트 = `raw` 축. **오늘 둘 다 독자가 없어서 아무 일도 안 일어난다** — 즉 「지금은 안 깨진다」이고 「맞다」가 아니다 |
| `session.resume` | **아니오 — 2b** | 2a 는 값을 만들지만 켜는 것은 2b 다. ★켜는 것이 `needs_session()` 과 묶여 있다★(ADR-0185 결정 2 ②: 켜면 발급도 함께 켜져 codex 가 쓰지 않는 uuid 가 심긴다) |
| `output.markdown` / `tool_events` / `usage` | 아니오 | 오늘 챗 렌더가 이 셋을 안 묻고 `StructuredEvent` 변형 유무로 그린다 |

★**결정 = 2a 는 「독자를 만드는 칸」을 0 으로 둔다 — 메인 판정**★(§10-3). 근거 = 위 표에서 **독자가 없어서 2a 가 깨지는 칸이 하나도 없다.** ★보강 실측 둘: **챗 표면은 resize 를 한 번도 안 부른다**(운영 호출 넷이 전부 `TerminalSlot.tsx` 안) · **`canInterrupt` 를 읽는 화면 코드가 0 이다**(→ T-34)★.

★**1·2 판이 「대신 값을 정직하게 신고한다」를 권고에서 떼어 낸 것은 그때 그것이 구조 변경을 요구했기 때문이다 — 지금은 공짜다**★. ADR-0189 가 새 통로 구현체를 세웠고 그 구현체는 `StdioTransport` 의 하드코딩을 안 지난다. ★**그 값을 누가 만드나까지 ADR-0191 이 닫았다 — 그 백엔드의 통로 생성 코드가 만들어 주입하고, 구현체는 하드코딩하지 않고 받아 신고하며, 조립점은 그 값을 만들지도 통로의 구체 타입을 알지도 않는다**★(정본 = §5-5 의 세 문장). ★3판이 여기에 남겨 두었던 「레버가 어느 생성자에 붙나」(§10-11-a)는 그것으로 **닫혔다**★ — `StdioTransport::open` 을 넓히는 갈래는 죽었고(codex 는 그 파일을 안 지난다), 「새 struct 의 생성자」도 정확한 답이 아니다(만드는 자리가 조립점이 아니라 백엔드 폴더다). 남은 것은 **독자 축**뿐이고 ★그 결론은 위 표 그대로다 — **2a 가 깨지는 칸이 하나도 없다.**★

- 신고를 안 바꾸면 codex 세션이 `input.raw=true` · `input.message=false` · `control.interrupt=false` 로 신고한다 — 셋 다 사실과 다르다. ★**「지금은 독자가 없어서 안 깨진다」이고 「맞다」가 아니다**★.
- ★단 이 권고가 「갭 2 가 닫힌다」는 아니다★ — 16 칸이 여전히 안 읽히고, **그 사실을 §11 에 남긴다.**
- **결정은 §10-3**(이 판정이 갭 3 에 매달려 있으므로 순서가 있다 — §10 끝) **+ §10-11**(신고 축).

### 7-3. 갭 3 — 챗 빈 상태가 claude 브랜딩이다

**구체적으로 무엇이 보이나:** codex 에이전트를 챗 표면에 그리면 **복원 완료('live') + 0건 + 미전송** 조건에서 `<ClaudeMascot />`(`src/components/slot/RichSlot.tsx:305`)와 리터럴 `Claude Code`(`:308`)가 뜬다. 그 리터럴은 **의도적으로 i18n 밖**이다(`:297` 주석 + ADR-0145 「빈 상태 구성」). ★그 세 조건이 새로 만든 codex 에이전트의 첫 화면 그대로다★ — 즉 이 갭은 드문 경로가 아니라 **첫 화면**이다.

| 안 | 무엇을 | 대가 |
|---|---|---|
| **(가)** | 2a 에서 codex 를 **챗 표면에 아예 안 그린다**(터미널 유지) | ★이 문서의 범위 정의와 부딪힌다★ — 2a 는 「화면에 중립 어휘로 그려지는 것까지」다. 고르면 2a 의 성공 기준이 「번역기가 중립 이벤트를 낸다」까지로 줄고 화면 확인이 §9 에서 빠진다 |
| **(나)** | **브랜딩을 중립화한다** | ★조건부로 감추는 판정이 `if (backend === 'codex')` 가 되면 그것이 위반★. 판정 축을 capability 나 **표시명 축**으로 세워야 한다 |
| **(다)** | 2a 는 터미널 전용, **챗 표면은 2b** | (가)와 같은 대가 + 2b 가 커진다 |

**권고: (나) — 단 판정 축은 백엔드 이름이 아니다.** 근거 = ① 범위 정의가 화면까지다 ② 마스코트는 ADR-0145 가 「빈 상태 구성」으로 세운 것이라 **지우는 문제가 아니라 무엇을 그릴지의 값을 어디서 받나**가 문제다. ★**그리고 이 권고가 무게를 얻었다**★ — codex 통로 생성 코드가 caps 에 `output.structured = true` 를 실어 주면(§5-5 · ADR-0191) 프론트가 `renderMode.ts:24` 에서 **자동으로 `rich` 를 고른다.** 즉 ★**아무것도 안 하면 (나) 로 간다**★ — 남는 질문은 「그리나」가 아니라 「빈 상태 라벨을 어떻게 하나」다. **결정은 §10-4.**

★**초판의 근거 ③ 은 걷는다 — 두 라벨을 섞어 놓았다**★. 그 근거는 「표시명이 `display_name ?? basename(cwd)` 규칙으로 세 곳에서 같게 파생된다」였는데(백엔드 = `crates/engram-dashboard-agent/src/name.rs:55-58` · 트리 = `src/components/agent/AgentList.tsx:269` · 챗 헤더 = `RichSlot.tsx:125`), ★그것은 **에이전트·폴더 이름**이고 `RichSlot.tsx:125` 는 이미 그 값을 **챗 헤더**에 쓰고 있다★. 빈 상태의 `Claude Code` 는 **제품명**이고 그래서 ADR-0145 가 의도적으로 i18n 밖에 뒀다(`RichSlot.tsx:297` 주석: 「문구는 제품명이라 번역 대상이 아니다」).

★**그래서 그 자리에 폴더 basename 을 넣는 것은 중립화가 아니다**★ — 헤더에 이미 있는 값을 한 번 더 크게 찍는 것이고, 「이 대화가 무슨 프로그램인가」라는 원래 뜻이 사라진다. **진짜 중립 제품 라벨은 값의 출처가 새로 있어야 한다** — 백엔드가 자기 표시명을 신고하는 칸이든, 프론트의 백엔드-무관 사전이든. ★**전자면 `Capabilities` 어휘가 늘고 그것이 §10-6 을 다시 연다**★. 즉 (나)의 대가는 「분기 하나」가 아니라 **어휘 하나**일 수 있다.

- ★그리고 §7 머리의 불변식은 그대로다★ — 무엇을 고르든 `if (backend === 'codex')` 는 안 된다(`src/api/types.ts:110`).

★**이 답이 2a 의 성공 기준을 정한다**★ — (가)/(다)를 고르면 §9 의 판정이 자동 단언까지고, (나)를 고르면 **§9 의 사람 눈 목록이 늘어난다**(빈 상태에 무엇이 뜨나). **결정은 §10-4** 이고 §10-3 이 여기 매달린다.

### 7-4. ★§0 의 약속을 줄에 맞춘다 — 「턴 관측은 2a 의 산출물」은 조건 넷이다★

§0 이 우편(2b)의 전제로 「턴 관측이 2a 의 산출물이다」를 적었다. **그 문장을 지탱하는 조건을 세어 보니 넷이고, 오늘 서 있는 것은 하나뿐이다.** 하나라도 빠진 채 2b 가 `reads_messages()` 를 열면 결말은 **턴 중 우편 주입**이다 — 그것이 그 값을 false 로 둔 원래 사유다(`crates/engram-dashboard-agent/src/backend/codex/mod.rs:76-81`).

| # | 조건 | 오늘 | 줄 |
|---|---|---|---|
| ① | **`turn_classifier` 선언** | ★**없다**★ — `CodexBackend` 가 이 메서드를 override 하지 않아 trait 기본값을 탄다. 기본값은 `no_turn_signals`, 즉 **침묵**이다 | 기본값 `crates/engram-dashboard-agent/src/backend/mod.rs:143-145` → 실물 `:271-273`. 선언 부재 = `backend/codex/mod.rs` 에 그 이름 0회(§8) |
| ② | **번역기가 분류자가 집을 수 있는 이벤트를 낸다** | 번역기 자체가 2a 산출물 | 분류자 시그니처가 `fn(&OutputEvent) -> Option<TurnSignal>`(`backend/mod.rs:268`)이라 ★**`turn/started`·`turn/completed` 가 `OutputEvent` 로 착지해야 분류자에 닿는다**★. §6-1 표의 그 두 행이 이 조건이다 |
| ③ | **명부 프록시 `turn_signal`** | ★**공짜로 선다 — 단 사유가 4판에 잘못 적혀 있었다**★ | `LiveAgent.turn_signal = a.capabilities.output.structured`(`crates/engram-dashboard-daemon/src/messaging_host.rs:104`), 게이트에서 AND 된다(`crates/engram-dashboard-messaging/src/service.rs:624-626`). ★**「`StdioNdjson` 갈래에서 조립점이 하드코딩 주입하므로 codex 가 그 통로를 타는 순간 참이 된다」는 틀렸다**★ — ★codex 세션은 `transport/stdio.rs` 를 **지나지 않는다**★(§5-5 · §8). `manager.rs:89` 의 그 주입은 **claude json·평문 경로 것**이다. 이 칸이 참이 되는 계기는 「그 통로를 타는 것」이 아니라 ★**codex 통로 생성 코드가 caps 에 `output.structured = true` 를 주입해 신고하는 것**★이고(ADR-0191 · §5-5), 그 주입은 챗 표면 때문에 어차피 필요하다 — **그래서 추가 작업이 0 이라는 뜻에서만 「공짜」다** |
| ④ | ★**쓰기 시점의 턴-시작 신호**★ | ★**없다**★ | 오늘 claude 에서 그 신호를 만드는 것은 **입력 에코**다 — `write_input_observed` 가 `encoder.input_echo_event(...)` 를 물어 `Some` 이면 `core.emit` 한다(`crates/engram-dashboard-agent/src/session.rs:175-176`). 그 메서드의 **trait 기본값이 `None`** 이라(`backend/mod.rs:209-211`) 선언하지 않은 백엔드는 에코를 하나도 안 낸다 |

★**④가 이 절의 새 발견이고, 다른 셋보다 조용하다**★ — ①②③이 다 서도 ④가 없으면 **우리가 `turn/start` 를 쓴 순간부터 codex 의 첫 알림이 도착하기까지 그 에이전트는 「미관측」이다.** 그리고 미관측은 게이트에서 **idle 로 흡수된다**(positive-knowledge-only — `crates/engram-dashboard-messaging/src/busy.rs:178-180` 이 `None` 이면 `false` 를 돌려준다). 즉 ★**그 창에 정확히 편지가 꽂힌다**★ — 우편이 막으려던 바로 그것이다. 오늘 claude 가 그 창을 안 갖는 이유는 에코가 `Structured{kind:"user"}` 로 즉시 `Progress` 를 만들기 때문이다.

★**같은 fail-open 이 §4-10 의 입력 큐도 때린다 — 그쪽은 이 절을 안 기다리고 스스로 닫았다**★. 큐의 해제 조건을 코어의 사실 표에서 읽으면 이 창에서 `turn/start` 두 개가 겹치므로, ★**codex 의 해제 판정을 통로 구현체 자신의 턴 기계로 못 박았다**★(ADR-0193 · §4-10 규율 4). ★그러나 그것이 이 절을 닫지는 않는다★ — 우편 게이트(`busy.rs`)는 **여전히 코어를 읽고**, 그 독자는 큐가 아니라 메시징 커널이다. **큐에서는 닫히고 우편에서는 안 닫힌다** — 우편 쪽을 닫는 것은 아래 ①②④ 선언이다.

★**그리고 2a 가 스스로 만들 수 있는 함정 하나 — 이름이 이미 붙어 있다**★

③은 공짜로 서고 ①은 **선언을 안 하면 안 선다.** 그 조합(= `output.structured` 는 `true` 인데 턴 이벤트가 하나도 없다)이 정확히 메시징 커널이 「프록시가 깨지는 날」로 이름 붙여 둔 상태다:

> ★busy 관측 대상 = 턴 신호를 내는 백엔드★: … decoder 는 구조화 출력 capability 와 **정확히 같은 조건**으로 존재한다. … **프록시가 깨지는 날(구조화인데 턴 이벤트가 없는 백엔드 등장) 진짜 capability 필드를 추가한다** — `crates/engram-dashboard-messaging/src/busy.rs:19-24`

★**즉 「①을 안 하고 넘기면」 그 문장이 요구하는 것은 `Capabilities` 에 칸을 하나 더하는 것이고, 그것은 §10-6 을 다시 여는 일이다.**★ ①을 하는 것이 그 비용을 피하는 길이다.

**오늘 안 깨지는 이유(fail-safe)와 그것을 안심으로 읽지 않을 이유:**

- `reads_messages()` 가 false 이고(`backend/codex/mod.rs:82-84`) 배달 명단이 그 값으로 걸러진다(`messaging_host.rs:160` — `is_live(a) && a.reads_messages`). **그래서 오늘 codex 에는 편지가 애초에 안 간다.** 2a 가 무엇을 하든 이 축은 안 깨진다.
- ★**그러나 그것이 「2a 는 신경 안 써도 된다」가 아니다**★ — 그 값을 여는 것이 2b 이고, **2b 는 이 넷을 다시 확인할 자리가 아니다**(그 단계엔 번역기가 이미 굳어 있다). ①②④는 **번역기·인코더 선언과 한 몸**이라 2a 에서 같이 서야 한다.

★**그래서 §0 의 약속을 이렇게 다시 적는다**★: 「턴 관측은 2a 의 **자동 부산물이 아니고**, 2a 가 ①②④를 함께 선언할 때 성립한다.」 ★④의 모양(codex 도 합성 유저 에코를 내나 · 낸다면 그 이벤트 shape 는 무엇인가)은 **§6-1 표에 없던 행**이고, 그 shape 는 프론트 dedup 계약과 묶여 있다★(에코의 shape 는 그 백엔드 decoder 가 replay 로 만드는 것과 같아야 한다 — `backend/mod.rs:204-205`). **codex 가 유저 메시지를 replay 로 되울리는지가 `[미확인]` 이라 그 짝을 지금 확정할 수 없다** → **§10-15** · §11.

---

## 8. 손대는 곳

★**아래는 2a 의 접합 지도이지 구현 지시가 아니다**★ — 줄 번호는 이 문서를 쓴 시점(2026-09-09)의 것이고, 코더는 앵커 주변을 다시 읽고 들어간다. **★§10-x 에 매달림★** 표시가 붙은 것은 모양이 그 결정에 달렸다는 뜻이다.

★**네 상태기계가 「어디 사나」는 §3 표가 정본이고 이 절은 그것을 옮겨 적는다 — 둘이 어긋나면 §3 이 맞다**★. 초판은 넷을 통째로 `backend/codex/` 또는 상관 계층에 두었는데 §3 은 소유권을 갈라 배분했다. 그 배분이 이렇다:
| 상태기계 | 새로 만드나 | 어디 |
|---|---|---|
| 프로세스 | ✗ **이미 있다** — 단 소유자가 `StdioTransport` 가 아니라 **새 통로 구현체**다 | child + Job Object. ★같은 모양이지 같은 객체가 아니다★ — `StdioTransport` 는 이 경로에서 안 만들어진다(§5-5) |
| 연결 | ✓ | **codex 통로 구현체** 안 — 대기 맵 + 아웃바운드 id 카운터(§5-1). ★자리는 **ADR-0189** 가 정했다★ |
| 스레드 — 신원 축 | ✗ **이미 있다** | 프로필 레지스트리(`backend_session_id`) — 새 필드 없음(ADR-0185 결정 2). ★기록하는 손 = 조립점이 준 콜백★(§5-6) |
| 스레드 — 런타임 축 | ✓ | **codex 통로 구현체** — `backend/codex/` 안 |
| 턴 | ✓ (기계만) | **통로 구현체**. ★**사실 계층(`turn.rs`)은 안 건드린다**★ — 코어 호출 지점을 늘리지 않는다(ADR-0127 결정 5). ★**codex 의 큐는 그 계층을 읽지도 않는다**★ — 해제 판정이 구현체 자신의 턴 기계다(ADR-0193 · §4-10 규율 4). 그 계층을 읽는 것은 **claude json 쪽 해제 조건**이다 |

### 새로 만드는 것

| 무엇 | 어디 | 비고 |
|---|---|---|
| app-server 프로토콜 어휘 — 메서드 이름 · 알림 이름 · 봉투 | `crates/engram-dashboard-agent/src/backend/codex/` **안** | ★ADR-0004 — 이 폴더 밖에 나가면 격리 위반★. 오늘 그 폴더에 파일이 `mod.rs` 하나뿐이라 모듈이 는다 |
| ★**codex 통로 구현체**★ — `AgentTransport` 구현. 자식 + Job Object · 단일 writer stdin · 자기 읽기 스레드 · 대기 맵 · id 카운터 · `thread.id` · 준비 상태 | `backend/codex/` **안** | ★**ADR-0189 가 확정한 자리다 — 「제안」이 아니다**★. §4-1·§4-4·§4-9·§5-1. **읽기 스레드는 응답을 기다리지 않고**(§5-2) **stdin 락도 잡지 않는다**(§4-2) |
| ★**그 통로를 만들어 넘기는 코드**★ — 자기 caps(주입값) · 자기 decoder · 자기 argv 를 함께 얹는다 | 같은 폴더 안, `backend_for` 뒤 | ★**ADR-0191**★ · §5-1. 조립점은 그것을 받아 쓸 뿐 **구체 타입을 이름으로 모른다.** 선례 = `backend::output_decoder`(`backend/mod.rs:477`) |
| 인바운드 분기 — 요청 / 알림 / 응답 / 파싱 실패 | 같은 구현체 안 | §5-2 의 네 경우. ★**밖으로 나가는 간선이 0 이다**★ |
| 번역기(decoder) — codex 알림 → `OutputEvent` | `backend/codex/` 안 | `OutputDecoder` 구현(trait = `crates/engram-dashboard-agent/src/transport/mod.rs:32-39`). claude 쪽 선례 = `ClaudeStreamDecoder`. ★**순수해야 한다 — 알림 문자열 in, 중립 이벤트 out**★(§9-4 의 골든이 그것을 요구한다) |
| ★**json 모드 입력 큐**★ — 「지금 보낼 수 있나」 축 + 순서 보존 흘리기 | 쓰기 창구 앞(json 모드 **공통**) | ★**codex 전용이 아니다 — claude json 도 탄다**★(ADR-0190 · §4-10). ★**해제 판정의 주인이 갈린다**★(**ADR-0193** — ADR-0190 결정 4 의 개정) — claude json 은 `AgentManager::turns()` 를 읽고, **codex 는 통로 구현체 자신의 턴 기계를 읽는다**(코어 쪽은 codex 에서 fail-open). ★판정과 「진행 중으로 전이」는 **한 락 안**이다★(§4-10 규율 4 — `send_input(&self)` 이라 동시 호출이 가능하다) |
| ★**유계 writer 스레드**★ — ★**이 통로 안에서**★ stdin 을 blocking 으로 잡는 유일한 스레드 | 같은 창구 안 | §4-9. ★**읽기 스레드의 인바운드 응답도 이 큐를 탄다**★ — 그래야 읽기가 stdin 락에서 멈추지 않는다(§4-2 의 자기잠금). ★**「워크스페이스에서 유일」이 아니다**★ — claude json 의 `StdioTransport::send_input` 도 오늘 같은 모양으로 블록한다(`transport/stdio.rs:295-304`). ★**종료 회계 = §5-7**★: 이 스레드는 첫째 동사가 **인과로** 끝내고 아무도 기다리지 않는다 — **셋째 동사를 만들지 않는다** |
| 입력 인코더 — 텍스트 1턴 → `turn/start` JSON | `backend/codex/` 안 (+ `InputEncoder` 셋째 태그를 세울지는 §10-7 ②) | ★**세션별 상태는 여기가 아니라 통로 구현체가 진다**★(§5-3) — 그래서 인코더를 상태 갖는 객체로 바꾸는 변경은 **불필요하다** |
| 상태기계 **셋**(연결 · 스레드 런타임 축 · 턴) | 전부 **통로 구현체 안** | §3. ★**넷이 아니다**★ — 프로세스 기계와 스레드 신원 축은 이미 있다(위 배치 표). ★시간·프로세스 없이 단독으로 도는 하네스를 함께★(ADR-0012) |
| 시험대의 app-server 질문들 | `crates/engram-dashboard-agent/tests/backend_contract.rs` **연장** | ★새 하네스를 만들지 않는다★ — §9 |
| `AgentCommand::Codex` 의 통로 모드 표현 | `crates/engram-dashboard-agent/src/profile.rs:66-69`(오늘 `extra_args` 하나) | ★**§10-7 ① 에 매달림**★ — 칸을 더하나 `extra_args` 로 나르나 **별 변형을 세우나**. ★그 선택이 트립와이어 커버리지를 함께 정한다★ — 선언 표가 변형당 한 줄이라 **칸으로 나르면 둘째 모드가 검사를 안 탄다**(`backend/mod.rs:779` · §5-5) |
### 고치는 것

| 파일:줄 | 무엇 | 절 |
|---|---|---|
| `crates/engram-dashboard-agent/src/backend/codex/mod.rs:56-58` | `needs_session()` doc — 「호출자가 세션 id 를 정할 수 없다」는 **사실은 맞지만**, ADR-0185 결정 1 이 그 사실을 복원 성립과 분리했다. ★`// ADR-0185` 앵커와 함께 고친다★ | ADR-0185 「영향」 |
| 같은 파일 `:141-144` | `capabilities()` doc — 「호출자가 sid 를 못 정하므로 **무손실 복원이 성립하지 않는다**」. ★**결정 1 이 이 문장을 뒤집었다**★. 그 ADR 이 「Phase 2 가 `// ADR-0185` 앵커와 함께 고친다 … 방치하면 결정이 뒤집힌 채 전달된다」로 이 작업을 지목한다 | ADR-0185 「영향」 |
| 같은 파일 `:145-158` | `BackendCaps` — `session.resume` 값. ★**2b**★. 켜는 것이 `needs_session()` 과 묶여 있다(ADR-0185 결정 2 ②) — 2a 는 값을 안 뒤집는다 | §7-2 |
| 같은 파일 `:80-81` | `reads_messages()` false 사유 주석의 마지막 두 줄이 「★여는 조건도 다르다★: 턴을 관측할 수 있게 되면(상주 JSON 서버) 이 값이 열린다」로 적혀 있다(그 doc 전체 = `:76-81`, fn = `:82-84`) — ★**그 조건이 2a 에서 충족된다**★. 값을 여는 것은 2b(우편은 범위 밖)지만 **조건이 충족됐다는 사실을 주석에 남긴다** | §0 |
| `backend/codex/mod.rs` (선언 없음) | ★**넷을 하나도 선언하지 않는다**★ — `transport_shape` · `input_encoder` · **`output_decoder`** · `turn_classifier`(실측 — 그 파일에 그 네 이름이 0회. 선언하는 것은 `needs_session`·`supports_control_channel`·`accepts_mcp_config`·`reads_messages`·`build_spec`·`capabilities` 여섯이다). 즉 기본값 `Pty` · `Raw` · **decoder 없음** · 신호 없음이다 | §3 · §5-5 · §6 · §7-4 |
| `backend/codex/mod.rs` — `output_decoder` | ★**번역기를 실제로 꽂는 선언이 이것 하나다**★. trait 기본값이 `None` 이라(`backend/mod.rs:219-221`) **선언하지 않으면 번역기가 조립되지 않고 바이트가 직통한다** — 오류도 경고도 없다. claude 쪽 선례 = `backend/claude/mod.rs:368`. ★단 **뽑아 넘기는 자리가 바뀐다**★ — 오늘은 조립점이 `backend::output_decoder(&profile.command)` 로 따로 뽑아 `select_transport` 에 넘기는데(`manager.rs:1039` → `:79-99`), ADR-0191 뒤로는 백엔드가 통로를 만들 때 **자기 decoder 를 자기가 꽂는다** | §6 · §5-1 |
| ★**`backend/codex/mod.rs:90-137` — `build_spec`**★ | ★**새 행. 3판이 빠뜨렸다**★ — 오늘 이 함수는 **무조건 대화형 argv** 를 만든다(`--cd <cwd> -s workspace-write -a on-request` + 패스스루, 상수 `:43-51`). app-server 를 띄우려면 argv 가 ★**`app-server --stdio` 축으로 갈려야 한다**★ — 실측이 뜬 형태가 그것이고 정책은 argv 가 아니라 `thread/start` params 로 갔다(§2 L4 · §10-5). **프로세스 cwd 는 `CommandSpec.cwd` 가 그대로 지고, 워크스페이스 폴더는 그 params 의 `cwd` 로 간다.** ★`cmd.exe /c` 감싸기는 그대로 필요하다★(shim — L15). ★**그리고 그 감싸기가 남기는 `%VAR%` 확장 위험이 이 모드에서 *줄어든다* — §8 이 4판에서 그것을 안 적었다**★: 오늘 argv 로 실려 cmd 를 지나던 **워크스페이스 경로**(`--cd <cwd>`)가 app-server 모드에서는 **argv 를 떠나 `thread/start` params 로 가기 때문**이다(L4). ★**없어지는 것은 아니다**★ — 패스스루 `extra_args` 는 여전히 argv 를 타고 cmd 를 지난다. 그리고 그 확장이 우리 값에서 실제로 무엇을 하는지는 ★`[미확인]`★ 이다(재 본 적이 없다) | §5-1 · §10-5 · ★**§10-7 ①**★ |
| `crates/engram-dashboard-agent/src/backend/mod.rs:374-379` | `TransportShape` **셋째 변형** — 파이프 + 양방향 JSON. ★**ADR-0189 「영향」이 확정 — 결정 대기가 아니다**★. ★단 ADR-0191 뒤로 그 값은 **선언**이지 생성 스위치가 아니다★ — 생성 match 가 은퇴하므로 **요구되는 arm 이 없다**(§5-1·§5-5). 남는 독자는 아래 트립와이어 하나 | §5-1 · §5-5 |
| 같은 파일 **`:779`** | ★**선언 표 트립와이어**★ — `expected_codec_axis` 의 codex 행이 오늘 `(InputEncoder::Raw, false, TransportShape::Pty)` 다. app-server 모드는 **decoder 를 갖고 통로 모양도 갈린다** = 둘째·셋째 칸이 함께 뒤집힌다(shape 단언은 `:800-802`). ★**같은 커밋에서 갱신하지 않으면 `codec_axis_is_consciously_declared_for_every_backend` 가 빨개진다**★. ★그리고 그 표의 키가 `AgentCommand::Codex { .. }` **변형 하나**라, 모드를 칸으로 나르면 둘째 모드가 이 검사를 **안 탄다**★ | §5-5 · ★**§10-7 ①**★ |
| 같은 파일 `:296-301` | `backend_for_encoder` 표 — ★셋째 `InputEncoder` 태그를 세울 때만★ | ★**§10-7 ②**★ · §5-3 |
| `crates/engram-dashboard-agent/src/manager.rs:1033-1043` · `:1235` · `:1241` | ★**조립점이 국면마다 `backend_for` 를 다시 타는 여섯 줄** — 한 번의 호출로 접고 통로를 그 묶음에 싣는다★(ADR-0191 · §5-1). ★**그리고 shape-keyed 생성 match 가 은퇴한다**★ — `spawn_session` 의 `transport_shape` 인자(`:1235`)와 `select_transport(transport_shape, …)` 호출(`:1241`)이 그 실물이다. ★**`select_transport` 자체의 두 갈래와 `structured: true` 주입(`:89`)은 그 코드가 옮겨 가도 값·순서가 그대로다**★ — ★claude·shell 동작 변화 0 이 수락 조건★. ★**어디에도 shape 로 생성자를 고르는 match 를 다시 만들지 않는다**★ | §5-1 · §5-7 |
| `crates/engram-dashboard-agent/src/transport/stdio.rs:363-384` · `:47-50` | ★**안 고친다 — 3판의 「조건부로 남는다」 행은 닫혔다**★. codex 세션은 이 파일을 지나지 않고, 「하드코딩 금지 + 생성 시 주입」 규율은 그대로 유효하며, **주입값을 만드는 자리만** 백엔드로 옮겨 간다(ADR-0191 · §5-5 의 세 문장). 이 파일은 claude json·평문 경로 것으로 **한 글자도 안 바뀐다** | §5-5 |
| `src/components/slot/structuredAccumulator.ts:110-114,123,129-132` | claude 관례 셋 — ★**「걸러 낸다」가 아니라 「우리가 보낸 것」 표시를 읽는다**★(§10-2 사용자 결정). 되울린 유저 항목은 **재부착 시 화면 복원 재료**라 버리지 않는다 | §7-1 · ★**§10-2**★ |
| 같은 파일 `:66-138` | ★**새 행**★ — switch 에 **`default:` arm 이 없다.** §10-6 이 타입 하나를 더하기로 했으므로 ★그 가지를 안 만들면 새 타입이 **조용히 사라진다**★ | ★**§10-6**★ · §11 |
| `src/components/slot/RichSlot.tsx:297,305,308` | 챗 빈 상태 브랜딩 | ★**§10-4**★ · §7-3 |
| `crates/engram-dashboard-protocol/src/domain.rs:23-77` + `bindings/` | ★**`StructuredEvent` 에 타입 하나를 더한다 — 「턴 끝 + 어떻게 끝났나」**★(§10-6 사용자 결정). `Capabilities` 쪽은 안 늘린다(지금은) | ★**§10-6**★ · §6-1 |
| `crates/engram-dashboard-protocol/src/lib.rs` `PROTOCOL_VERSION` | ★**안 올린다**★ — §10-6 이 그 판정을 함께 냈다(화면으로 흐르는 이벤트 enum 은 모르는 종류를 조용히 통과시킨다). ★이 행을 「어휘가 늘면 자동으로 bump」로 되돌리지 말 것★ | ★**§10-6**★ |
| `crates/engram-dashboard-agent/src/backend/mod.rs:443-446` | `InputEncoder::submit_sequence()` 의 match 는 **exhaustive** 다(`Raw => Some(b"\r")` · `ClaudeStreamJson => None`). ★셋째 태그를 더하면 **컴파일러가 여기 arm 을 요구한다**★ — 태그를 안 세우면 이 행이 안 든다 | ★**§10-7 ②**★ · §5-3 |
| `crates/engram-dashboard-agent/src/backend/codex/mod.rs` — `input_echo_event` | trait 기본값이 `None` 이라(`backend/mod.rs:209-211`) 선언하지 않으면 **쓰기 시점 턴-시작 신호가 없다**(§7-4 ④). ★선언하면 그 이벤트가 §10-2 의 「우리가 보낸 것」 표시를 단 형태여야 한다★ | ★**§10-15**★ · §7-4 |
| `AgentTransport`(`transport/mod.rs:41-57`) | ★**안 고친다 — 3판의 「읽기 함수 하나」 행은 철회됐다**★. 「우리가 죽였나」는 **통로 사유 플래그**라 묻는 쪽과 답하는 쪽이 같은 객체이고, 공용 계약이 그것을 나를 이유가 없다. ★3판의 근거였던 「세 구현체 전부가 이미 그 비트를 갖고 있다」는 거짓이었다★ — `pub struct ApiTransport;`(`crates/engram-dashboard-agent/src/transport/api.rs:14`)는 필드가 하나도 없다. **여섯 메서드 그대로**(ADR-0189) | §3 · §10-13 |
| 쓰기 창구 앞 — ★**json 모드 입력 큐**★ | §4-10. ★**claude json 경로의 쓰기 타이밍이 바뀐다**★(턴 중 입력이 즉시 안 나간다). ★**`Ok` 의 뜻은 안 바뀐다 — 하나로 유지한다**★(사용자 결정 · §10-20): `Ok` = 「우리가 전량 맡았다」. ★**바뀌는 것 하나 = 담긴 뒤에 실패한 write 는 그 호출자에게 돌아갈 길이 없다 — 한계로 남긴다**★(§11) | §4-10 · ADR-0190 · ADR-0193 |
| `src/components/slot/structuredAccumulator.test.ts` | §10-2 의 단언 ③(「한 턴의 첫 tag1 프레임이 `turnDone` 을 내린다」)이 사는 자리. **Rust 골든과 레인이 갈린다**(vitest) | ★**§10-2**★ · §7-1 |
| `src/api/wsFrame.ts:35` | §10-6 을 「새 프레임 태그를 만든다」로 읽는 갈래에서만 든다. 그 줄이 `0`·`1` 외를 **로그도 카운터도 없이** 버리므로, 고치지 않으면 새 태그 스트림이 「한가한 에이전트」와 구별되지 않는다(§6-2). ★**타입 추가는 tag1 안이라 이 행이 안 들 수 있다**★ | ★**§10-6**★ |

★**앵커 부채를 여기서 닫는다**★ — `rg "ADR-0185|ADR-0186" crates/ src/ src-tauri/` = **0줄**(ADR-0186 「영향」의 실측). CLAUDE.md 「설계 결정 기록」이 이름한 두 발견 표면 중 **코드 앵커 쪽이 비어 있다**는 뜻이고, ADR-0185·0186 둘 다 그 채움을 **이 단계 몫**으로 지목한다. 위 두 주석 수정이 그것을 닫는다. ★**두 주석을 「사실 서술이라 무해하다」로 남기지 말 것**★ — 그 자리가 다음 세션이 codex 백엔드에서 가장 먼저 읽는 텍스트다.

### 손대지 않는 것 (명시)

- ★**「핵심 불변식」 전부 — 아홉을 다 적는다**★(초판은 여섯만 적고 「전부」라고 했는데, ★빠진 셋이 정확히 2a 가 스치는 것들이었다★):
  1. **kill 인과**(ADR-0001) — §4-8 이 그대로 성립함을 보인다. **transport 안에 아무것도 더하지 않는다.**
  2. **finalize 1회**(ADR-0005).
  3. **락 순서**(ADR-0006).
  4. ★**상태 알림 분담**(ADR-0005)★ — 과도기 `Exiting` = manager, terminal = pump 단독이고 **프론트는 `status_changed` 로 terminal 을 판정하지 않고 `agent-list-updated` 로 판정한다.** ★codex 가 스레드·턴 상태를 내기 시작하면 그것을 「세션 상태」로 승격시키고 싶어지는데, 그 순간 이 분담이 깨진다★ — `thread/status/changed` 는 §6-1 표에서 **화면 어휘가 아니라 통로 구현체의 입력**이다.
  5. **replay→live 순서**.
  6. ★**턴 관측 정리 = 두 지점뿐**(ADR-0127 결정 5)★ — `OutputCore::finish` + `emit` 의 finalize 재확인. **세 번째 호출자를 늘리지 않는다.** §3 이 그 판정을 상세히 적는다.
  7. **등록 순서**(ADR-0019) — `sessions` insert 가 pump 시작보다 먼저. ★§6-3 이 그 사실에 기대고 있다★(그래서 게이트를 세션 층이 아니라 **통로 구현체**에 둔다).
  8. ★**소유권 분할**★ — transport = master/writer/child/shutdown/job · core = subscribers/replay/seq/status/finalized/drain_handle · session = id/cwd/epoch/cols/rows. ★**ADR-0189 는 이 분할을 바꾸지 않고 transport 칸 안에서 소유물이 늘 뿐이다**★(§5-7).
  9. **화신 표식 비교 규율**(ADR-0163/0164) — 일치/불일치만, 구독 effect deps 는 `[viewId, agentId]`.
- ★**셋을 더 적은 것이 이 절의 성격을 바꾼다**★ — 4·6·8 은 「안 건드린다」가 아니라 **「건드리고 싶어지는데 건드리지 않는다」**다. 그래서 위에 각각 사유를 붙였다.
- ★**`Capabilities::compose` 의 소유권 분할**★(`crates/engram-dashboard-agent/src/types.rs:475-486`) — **`BackendCaps` 에 `output` 영역을 더하지 않는다.** `trd.md` §4-8 이 이미 그 유혹과 그 대가를 적었다(지시서 문장을 따르면 소유권 분할이 무너진다).
- `src/components/slot/renderMode.ts:24` 의 **판정 축** — 값이 바뀔 뿐 축은 그대로.
- `src-tauri/src/daemon_client/**` · `crates/engram-dashboard-net/**` · 루트 `Cargo.toml` members (§0 — 병렬 세션 소유, `[미확인]`).
- **claude 실행 경로** — ADR-0185 「claude 경로는 한 줄도 바뀌지 않는다」. ★**단 그 문장의 범위는 spawn·복원 경로다 — 이번 판에서 둘이 그 밖에서 닿는다**★: ① §4-10 의 입력 큐가 **claude json 의 쓰기 타이밍**을 바꾼다(턴 중 입력이 즉시 안 나간다 — ADR-0190, 사용자 확인받음) ② §10-2 가 claude decoder 의 `MessageDone` 계약을 **테스트로 박는다**(그건 「바꾸는 것」이 아니라 「이미 참인 것을 적는 것」이다). ★그 둘을 「한 줄도 안 바뀐다」로 덮지 말 것★.
- `LLM_BACKEND_POLICY`(`crates/engram-dashboard-agent/src/commands.rs:306-315`) — codex 를 LLM 표면에 여는 것은 별건이다. ★단 그 거절의 사유가 「사람이 아닌 호출자는 신뢰 모달을 못 지난다」인데 **app-server 에 그 모달이 있는지 `[미확인]`** 이다★ → §10-9.
- 벤더 파일 읽기(`state_5.sqlite` · rollout JSONL · `session_index.jsonl`) — ADR-0008 「추적 파일로 기능 확장 금지」. **S9 는 진단 지식으로만 갖는다.**

---

## 9. 검증 계획

### 9-1. 첫 게이트 — ★앞 절반은 통과했다. 남은 것은 **재기동 후 잇기**다★

**1·2 판의 이 게이트는 「스레드를 만들어 id 를 받는다」였고, 그것은 2026-09-09 에 통과했다**(§2 L4 — `result.thread.id` 가 UUIDv7, `thread.sessionId` 와 동일). ★**그래서 게이트가 좁아졌다.**★

**남은 것 셋:**

1. ★**`thread/resume` 이 서나**★ — 같은 프로세스 안에서 한 번, **프로세스를 다시 띄워 한 번.** 이것이 Phase 2b 전체가 매달린 유일한 관측이다.
2. ★**재기동 뒤 `thread/resume` 한 서버가 진행 중이던 턴의 상태를 다시 알려 주나**★ — **1 과 같은 자리에서 잰다.** ★**이 행이 2a 에서 지던 하중이 없어졌다**★ — 3판은 §3 조정 규칙 4 가 이것 위에 서 있었는데 그 규칙이 걷혔다(ADR-0192). 지금 이것을 요구하는 것은 **Phase 2b 의 resume 배선**뿐이고, 2a 는 그 답이 어느 쪽이든 설계가 안 바뀐다. **그래도 같은 왕복에서 공짜로 얻으므로 잰다.**
3. ★**codex 가 유저 메시지를 replay 로 되울리나**★ — §10-15(합성 에코의 shape)가 이것에 매달린다. ★`userMessage` 아이템이 실재하는 것까지는 실측이다★(§2 L8) — 모르는 것은 **재부착 시 그것이 다시 오나**다.

★**그리고 이 게이트에 스키마 판독 항목 둘을 함께 싣는다 — 왕복이 아니라 읽기지만 같은 자리에서 막힌다**★:

1. ★**`ThreadItem` 20 변형의 목록**★ — 조사 보고서가 개수만 적고 열거하지 않았다(F8). §6-1 번역 표의 그 행이 채워지지 않는 유일한 이유이고, **열거 없이 번역기를 짜면 어느 변형이 조용히 버려지는지 아무도 모른다.** 판정 = 20개 이름과 각 payload 모양을 적어 §6-1 에 행으로 넣는다.
2. **알림 81종의 전체 명단** — ★관측된 것은 11 종이다★(§2 L7). 판정 = 명단을 얻어 §6-1 에 안 든 것이 무엇인지 확정한다.

★**이 둘은 codex 를 돌릴 필요가 없다 — 로컬 스키마 생성이 된다**★(`codex app-server generate-json-schema --experimental --out <dir>`, 종료코드 0, **네트워크·모델 호출 없음** — §2 L14). 그래서 `--ignored` 레인이 아니라 **착수 전 판독**으로 처리할 수 있고, 재기동 왕복보다 먼저 끝난다.

★**그리고 도구를 실제로 쓰는 턴을 이 게이트에 하나 더한다**★ — 2a 실측이 읽기 전용·자잘한 프롬프트로만 돌아서 **① 명령 출력이 조각으로 흐르나 ② 승인 요청 11 종이 실제로 오나**가 둘 다 미측정이다(§11). ★그 둘이 §10-6-b ② 와 §5-2 첫 행의 근거를 만든다.★
**어떤 결과가 ADR-0185 결정 2 를 뒤집나 — ★네 행 중 셋은 이미 답을 받았다★**

★**그 표는 §1 「2a 의 첫 게이트가 무엇을 뒤집을 수 있나」가 정본이고 여기 베끼지 않는다**★ — 앞 셋(응답에 id 가 없다 · UUID 가 아니다 · 턴마다 재발급된다)이 전부 「안 흔들렸다」로 닫혔다`[실측]`.

**남은 한 행:**

| 관측 | 결정 2 | 결정 1 |
|---|---|---|
| `thread/resume` 이 재기동 뒤 **안 선다** | 수령은 성립하지만 **쓸 데가 없어진다**(2b 전체) · `session.resume` capability | **안 건드린다** — 「복원이 무엇에 의존하나」의 답이라 복원이 불가능해도 그 문장은 안 틀린다. 대신 **그 축의 기능이 없어진다** |

★**결정 1 이 어느 행에서도 안 흔들리는 이유**★ — 그것은 **스키마(`ThreadStartParams` 에 id 계열 속성 0개) + 상류 거절** 위에 서고, 위 네 행은 전부 **응답 쪽** 관측이다. ADR-0185 자신이 그 구별을 적어 두었다(「거기서 어긋나면 결정 2가 흔들린다(결정 1은 스키마·상류 거절 위에 서므로 그대로다)」).

### 9-2. 어디서 도나 — ★Phase 0 시험대를 늘린다★

**자리:** `crates/engram-dashboard-agent/tests/backend_contract.rs` — **존재 확인함**(114KB · `#[ignore]` 14건 + 비-ignore 선언 항목 1건 `declaration_table_is_filled_for_every_backend`). ★**두 번째 하네스를 만들지 않는다.**★

- 그 파일 헤더가 「이 파일이 소유하는 것은 질문표 하나다. 백엔드가 셋째로 늘면 `backend_table` 에 **행이 하나 늘고** 질문 함수들은 그대로다」로 확장 규율을 적는다. ★**app-server 는 백엔드가 아니라 같은 백엔드의 둘째 통로라 「행」이 아니라 「질문」이 는다**★ — 그 구별을 지켜야 표가 뒤틀리지 않는다.
- **레인 규율은 그대로 탄다**: `#[ignore]` + `cargo test -p engram-dashboard-agent --test backend_contract -- --test-threads=4 --ignored` + ★그 레인 안에서 바이너리 부재 = 실패★(조용한 skip 금지 — 그 파일 헤더가 그 사고를 적는다: Windows 에서 CLI 는 `cmd.exe /c` 로 감싸져 뜨므로 바이너리가 없어도 `cmd.exe` 는 반드시 성공한다).
- **비-ignore 선언 항목은 CI 에서 그대로 돈다** — app-server 쪽 선언(통로 모양·인코더 태그·capability)도 그 표에 실어 **CI 신호를 받게** 한다.
- ★**안전 봉투를 app-server 축으로 다시 세워야 한다 — 그리고 실측이 그 축이 달라진다는 것을 확인했다**★. 그 파일의 `SafetySwap` 이 운영 argv 의 **정책 값 두 칸**(`-s`·`-a`)을 무해한 것으로 덮고, ★덮을 플래그를 argv 에서 못 찾으면 그 자리에서 실패★한다(헤더 규율). ★**app-server 경로에서는 그 정책이 argv 가 아니라 `thread/start` 의 params 로 간다**★`[실측]` — 우리가 보낸 것이 `{cwd, sandbox, approvalPolicy}` 이고 그 값이 응답에 그대로 되돌아왔다(§2 L4). ★**즉 argv 를 덮는 봉투는 이 레인에서 덮을 것을 못 찾아 실패한다**★ — 봉투의 모양 자체를 **params 축**으로 다시 세워야 한다.
  - ★**그리고 그 봉투를 이제 **지금** 만들 수 있다 — 3판에서 이 항목을 막던 「값이 미정」이 풀렸다**★. 운영값이 ADR-0192 로 정해졌으므로(승인 **안 묻기** + **작업폴더 쓰기** — §10-5) 봉투가 덮을 대상이 확정된다: ★`thread/start` params 의 `sandbox`·`approvalPolicy` 두 칸을 **무해한 쪽**(읽기 전용 + 안 묻기)으로 덮고, **그 두 칸이 params 에 없으면 그 자리에서 실패**한다★ — argv 판의 규율을 축만 바꿔 그대로 옮긴 것이다.
  - ★**덮는 값이 곧 이미 실측된 조합이라는 것이 이 봉투의 값어치다**★ — `sandbox:"read-only"` + `approvalPolicy:"never"` 는 2a 실측이 실제로 돈 조합이고 그 조건에서 **서버 요청이 0 건**이었다(L11). 즉 이 레인은 **승인 왕복이 없는 것이 관측된** 조합 안에서 돈다.
  - ★**그 대가도 적어 둔다**★ — 봉투가 운영값을 덮으므로 ★이 레인은 「운영값(쓰기 허용 + 안 묻기)에서 무슨 일이 일어나나」를 **재지 못한다**★. 그것은 §11 의 미측정 행으로 남고 사람 눈 레인에서만 볼 수 있다.

### 9-3. 자동 단언 · 사람 눈 — 갈라 적는다

| 자동으로 단언되는 것 | 사람 눈으로만 판정되는 것 |
|---|---|
| `initialize` 왕복(이미 `[실측]` — 회귀로 고정) | **챗 표면에 codex 응답이 사람이 읽을 형태로 흐르는가** — 델타 누적·마크다운·코드블록 |
| `thread/start` → id 수령, 그 id 가 `Uuid` 로 파싱되나(9-1) | **추론 텍스트가 답과 구별되어 보이는가**(§6-1 의 미정 축이 화면에서 어떻게 보이나) |
| ★**통로 구현체가 persist 전 턴 시도를 거절하나**★ — §6-3 의 게이트. 순수 하네스에서 반영 콜백을 **아직 안 돌려주고** 입력을 밀어 넣어 `turn/start` 가 **안 나가는 것**을 단언한다(codex 불필요). ★초판의 「`agents.json` 에 값이 실렸는지 파일로 확인」은 이 자리에 못 쓴다★ — 그건 **값이 결국 도착했다**를 증명하지 그것이 **첫 턴보다 먼저** 도착했다를 증명하지 않는다. ★**그리고 단언의 천장을 §6-3 이 내렸다 — 이 행이 잴 수 있는 것은 「반영 호출이 돌아왔다」까지다**★: `observe_session_id` 의 `bool` 은 메모리 변경 여부이고 그 안의 `store.save` 는 실패를 `tracing::error!` 로 삼킨다(`crates/engram-dashboard-agent/src/profile.rs:400-409` · `:310-311` · `crates/engram-dashboard-agent/src/persistence/mod.rs:102-103`). **「디스크에 있다」를 단언하는 항목은 만들 수 없다** | **빈 상태에 무엇이 뜨는가**(§7-3 갭 3 — (나)를 고를 때만) |
| **반영 호출이 돌아온 뒤에는 턴이 나가나** — 같은 하네스의 반대 방향(게이트가 영구히 막는 회귀 방지) | |
| ★**모르는 인바운드 요청에 유계 오류 응답이 나가나**★(§6-2) — 순수 하네스에 `{"method":"모르는이름","id":N,…}` 을 넣고 **그 `id` 로 응답이 나가는 것**을 단언한다. 짝 = `id` 없는 모르는 알림에는 **아무것도 안 나가는 것** | |
| ★**codex 알림이 `TurnSignal` 로 분류되나**★(§7-4 ①②) — 번역 표 골든의 짝. `turn/started` → `Progress` · `turn/completed` → `Ended` 가 **분류자를 통과**하는 것까지 단언한다(골든이 `OutputEvent` 까지만 재면 ①이 빠진 것을 못 잡는다) | |
| ★**유계 쓰기가 큐 포화에서 `Err` 로 거절하나**★(§4-9) — 순수 하네스. ★짧은 `Ok` 로 신고하지 않는 것★도 함께 단언한다(`WriteOutcome` 의 바이트 칸은 동어반복 — `crates/engram-dashboard-agent/src/types.rs:663-671`) | |
| ★**한 턴의 첫 tag1 프레임이 `turnDone` 을 내리나**★(§7-1 항 2 · §10-2 (가) ③) — ★**이 행만 레인이 다르다**★: 프론트 단언이라 `src/components/slot/structuredAccumulator.test.ts` + `npm test`(vitest)이고 위 Rust 항목들과 같은 명령으로 안 돈다 | |
| 재기동 후 `thread/resume` 이 서나(9-1) | **턴 경계·대기 인디케이터가 codex 에서 맞는가**(§7-1 의 셋이 화면에서 어떻게 보이나) |
| 비-JSON 라인을 섞어도 죽지 않나(§4-6) — 순수 하네스에서 주입 | **승인 요청이 왔을 때 화면이 멈추지 않는가** — 2a 정책의 결과가 보이나 |
| ★우리 id `0` 과 서버 id `0` 이 섞여도 응답이 안 엉키나★(§4-4) — 순수 하네스 | 리사이즈·재부착(replay) 후 챗 화면이 온전한가 |
| EOF 시 대기 RPC 가 오류로 깨고 활성 턴이 미확정이 되나(§3) — 순수 하네스 | |
| stdin 닫기 → exit 0 + Job Object 트리 비움(§4-8, kill 인과 회귀) | |
| 번역 표 골든 — codex 알림 샘플 문자열 → `OutputEvent`(§6-1) — 순수 하네스. ★**중단 케이스를 함께 잰다**★ — `item/started` 가 `item/completed` 없이 남는 실측(§2 L10)이 골든의 한 행이어야 한다 | |
| `MessageDone` 이 턴마다 정확히 1회(§10-2) | |
| ★**턴 끝 타입이 실제로 나가나**★(§10-6) — 새 `StructuredEvent` 타입 + 「어떻게 끝났나」. 짝 = 프론트에 **모르는 종류 가지**가 있어 그것이 화면에 닿는 것(vitest 레인) | |
| ★**「우리가 보낸 것」 표시가 붙나**★(§10-2) — 번역기가 되울린 유저 항목에 그 표시를 달고, 프론트 dedup 이 **uuid 가 아니라 그 표시**를 읽는 것. ★두 레인에 걸친다★(Rust 골든 + vitest) | |
| ★**입력 큐가 순서를 지키고 버리지 않나**★(§4-10) — 순수 하네스. 준비 전에 셋을 넣고 준비 신호 뒤 **셋이 순서대로** 나가는 것 · 턴 중 입력이 턴 종료 뒤 나가는 것 · **상한 초과가 `Err`** 인 것(짧은 `Ok` 금지 — §4-9). ★claude json 경로에도 같은 단언을 건다 — 공통이기 때문이다★ | |
| ★**codex 큐의 해제가 코어의 미관측을 idle 로 읽지 않나**★(§4-10 규율 4) — 순수 하네스. 코어의 사실 표를 **비운 채로**(= `turn_fact` 없음 = `busy.rs:177-180` 이 `false` 를 돌려주는 상태) `turn/start` 를 하나 내보낸 뒤 둘째 입력을 넣고 **그것이 안 나가는 것**. ★이 단언이 없으면 fail-open 회귀가 「가끔 두 개가 겹친다」로만 나타난다★. ★**짝 = 동시성 쪽**★ — `send_input` 을 **두 스레드에서 동시에** 부르고 **`turn/start` 가 하나만 나가는 것**(판정과 전이가 한 락 안 — §4-10 규율 4). 그 락이 갈리면 같은 겹침이 「가끔」으로 돌아오고, 앞 단언만으로는 안 잡힌다 | |
| ★**담긴 뒤의 write 실패가 출력 스트림으로 나오나**★(§4-10) — 순수 하네스. 큐에 담겨 `Ok` 를 돌려준 뒤 writer 를 실패시키고 **`Error{message}` 가 `core.emit` 으로 나가는 것**. ★**「연결이 끊김으로 올라가 세션이 terminal 로 착지하는 것」은 이 행에서 걷었다**★ — 그 착지를 실행할 주어가 없다(§4-9 · 사용자 결정). ★그래서 이 단언이 재는 것은 **화면·LLM 이 그것을 본다**까지이고, 「호출자가 듣는다」는 **어느 단언도 못 잰다**★(§11 의 한계 행) | |
| ★**읽기 스레드가 stdin 락에서 안 멈추나**★(§4-2) — 순수 하네스. writer 를 블록시킨 상태에서 인바운드 요청을 밀어 넣고 **읽기가 계속 도는 것**(다음 줄들이 소비되는 것). 짝 = 그 응답이 writer 큐에 담기고, **큐가 차면 `Err` 로 거절되고 그 거절이 로그에 남는 것**(★세션을 내리는 착지는 단언하지 않는다 — 그런 동사가 없다, §4-9★) | |
| ★**EOF 에서 재부착이 **안** 나가나**★(§3 · ADR-0192) — 순수 하네스. 우리 kill 이든 예상 못 한 EOF 든 **둘 다** 새 프로세스·`thread/resume` 요청이 나가지 않고, 대기 RPC 가 오류로 깨고, 세션이 terminal 로 착지하는 것. ★3판이 여기 두었던 「읽기 함수가 조정을 멈추나」는 걷었다 — 멈출 조정이 없다★ | |
| ★**우리 kill 과 예상 못 한 EOF 가 서로 다른 `TerminalReason` 으로 착지하나**★(§3 마지막 절) — 순수 하네스. 통로 **사유 플래그**만으로 갈리는 것을 재고, ★`AgentTransport` 표면이 여섯 그대로인 것★도 함께 본다(선언 판독) | |
| ★**claude·shell 경로의 동작 변화가 0 인가**★(§5-1 · ADR-0191) — 조립점 접기의 수락 조건. 기존 통로 선택 단위 테스트(`select_transport` 격리 하네스 — `manager.rs:66-67` 이 그 존재 이유를 적는다)가 **판정이 한 건도 안 바뀌고 초록인 것** + 선언 표 트립와이어가 claude·shell 행에서 그대로 초록인 것 | |
| ★**codex 가 셋째 `transport_shape` 를 실제로 선언하나**★(§5-5) — 선언 표 트립와이어가 그것을 잰다(`backend/mod.rs:800-802`). ★이 단언은 **비-`#[ignore]` 레인**이라 CI 신호를 받는다★(§9-2). 짝 = `output_decoder` 가 `Some` 인 것(같은 표의 둘째 칸) | |

★**오른쪽 열은 CI 가 못 한다**★ — 창이 필요하고 실 codex 가 필요하다(CLAUDE.md 「CI」의 그 두 항목). 절차는 CLAUDE.md 「GUI 실측」 규율을 그대로 탄다 — **앱을 셸에서 직접 띄우지 않는다**(프로세스 트리 밖 + 출력은 파일로만). 구체 절차는 `/qa` 바인딩 §full 이 갖고 여기 되올리지 않는다.

★**오른쪽 열 없이 완료를 주장하지 않는다**★ — CLAUDE.md 「구현 실행 규약」: 테스트·타입체크 통과 ≠ 완료.

★**그리고 위 게이트가 덮지 않는 것을 여기 적어 둔다**★ — 「응답 수령 → persist 호출」 사이에서 **프로세스가 죽었을 때의 복구**는 자동 단언도 사람 눈도 못 잡는다. 그 창은 게이트 **앞**에 있고(§6-3), 그 결말은 **저장된 옛 sid 로의 조용한 재부착**이다(같은 절). ★그 복구는 `[미확인]` 으로 §11 에 남는다 — 2a 가 닫는다고 적지 않는다.★ 그리고 ADR-0185 가 「첫 턴 전 persist」를 성립한 불변식으로 인용하는 것을 금했으므로, 위 두 단언이 초록이어도 **그 문장을 불변식으로 옮겨 적지 않는다.**

### 9-4. 모듈 격리 — ADR-0012 강제

**모든 모듈은 외부 의존을 seam 으로 끊어 단독 실행 하네스를 갖는다.** 2a 에서 그 요구가 가장 굵게 걸리는 셋:

| 모듈 | 무엇을 끊나 | 단독으로 무엇을 잰다 |
|---|---|---|
| **통로 구현체의 상관 부분**(보낸 요청 ↔ 도착한 응답) | 실 프로세스 · 실 파이프 | 바이트 in / 바이트 out. id 공간 분리(§4-4) · EOF 시 대기 RPC 실패(§3) · `-32001` 재시도(§4-5) · 비-JSON 라인 무시(§4-6). ★**여기가 이 단계의 회귀망 본체다**★ |
| **번역기** | 실 codex | codex 알림 JSON 문자열 in → `OutputEvent` out. 골든 |
| **통로 구현체의 상태기계** | 시간 · 프로세스 · 실 codex | 전이만. 미확정이 EOF 에서 실제로 기록되나 · persist 전 턴 시도가 거절되나(§6-3·§9-3) · ★EOF 에서 재부착이 **안** 나가나★(§3 · ADR-0192 — 3판의 「조정 전 재시도가 막히나」를 대체한다) · 턴 중 입력이 큐에 머무나(§4-10). ★**넷 중 둘만 여기서 잰다**★ — 프로세스 기계는 이미 `StdioTransport` 것이고 스레드 신원 축은 프로필 레지스트리 것이라 각자 기존 하네스가 있다(§8 의 배치 표) |

★**셋이 실 codex 없이 도는 것이 핵심이다**★ — `#[ignore]` 라 그 레인은 **기본 회귀·CI 에서 구조적으로 빠지고 부를 때만 돈다**(`trd.md` §3-2 의 그 결정). ★**초판의 「codex 가 있는 PC 에서만 돈다」는 틀렸다 — 두 줄 위에 적힌 규율과 정면으로 부딪힌다**★: 그 레인은 **부르면 어디서든 돌고, 바이너리가 없으면 skip 이 아니라 실패한다**(§9-2 · 그 파일 헤더 `crates/engram-dashboard-agent/tests/backend_contract.rs:14-17` — 「이 레인을 부른 사람은 '재겠다' 고 말한 것이라 조용한 초록을 돌려주면 안 된다」 · `trd.md` §3-2 의 같은 문장). **바뀌는 판정은 없다** — 실 codex 를 요구하는 단언에만 의존하면 **CI 신호가 0 인 스위트**가 된다는 결론은 「CI 가 그 레인을 안 부른다」 하나로 이미 선다. 위 셋은 인메모리라 CI 에서 그대로 돈다 → `-- --test-threads=4` 를 붙이지 않는다(CLAUDE.md 「병렬은 테스트 바이너리마다 걸린다」의 판정 규칙 — 자식 프로세스를 하나도 안 띄운다).

---

## 10. 결정 대장 — ★무엇이 닫혔고 누가 닫았나 · 무엇이 열려 있나★

★**1·2 판의 이 절은 「이 문서는 아래 중 어느 것도 고르지 않는다」로 시작했다. 그 문장은 더 이상 참이 아니다**★ — 그 사이에 **ADR 넷**이 핵심을 닫았고, **사용자 결정**과 **메인 판정**이 몇 개를 더 닫았다. 그래서 이 절은 이제 두 가지를 한다: **닫힌 것은 「누가 닫았나」와 함께 적고**, **안 닫힌 것은 열린 채로 둔다.**

★**규율은 그대로다**★(CLAUDE.md 「개발 스텝」): **사용자가 체감하는 것**(동작·정책·데이터 위치)은 이 문서가 고르지 않는다 — 아래 「열림(사용자)」 행이 그것이다. **안 보이는 내부 구현**(이름·상수·코드 배치)은 메인이 정하고 보고한다. ★**아래 「메인 판정」 행을 사용자 결정으로 읽지 말 것 — 반대도 마찬가지다.**★

### 10-A. 닫힌 것

| # | 항목 | 무엇으로 닫혔나 | 결론 |
|---|---|---|---|
| **8** | ★**app-server 가 맞는 경로인가**★ | **ADR-0187**(사용자 결정 + 세 모드 실측) | **app-server 다.** `codex exec --json` 은 ★글자를 흘리지 않는다★ — 도착 이벤트 넷이고 본문이 완성된 채 한 번에 온다(중단 시점까지 두 줄). 공식 SDK 는 ★존재하지 않는다★(`codex --help`·설치 패키지 검색 0건). ★**1·2 판이 「가장 위」로 세워 둔 전제가 여기서 답을 받았다**★ — 그래서 §3~§6 은 다시 열리지 않는다(그 ADR 의 「다시 열 트리거」 둘 제외) |
| **1** | ★**통로의 모양**★ | **ADR-0189**(사용자 판정 + 소유권 추적 + 주석 원문 재독) | **`backend/codex/` 안의 `AgentTransport` 구현체.** 공용 계약 여섯은 그대로, 상관은 구현체 안에서 끝난다. ★2 판 리뷰의 BLOCK 이 뒤집힌 근거는 **주석의 주어가 `StdioTransport`** 라는 재독이고, 그 재해석은 **사용자 판정**을 받았다★(§5-4) |
| **14** | ★**인바운드를 어느 경로로 보나**★ | **같은 ADR-0189** | ★**소멸했다**★ — 상관이 구현체 안에서 끝나므로 **층간 간선이 0** 이다. 1·2 판이 재던 셋((ㄴ) decoder 사설 채널 · (ㄷ) `OutputCore::subscribe` · (ㄹ) 세션 대기표)은 전부 **상관을 밖으로 내보내야 할 때만** 필요한 값이었다(§5-2). ★되꺼내지 말 것★ |
| **2** | 갭 1 — claude 관례가 프론트 누산기에 박힌 것 | **사용자 결정** | ★**중립 어휘 계약을 테스트로 박는다**★(1·2 판의 (가)). ★**「걸러 낸다」가 아니다**★ — 되울린 유저 항목은 **재부착 시 화면을 되살리는 재료**라 버리면 안 된다. **번역기가 그것을 「우리가 보낸 것」으로 표시하고 프론트는 그 표시만 읽는다.** 박을 단언 = ①「`MessageDone` = 턴 경계, 턴마다 정확히 1회」 ②「새 턴 시작 신호」 ③「한 턴의 첫 tag1 프레임은 `turnDone` 을 내린다(또는 내지 않는다)」 — ★③만 레인이 다르다★(vitest · `src/components/slot/structuredAccumulator.test.ts`). ★**claude 쪽은 「바꾸는 것」이 아니라 「이미 참인 것을 테스트로 적는 것」**이고, 프론트 해석의 전면 회수는 여전히 Phase 3 다★ |
| **6** | 데이터 칸 — `StructuredEvent` 가 변형을 얻나 | **사용자 결정** | ★**타입 하나를 더한다 — 「턴 끝 + 어떻게 끝났나」**★. **`PROTOCOL_VERSION` 은 올리지 않는다.** ★1·2 판이 이 행에 적은 「새 변형은 명부를 통째로 잃는다」는 **이 축에 안 걸린다**★ — 그 사유(v4)는 **디스크 파일의 에이전트 종류 enum** 얘기이고, **화면으로 흐르는 이벤트 enum 은 모르는 종류를 조용히 통과시킨다.** 즉 타입 추가가 싸다. 그리고 「한 턴에 완료 항목이 여럿」이 ★실측★이라 **턴 끝 ≠ 메시지 끝**이고 별 타입이 의미상 맞다. ★**대가 하나를 함께 적는다 — 그리고 그 「조용히」의 뜻을 정직하게 적는다**★. **프론트에 「모르는 종류」 가지가 없다**(누산기 switch 에 `default:` arm 이 없다 — `src/components/slot/structuredAccumulator.ts:66-138`). ★**Rust 쪽에서만 조용한 통과이고, 프론트에서는 조용하지 않다**★ — 모르는 `type` 은 항목이 안 들어가고 `turnDone` 도 안 움직이는데 **호출자는 무언가 온 것처럼 행동해 `setAwaiting(false)` 를 발화한다**(`src/components/slot/RichSlot.tsx:167`). 즉 결말이 **항목 소실 + 스피너 소등**이고, 릴리스 WebView2 에는 devtools 가 없어 `console` 도 못 본다(§6-2 의 픽셀 추적). ★**그리고 그 창이 드문 조합이 아니다**★ — 신데몬 + 구셸은 **로컬 개발에서 일상**이다(데몬이 셸 재빌드보다 오래 살아서 그렇다 — `crates/engram-dashboard-protocol/src/lib.rs:74-76` 이 그 사실을 적는다). ★**판정(버전 미bump)은 그대로 유지한다**★ — 그 창을 닫는 것은 bump 가 아니라 **`default:` 가지 하나**이고(§8 의 그 행), 이 결정이 그 가지를 **이 단계의 비용으로** 함께 샀다. 나머지 축 셋(추론 · 도구 결과 · 중립 제품 라벨)은 이 결정에 안 들었고 아래 10-B 에 남는다 |
| **3 · 11** | capability **독자**를 만드나 · **신고**를 정직하게 하나 | **메인 판정** | ★**2a 는 capability 독자를 만들지 않는다. 값은 그냥 정직하게 신고한다.**★ 근거(실측 둘): ① **챗 표면은 resize 를 한 번도 부르지 않는다** — 운영 호출 넷이 전부 `TerminalSlot.tsx` 안이고 `RichSlot.tsx` 에는 한 줄도 없다 ② ★**`canInterrupt` 는 화면에서 읽는 코드가 0 이다**★ — `src/components/agent/mergeTreeNodes.ts` 의 세 줄(`:55` 선언 · `:94` 대입 · `:111` 예약 기본값)이 전부이고 그 값을 소비하는 컴포넌트가 없다. ★그 절반(사람이 누를 표면)은 **T-34** 가 진다★. ★★**신고 레버는 ADR-0191 이 닫았다 — 3판이 10-B 에 열어 둔 `11-a` 는 없어졌다**★★: 구현체는 caps 를 **하드코딩하지 않고 생성 시 주입받고**, 그 **주입값을 만드는 것은 그 백엔드의 통로 생성 코드**이며, **조립점은 그 값을 만들지도 통로의 구체 타입을 알지도 않는다**(세 문장의 정본 = §5-5). ★그래서 「`StdioTransport::open` 을 넓힌다」도 「새 struct 의 생성자」도 정확한 답이 아니다★ — 레버는 `backend/codex/` 안이다 |
| **5** | 2a app-server 통로의 **기동 정책값** | **ADR-0192**(축은 여전히 ADR-0188) | ★**값이 정해졌다 — 3판에서 「후속 대기」로 비어 있던 자리다.** 기준은 「백엔드 기본값을 claude 와 맞춘다」이고, 오늘 claude 는 승인을 끄고 돈다★. 그래서 **승인 = 안 묻기 · 쓰기 = 작업폴더 안까지 허용.** ★글자 그대로 claude 와 동일은 샌드박스 해제까지 가므로 **한 칸 좁혀** 작업폴더 쓰기에 세웠다★. ★★**그리고 이 통로에서 그 값이 가는 곳은 argv 가 아니라 `thread/start` 의 params 다**★★`[실측]` — `{cwd, sandbox, approvalPolicy}`(§2 L4). ★1·2·3 판이 「오늘 코드가 넣는 값 그대로(`-s workspace-write -a on-request`)」로 적은 것은 **PTY 경로의 argv** 이고 이 통로에서는 **가리키는 실물이 없다**★ — 그 argv 는 §8 의 `build_spec` 행이 따로 진다. **`--cd` 도 안 쓴다** — 워크스페이스 폴더가 같은 params 로 간다. ★**잠정이다**★ — ADR-0188 의 후속 값 설계가 이것을 **공용 선언 형태**(선언은 백엔드 중립, 내부 실행만 각 백엔드)로 접을 **여지를 남긴다.** ★**그리고 이 조합은 미측정이다**★ — 실측된 무승인 조합은 **읽기 전용** 쪽이고(L11 의 서버 요청 0 건이 그 조건에서 나왔다), 「쓰기 허용 + 안 묻기」에서 승인 요청이 오나는 §11 이 진다 |
| **9** | app-server 경로의 LLM 표면을 여나 | **메인 판정** | ★**지금 열지 않는다**★ — **권한 축이 정해진 뒤에 연다.** 지금 열면 **하드코딩된 권한 위에서 LLM 이 에이전트를 띄우게 된다**(ADR-0188 이 그것을 구조 결함으로 지목한 바로 그 상태). ★열 때의 크기는 그대로 셋이다★ — `LLM_BACKEND_POLICY` 표 한 줄 + `AgentBackend` 어휘에 `Codex` + `backend_command` 짝(`crates/engram-dashboard-agent/src/commands.rs:300-315`), 그리고 `agents.json` 이주 검토 |
| **10** | codex 가 kill 전에 유계 graceful close 를 받나 | **메인 판정** | ★**안 한다**★ — **턴 도중 stdin 을 닫으면 어떻게 되는지가 미측정**이다(측정된 것은 「턴 없을 때 닫으면 46ms 에 종료 0」). 근거 없는 문을 내지 않는다. ★그 문의 값이 왜 싸지 않은지는 §4-8 이 그대로 갖는다★ — `AgentTransport` 에 `close_stdin` 이 없고, 유일한 `stdin.take()` 는 kill **뒤** best-effort 이며, `ControlCaps.graceful_shutdown` 은 세 구현체 전부 false 인 채 **읽는 분기가 워크스페이스에 하나도 없다.** ★**어느 답이든 `transport.shutdown()` 안은 한 줄도 안 바뀐다**★ |
| **12** | 「턴이 미확정으로 끝났다」를 재기동 뒤까지 나르나 | **메인 판정 + ADR-0192** | ★**안 나른다 — 그리고 이제 나를 곳도 없다**★. 다리 둘: ① **소비자 부재** — ADR-0192 가 「EOF 뒤 재연결」을 걷어서 그 값을 읽고 행동할 코드가 아예 없다 ② **persistence 위험** — 스키마를 올리면 불일치가 **백업 없이 빈 목록으로** 떨어지는 경로를 지나고, 화신 귀속을 디스크가 표현 못 하며, 저장 실패가 삼켜져 「썼다」를 확인할 수 없다(§3 의 표가 정본). ★**1·2·3 판의 「매번 재부착 후 재관측한다」는 걷었다**★ — 재부착이 없어졌으므로 재관측이라는 대안 자체가 없다. 미확정은 **로그 한 줄**로 끝난다 |
| **13** | 「우리 kill 이냐」를 물을 자리를 공용 계약에 내나 | **메인 판정(3판 판정을 뒤집음)** | ★**안 낸다 — `AgentTransport` 는 여섯 그대로다**★(ADR-0189 준수). 3판은 「결함 수정으로 읽기 함수를 하나 더한다」로 닫았는데 ★**그 근거 둘이 다 죽었다**★: ① 「세 구현체 전부가 이미 그 비트를 갖고 있다」가 **거짓** — `pub struct ApiTransport;` 는 필드가 하나도 없다(`crates/engram-dashboard-agent/src/transport/api.rs:14` · `fn shutdown(&self) {}` `:50`), 그래서 계약에 함수가 생기면 `Unsupported` 가 늘고 그것이 그 ADR 이 기각한 모양이다 ② 「안 가리면 죽는 길에 새 프로세스를 띄운다」의 그 조정 규칙이 **없어졌다**(ADR-0192). ★**구별 자체는 여전히 필요하고 통로 사유 플래그로 족하다**★ — codex 통로도 자기 읽기 스레드를 소유하므로 `StdioTransport` 와 같은 모양(`transport/stdio.rs:46` ↔ `:251`·`:271`·`:340`)을 그대로 쓴다. ★3판이 여기 적어 둔 「ADR 과의 긴장」은 없어졌다 — 계약을 안 건드리므로 부딪힐 문장이 없다★ |
| **18** | ★**그 통로를 누가 만들어 조립점에 넘기나**★ | **ADR-0191** | ★**백엔드가 만들어 넘긴다. 가르는 switch 는 `backend_for(c)` 하나다**★ — 조립점이 국면마다 그 switch 를 다시 타는 여섯 줄(`crates/engram-dashboard-agent/src/manager.rs:1033-1043`)이 한 번의 호출로 접히고 통로가 그 묶음에 실린다. ★**기각된 갈래 = `select_transport` 의 중앙 `TransportShape` match 에 셋째 갈래를 더하는 것**★(`manager.rs:79-99`) — 가르는 자리를 둘로 쪼개고, 자기 주석이 「이 함수는 어느 backend 도 이름으로 모른다(ADR-0004)」라 적은 함수(`:75-76`)가 백엔드 타입을 이름으로 알게 된다. ★**단 그 귀결이 「셋째 `TransportShape` 변형도 안 세운다」는 아니다**★ — ADR-0191 은 그 생성 match 를 **모든 백엔드에서** 걷으므로 비-exhaustive 가 될 match 가 안 남고, 변형은 **선언 축**으로 그대로 선다(ADR-0189 「영향」). ★없어지는 것은 match 이지 enum 이 아니다★. **claude 경로의 값·순서는 그대로이고 동작 변화 0 이 수용 조건이다**(§9-3 이 그 단언을 진다) |
| **19** | ★**예상 못 한 EOF 뒤에 같은 통로가 다시 붙나**★ | **ADR-0192** | ★**안 붙는다 — claude 도 안 한다**★. 3판의 §3 조정 규칙 4·5 를 걷었다. 사유 = 그 설계가 **ADR-0005(finalize 1회)·ADR-0019(reaper 단일 소비자)·Job Object 경계**와 동시에 부딪혔고(§3 「왜 걷었나」), 「새 프로세스를 띄운다」의 소유자가 문서 어디에도 없었으며(`activate_profile` 은 통로가 부를 동사가 아니고 `restart_policy` 는 생산 소비자 0), codex 쪽 근거(#40766·#33241)도 「죽은 것 같으니 하나 더 띄운다」를 사고 모양으로 지목한다. **재기동은 새 화신뿐이고 그 배선은 Phase 2b** |
| **20** | ★**큐 앞에서 입력 영수증(`send_input` 의 `Ok`)을 둘로 쪼개나**★ | **사용자 결정** | ★**안 쪼갠다 — 하나로 퉁친다**★. 사유(사용자 말 그대로) = 「**우편은 보냈고 받는 쪽이 소화한다**」. ★그리고 그것이 오늘의 자세이기도 하다★ — 터미널 경로의 `Ok` 는 「요청 바이트 전량 수용」이고(`crates/engram-dashboard-agent/src/session.rs:150-156` · ADR-0088) **받는 CLI 가 읽었는지·행동했는지는 아무도 확인하지 않는다.** 큐는 그 바를 낮추지 않는다. ★**「담겼다 / 실제로 썼다」 두 등급 신고는 기각**★이고 우편 계층도 그대로 「전달했다」를 적는다. ★★**대신 실제로 바뀌는 것 하나를 한계로 적는다 — 담긴 뒤에 실패한 write 는 그 호출자에게 돌아갈 길이 없다**★★(오늘은 write 가 호출 안에서 일어나 `Err` 로 돌아간다). ★**정정 채널을 발명하지 않는다**★ — §11 의 한계 행. **메커니즘 정본 = §4-10** |
| **21** | ★**무기한 쓰기 정지(stall)에 강제 종료 감시를 만드나**★ | **사용자 결정** | ★**안 만든다**★. 4판은 「그 실패가 **연결 = 끊김**으로 올라가 §3 의 EOF 처분과 같은 착지를 탄다」로 적었는데 ★**그 착지에 실행할 주어가 없다**★ — 막힌 write 는 자식을 살려 둔 채 멈춘 것이라 EOF 를 안 만들고, 세션을 내리려면 **타이머로 산 에이전트를 죽이는 동사**를 새로 만들어야 한다(§3 이 「구현자가 트리거를 발명한다」로 금한 부류). ★**대신 이 저장소가 이미 고른 자세를 따른다 — 「너무 오래 멈춘 것」은 관측을 잔해로 선언하지 죽이지 않는다**★: 우편의 `BUSY_MAX_TURN` 30분 fail-open 상한과 그 sweep 이 그 실물이다(`crates/engram-dashboard-messaging/src/busy.rs:44-57` · `:136-146`). ★**남는 것 = 화면에 아무 신호가 없다(알려진 한계 · §11) + 겪거나 재고 나면 그때 값과 함께 만든다**★. 정본 = §4-9 |

### 10-B. 열린 것

★**아래는 일곱 ADR 과 사용자 결정 셋에 대고 다시 적은 것이다 — 3판에서 하나(11-a)가 닫혔고, 4판에서 하나(17)가 생겼으며, 5판에서 그 17 이 **절반으로 줄었다**.**★

| # | 항목 | 누가 정하나 | 갈림 · 무엇이 좁아졌나 |
|---|---|---|---|
| **17** | ★**담긴 입력에 「대기 중」을 화면에 알리나**★ | **사용자**(체감 동작) | ★★**절반이 닫혔다 — 에코 *순서* 갈래는 사용자 결정으로 없어졌다**★★. 4판은 이 행을 「에코 시점 + 「대기 중」 표시」로 이고 있었고 그 절반은 **`Ok` 가 두 뜻이 된다**는 프레이밍 위에 서 있었는데, ★**§10-20 이 영수증을 하나로 못 박아 그 프레이밍이 없어졌다**★ — `Ok` 는 「우리가 맡았다」 하나이고 **에코가 가리키는 사실이 정확히 그것**이라, 에코는 **오늘 자리 그대로**다(`crates/engram-dashboard-agent/src/session.rs:163` → `:175-176`). 「에코를 큐 방출 뒤로 미룬다」 갈래는 **되꺼내지 않는다**. ★**남는 질문 하나**★ = 담겨서 아직 안 나간 입력에 **「대기 중」임을 화면에 표시하나** — 오늘 프론트에 그 개념이 **0** 이라 만들면 새로 만드는 일이고, 안 만들면 사용자는 자기 입력이 **즉시 나간 것과 구별하지 못한다**. 갈림 = ① 표시 없이 간다(오늘 모양) ② 그 항목에 대기 표시를 붙인다. ★**이 문서는 고르지 않는다**★. 메커니즘의 정본 = §4-10 |
| **4** | 2a 에서 codex 를 **챗 표면에 그리나** | **사용자** | (가) 안 그린다 / **(나) 그리고 브랜딩을 중립화한다** / (다) 챗 표면은 2b. ★**좁아졌다 — codex 통로 생성 코드가 caps 에 `output.structured = true` 를 실어 주면 프론트가 `renderMode.ts:24` 에서 `rich` 를 자동으로 고른다**★(ADR-0191 · §5-5. ★셋째 `transport_shape` 변형은 그대로 서지만 그것이 **생성자를 고르지는 않는다** — 값은 선언이고 caps 는 통로 생성 코드가 주입한다★). 즉 **아무것도 안 하면 (나) 로 간다.** 남는 질문은 「그리나」가 아니라 ★**빈 상태의 `Claude Code` 리터럴과 마스코트를 어떻게 하나**★다(`src/components/slot/RichSlot.tsx:297`·`:305`·`:308` · ADR-0145). ★판정 축은 **백엔드 이름이 아니다**★ — `if (backend === 'codex')` 는 위반이다(`src/api/types.ts:110`) |
| **6-b** | 어휘를 **더 늘리나** — 추론 · 도구 결과 · 중립 제품 라벨 | **사용자** | 6 이 닫은 것은 **턴 끝 타입 하나**뿐이다. 남은 축 셋: ① **추론**(`item/reasoning/textDelta`·`summaryTextDelta` — ★기본 설정에서 `reasoning` 아이템이 `summary:[]`·`content:[]` 로 닫혔고 델타가 한 건도 안 왔다★`[실측]`, 그래서 **지금은 채울 내용조차 없다**) ② **도구 결과**(`item/commandExecution/outputDelta` — `ToolCall` 은 호출이라 결과 칸이 없다. ★그런데 그 이벤트가 실제로 오는지가 미측정이다 — 도구를 실행하는 턴을 안 돌려 봤다★) ③ **중립 제품 라벨의 출처**(4 에 매달림). ★**셋 다 「지금 늘릴 근거가 실측으로 서지 않는다」가 공통이다**★ |
| **7** | 새로 만드는 것의 **이름·자리** | **메인**(안 보이는 내부) / 일부 **사용자** | ★**절반이 닫혔다**★ — 모듈이 어디 사나는 ADR-0189 가, 통로를 누가 만들어 넘기나는 ADR-0191 이 정했다. 남는 것 둘: ① `AgentCommand::Codex` 가 통로 모드를 **칸으로** 표현하나 `extra_args` 로 나르나 **별 변형으로 세우나**(데이터 위치라 **사용자**) — ★★**이 행은 2a 구현 진입 *앞에* 답해야 한다**★★(사용자 결정 · §10-C 가 그것을 순서로 진다): **§8 의 `build_spec` 이 argv 를 그 표현으로 갈라야** 하므로 **첫 줄부터 필요하고**, ★**그 선택이 트립와이어 커버리지도 함께 정한다**★ — 선언 표의 키가 **변형당 한 줄**이라(`backend/mod.rs:779`) **칸으로 나르면 둘째 모드가 그 검사를 아예 안 탄다**(§5-5). ★**관측을 기다리는 항목(15) 뒤에 두지 않는다**★ — 여는 것 자체는 규약대로지만(이름·표현은 사용자 몫), **늦게 여는 것**은 구현을 막는다 ② `InputEncoder` 에 셋째 태그를 세우나(★컴파일러가 강제하는 자리 = `submit_sequence()` 의 exhaustive match, `backend/mod.rs:443-446`★ — 내부 구현이라 **메인**). ★`TransportShape` 쪽은 이 항목에 안 든다 — **셋째 변형을 세우는 것으로** 닫혔다★(ADR-0189 「영향」 · §5-1) |
| **15** | ★**codex 도 합성 유저 에코를 내나 · 낸다면 shape 는**★ | **관측 뒤 사용자** | 그대로 열려 있다. `input_echo_event` trait 기본값이 `None` 이라(`backend/mod.rs:209-211`) 선언하지 않으면 **우리가 `turn/start` 를 쓴 순간부터 첫 알림 도착까지 그 에이전트가 「미관측」**이고, 미관측은 게이트에서 idle 로 흡수돼 **그 창에 편지가 꽂힌다**(`crates/engram-dashboard-messaging/src/busy.rs:178-180`). ★**2 의 결정이 이 항목의 모양을 바꿨다**★ — 「우리가 보낸 것」 표시를 번역기가 붙이기로 했으므로, **에코를 낸다면 그 표시를 단 형태**여야 한다. shape 확정은 여전히 관측 대기다(codex 가 유저 메시지를 replay 로 되울리나 — §11) |
| **16** | 통로 구현체가 **「이미 persist 됐나」를 읽는 손** | **메인** | ★**절반이 닫혔다**★ — **쓰는 손**은 ADR-0189 가 정했다(조립점이 준 콜백, 선례 = `crates/engram-dashboard-daemon/src/lib.rs:284-290`). 남는 것은 **읽는 쪽**이다: 그 선례는 **쓰기 한 방향**뿐이라 「이미 기록됐나」를 묻는 동사가 없다. 갈래 = ① 콜백을 두 동사(읽기+쓰기)로 넓힌다 ② 구현체가 **자기가 방금 기록했다는 사실**만 기억하고 디스크를 안 묻는다(★§6-3 이 내린 게이트 조건의 정직한 이름이 이미 「반영 호출이 돌아왔다」라 ②로도 조건이 성립한다★). ★어느 쪽이든 **포트 이름이 중립**이어야 한다 — 「codex 용」이면 ADR-0004 가 샌다★. ★**ADR-0191 이 ②를 조금 더 싸게 만들었다**★ — 통로를 만드는 자리가 백엔드 폴더 안이라 그 콜백을 함께 얹는 자리도 거기이고, 「방금 기록했다」는 기억은 그 객체 안에서 끝난다 |

### 10-C. 남은 질문의 순서

★**5판에서 순서가 바뀌었다 — 맨 앞이 7 ① 이다**★. 4판은 「4 가 첫머리」였고 7 ① 을 **관측 대기 항목(15) 뒤에** 매달아 두었는데, ★**그 배치가 구현 진입을 막는다**★: 7 ① 이 안 정해지면 `build_spec` 이 만들 argv 의 모양이 없다.

1. ★★**7 ① — 구현 진입 앞. 이것 하나가 「먼저」다**★★. 통로 모드를 `AgentCommand::Codex` 의 **칸**으로 나르나 `extra_args` 로 나르나 **별 변형**으로 세우나. 그것이 정해져야 §8 의 `build_spec` 이 `app-server --stdio` 축으로 갈릴 수 있고, ★같은 선택이 **선언 표 트립와이어가 둘째 모드를 검사하나**까지 정한다★(`backend/mod.rs:779` · §5-5). ★**15 뒤에 두지 않는다**★ — 4판의 「15 → 7 ①」 사슬은 「에코 shape 가 같은 자리에서 정해진다」였는데, 그것은 **7 ① 을 늦출 사유가 못 된다**(shape 는 나중에 그 자리에 얹으면 되고, argv 갈림은 첫 줄부터 필요하다).
2. ★**4 — 그 다음**★. 11-a 가 ADR-0191 로 닫히면서 codex 세션의 `output.structured = true` 신고가 확정됐으므로 **프론트는 이미 `rich` 로 간다**(`renderMode.ts:24`). 즉 4 는 「그리나」가 아니라 ★**빈 상태 라벨을 어떻게 하나**★만 남았다.
3. **4 → 6-b ③** — 챗 표면에 그리기로 하면 빈 상태 라벨의 출처가 필요해지고, 그것이 `Capabilities` 쪽이면 어휘가 는다.
4. **15 → 7 ① 의 *뒤에* 얹힌다**(방향을 뒤집어 적는다) — 에코를 내기로 하면 그 shape 가 **1 이 이미 고른 자리**에 실린다. 15 는 관측 대기라 늦어도 되고, 늦어도 1 을 막지 않는다.

- ★**17 은 어디에도 안 매달리고 구현 전에 답이 있어야 하는 것도 아니다**★. 큐가 서는 순간 「표시 없음」이 기본값으로 돌고, 표시는 그 위에 얹는 변경이다. **단 그것도 선택이라는 것을 사용자가 알고 지나야 한다.**

★**그리고 「지금 물을 수 없는 것」이 여전히 있다**★ — **15** 는 절반이 관측 대기이고(codex 가 유저 메시지를 replay 로 되울리나), **6-b ①②** 는 실측이 아예 없다(추론 델타가 안 왔고 도구 실행 턴을 안 돌렸다). ★고르기 전에 재는 것이 §1 이 세운 순서 그대로다 — 그 재기를 §9-1 첫 게이트가 진다.★ ★**단 7 ① 은 그 부류가 아니다**★ — 재서 답이 나오는 것이 아니라 **고르면 되는 것**이라 관측을 기다릴 이유가 없다.

### 10-D. 판이 바뀔 때마다 무엇이 움직였나 — ★사유를 남긴다★

**3판에서(그대로 유효):**

- ★**절의 성격이 바뀌었다**★ — 「사용자 결정이 필요한 항목」에서 **「결정 대장」**으로. 1·2 판은 열여섯을 전부 미결로 이고 있었고, 그 미결의 맨 위(8·1)가 닫히면서 나머지 사슬이 풀렸다.
- **1·8·14 는 ADR 이 닫았다.** 2·6 은 **사용자**가, 3·9·10·11·12·13 은 **메인**이 닫았고 5 는 ADR-0188 로 내려갔다. ★그중 **13 과 5 는 4판에서 다시 움직였다**(아래)★. ★**그 구분을 각 행에 적어 둔 것이 이 판의 규율이다**★ — 「누가 닫았나」가 없으면 다음 세션이 메인 판정을 사용자 결정으로 읽거나 그 반대로 읽는다.
- **§4-9 의 유계 쓰기는 이 절에 없다** — 1·2 판이 그것을 1 의 결과로 적었는데 결정이 아니었다. ★단 **§4-10 의 입력 큐가 그 옆에 새로 섰다**★(ADR-0190) — 그쪽은 결정이 아니라 **확정된 규율**이고, 그 안에서 열린 것 셋(상한 값 · 화면 표시 · 우편 파킹과의 중첩)은 그 절이 진다.

**4판에서 — ★두 ADR 이 하나를 닫고 둘을 뒤집었다★:**

- ★**18·19 가 새로 닫힌 행이다**★ — 통로를 누가 만들어 넘기나(ADR-0191) · EOF 뒤 재연결(ADR-0192). 둘 다 3판이 **결정인 줄 모르고 본문에 처방으로 적어 두었던 것**이고, 적대 리뷰 둘이 각각 BLOCK 으로 그 자리를 짚었다.
- ★**11-a 가 닫혔다 — 열린 지 한 판 만이다**★. 3판이 「생성자 인자 확장」 판정과 코드의 어긋남으로 열어 두었는데, ADR-0191 이 **어느 생성자도 아닌 제3의 답**(백엔드 폴더 안의 통로 생성 코드)을 줘서 어긋남 자체가 없어졌다.
- ★★**13 이 뒤집혔다 — 「고쳤다」에서 「안 고친다」로**★★. 3판은 `AgentTransport` 에 읽기 함수를 더하기로 하면서 「구현체 셋 전부가 이미 그 비트를 갖고 있다」를 근거로 삼았는데 **그것이 거짓이었다**(`pub struct ApiTransport;` — 필드 0). ★**교훈을 남긴다: 「셋 다 이미 갖고 있다」류의 주장은 세 파일을 다 열어 보고 적는다**★ — 이 문서의 다른 자리들은 그렇게 적혀 있었고(`ControlCaps.graceful_shutdown` 세 줄 인용이 그 예) 이 한 자리만 안 그랬다.
- ★**12 의 사유가 하나 늘고 결론 문구가 줄었다**★ — 「매번 재관측한다」가 없어졌다(재부착이 없으므로). 판정은 그대로 「안 나른다」.
- ★**5 가 「옮겨 갔다」에서 「값이 정해졌다」로 바뀌었다**★ — ADR-0192 가 claude 기준의 기본값 하나를 박았다(승인 안 묻기 + 작업폴더 쓰기, **잠정**). ★**중립 축은 여전히 ADR-0188 이 소유하고, 이 값은 그 후속이 공용 선언 형태로 접을 여지를 남긴 채 서 있다**★ — 2a 가 자기 정책을 **따로 발명하면** 그 ADR 을 어기는 것이고, 지금은 발명이 아니라 「claude 와 같게 맞춘 값」이다.
- ★**17 이 새로 열렸다**★ — 큐의 `Ok` 뜻이 바뀌면서 화면에서 보이는 순서가 사용자 결정 거리가 됐다. 3판은 그 축을 아예 안 봤다.

**5판에서 — ★ADR 하나와 사용자 결정 셋★:**

- ★**20·21 이 새로 닫힌 행이다 — 둘 다 사용자 결정이다**★. **20** = 입력 영수증을 안 쪼갠다(「우편은 보냈고 받는 쪽이 소화한다」). **21** = 무기한 쓰기 정지에 강제 종료 감시를 안 만든다. ★**둘 다 4판이 「해결했다」로 적어 둔 자리를 「하나로 유지 + 한계로 명시」로 바꾼 것**★이고, 그래서 §11 의 한계 행이 둘 늘었다.
- ★**ADR-0193 이 ADR-0190 결정 4 를 개정했다**★ — 큐 해제 판정의 주인이 「코어」에서 **각 통로 구현체**로 갔다. ★4판은 그 개정을 **ADR 없이 TRD 본문에서** 하고 있었다★(「codex 에서 코어가 fail-open 이니 구현체가 판정한다」) — 적대 리뷰가 그 절차 위반을 짚었고, 지금은 문서가 **ADR 을 인용하고 사유는 그쪽에 둔다**. 그 자리에서 **락 규율 하나가 새로 명시됐다** — 판정과 「진행 중으로 전이」는 한 락 안(§4-10 규율 4).
- ★**7 ① 이 순서표 맨 앞으로 올라갔다**★ — 여는 것 자체는 규약대로지만, 4판의 순서표가 그것을 **관측 대기 항목 뒤에** 두어 구현 진입을 막고 있었다. ★열려 있는 것과 늦게 여는 것은 다른 문제다★.
- ★**17 이 절반으로 줄었다**★ — 에코 순서 갈래가 20 으로 닫히고 「대기 중」 표시 하나만 남았다.
- ★**§4-10 의 열린 항목이 셋에서 넷으로 갈라졌다**★ — 「큐 길이 상한 + 대기 시간 상한」이 한 줄로 묶여 있었는데, **시간 상한만 ADR-0190 의 「입력을 버리지 않는다」와 부딪힌다**(정당하게 긴 턴이 담긴 항목을 `Err` 로 떨어뜨린다). 길이 상한엔 그 긴장이 없다.
- ★**§10-N 표기를 하나로 고정했다**★ — 같은 행이 `§10-5` 와 「§10-A 표 5 행」 두 이름으로 인용되던 것을 짧은 쪽으로 통일했다(읽는 법 블록).

---

## 11. 무엇이 검증되지 않았나

★**1·2 판의 이 절은 「`codex app-server` 로 스레드를 한 번도 만들어 보지 않았다」로 시작했다. 그 문장은 죽었다**★ — 2026-09-09 에 스레드를 만들었고, 턴을 돌렸고, 중단했고, 같은 스레드로 한 번 더 돌렸다(§2 `[실측]` L4~L11). **그래서 §6 번역 표의 왼쪽 열은 이제 절반이 실측 위에 선다.**

★**그러나 그 실측이 돈 조건은 좁다 — 읽기 전용 + 승인 없음 + 도구를 안 쓰는 프롬프트**★. 아래가 그 밖에 남은 것이고, ★**조건이 바뀌면 가장 먼저 무너지는 것이 「서버 요청 0 건」이다**★.

| 항목 | 상태 | 어디서 닫히나 |
|---|---|---|
| ★**도구를 실제로 실행하는 턴**★ | `[미확인]` — 읽기 전용·자잘한 프롬프트로만 돌렸다 | ★**둘이 여기 매달린다**★: ① **명령 출력이 조각으로 흐르나**(`item/commandExecution/outputDelta`) — §10-6-b ② 가 이것 없이는 못 선다 ② ★**승인 요청 11 종이 실제로 오나**★ — 관측 0 건은 「그런 게 없다」가 아니라 **그 조건에서 안 왔다**는 뜻이다(§2 L11) |
| **승인 왕복** — payload · `availableDecisions` 실제 값 · 응답 봉투 | `[미확인]` — `[문서]` 뿐(D1) | 위 항목과 한 몸. §5-2 첫 행(「오면 method-not-found」)은 **그 왕복을 전제하지 않는 최소 처분**이다 |
| ★**턴 도중에 stdin 을 닫으면 어떻게 끝나나**★ · hard kill 이 rollout 을 자르나 | `[미확인]` — L2 의 exit 0 은 **활성 턴이 없는** 관측이다 | ★**§10-10 이 이 미관측 위에서 「graceful close 를 안 만든다」를 고른 결정이다**★ — 근거 없는 문을 내지 않는다 |
| ★**재기동 뒤 `thread/resume` 한 서버가 진행 중 턴의 상태를 다시 알려 주나**★ | `[미확인]` — `thread/resume` 자체를 안 불렀다 | ★**이 행의 하중이 줄었다**★ — 3판은 §3 조정 규칙 4 와 「매번 재관측한다」가 이것 위에 서 있었는데 **둘 다 없어졌다**(ADR-0192). 지금 이것을 요구하는 것은 **Phase 2b 의 resume 배선**뿐이고 2a 는 답이 어느 쪽이든 설계가 안 바뀐다. §9-1 첫 게이트에서 같은 왕복으로 잰다 |
| ★★**app-server 는 벤더가 「실험적」이라 부르는 표면이고 stdio 도 그 안이다 — 그런데 깨지는 변경을 접속 때 못 잡는다**★★ | `[문서]` — 두 사실의 곱이다: ① 그 딱지가 가리키는 것이 **「app-server 명령과 WebSocket 전송」**이라 **stdio 도 면제가 아니다**(D9 · HIGH 6 · `.claude/handoff/attachments/codex-app-server-survey.md:66`·`:120`) ② **핸드셰이크에 버전 필드가 없다**(S8 · 같은 파일 `:53`: 「a breaking change cannot be detected at handshake; it surfaces at runtime」) | ★**둘을 곱하면 이렇게 된다: 상류가 이 표면을 바꿔도 우리는 접속 시점에 아무것도 못 보고, 런타임에 터진다**★ — 그리고 그 「터짐」의 화면 모양이 §4-7 이 경고한 침묵이다(아는 이름이 하나도 안 와서 화면이 빈다). ★**대비는 §4-7 의 셋이 전부이고 그것들은 사후 신호다**★(번역 표 골든 = 이름이 **바뀌면** 잡는다 · 모르는 이름 로그 = 이름이 **생기면** 잡는다 · CLI 버전 기록 = 「언제부터」를 가른다). ★★**그리고 이 위험이 ADR-0187 본문에는 빠져 있다**★★ — 그 ADR 은 `exec`·SDK 를 실측으로 기각하고 app-server 를 골랐지만 「고른 표면이 실험적이고 버전 게이트가 없다」를 대가로 적지 않았다. ★이 문서가 그 ADR 보다 넓게 아는 자리이므로 여기 적어 둔다 — 그 ADR 의 「다시 열 트리거」에 이 축이 없다는 뜻이기도 하다★ |
| ★**「쓰기 허용 + 안 묻기」 조합에서 서버 요청이 오나**★ | ★`[미확인]` — **운영값 자체가 미측정이다**★. 실측된 무승인 조합은 **읽기 전용** 쪽이고 그 조건에서 서버 요청이 0 건이었다(L11) | ★**ADR-0192 가 고른 값이 그 조합이다**★(§10-5). 면제가 매달린 다리는 **「승인 안 묻기」** 쪽이고 그 다리는 안 움직였다 — 옮긴 것은 **샌드박스 축**이라, 남는 질문은 ★「샌드박스 밖으로 나가려는 동작에서 요청이 오나(아니면 그냥 거부되나)」★ 하나다. **처분은 이미 있다** — 오면 §5-2 첫 행의 유계 오류 응답을 탄다. §9-2 의 안전 봉투는 실측된 읽기 전용 조합으로 덮으므로 ★이 질문을 **레인에서 못 잰다**★ — 사람 눈 몫이다 |
| ★**`app-server` 서브커맨드에 최상위 `-s`/`-a` 가 함께 서나**★ | `[미확인]` — 실측 기동은 `app-server --stdio` **단독**이었고 정책은 params 로 갔다(§2 L4) | ★**지금 설계는 이것을 안 묻는다**★ — 정책이 params 로 가므로 argv 에 그 플래그를 실을 이유가 없다(§8 의 `build_spec` 행). ★단 「PTY 판 argv 를 그대로 재사용하자」는 갈래가 나오면 **여기서 막힌다** — 그때 재는 것이지 선다고 가정하지 않는다★. 참고로 `-a` 가 최상위에만 있고 `codex exec` 에는 없다는 것은 실측이다(L17) — **서브커맨드마다 인자 집합이 다르다는 것이 이미 관측된 사실**이라 「app-server 도 받겠지」로 넘길 자리가 아니다 |
| ★**살아 있는 대화의 승인 정책을 바꿀 수 있나**★ | `[미확인]` — `ClientRequest` 155 종에 그런 메서드가 있는지 안 봤고 claude stream-json 쪽도 안 봤다 | ADR-0188 「후속으로 남는 것」 ③. **못 바꾸면 「모드 변경 = 다시 띄우기」가 되어 사용자가 체감하는 동작이 달라진다** |
| ★**claude `--permission-mode` 6 값이 실제로 무엇을 하나**★ | `[미확인]` — ★값별 설명이 도움말에 한 줄도 없다★`[실측]` | ★**ADR-0188 이 값 설계를 후속으로 미룬 이유가 이것이다**★ — 축을 짜려면 그 여섯이 하는 일을 알아야 한다. 재고 나서 정한다 |
| **cold resume**(재기동 후 잇기) | `[미확인]` — 문서상 지원 + 적대 리뷰의 「우리 용례는 지원되는 쪽」 판정 | §9-1 |
| **추론 텍스트를 켜는 설정** | `[미확인]` — 기본 설정에서 `reasoning` 아이템이 빈 내용으로 왔다(L8) | §10-6-b ① — **지금은 늘릴 어휘의 내용조차 없다** |
| `-32001` 이 실제로 언제 나오나 · **큐 깊이** | `[미확인]` | 열린 채 — §4-5 는 「안 나올 것이다」로 생략하지 않는 것까지만 정한다 |
| **비-JSON stdout 라인**이 이 버전·이 조합에서 나오나 | ★**답이 났다 — 나오지 않았다**★(stdout 비-JSON 0 · stderr 0 바이트, 두 실행 모두) | ★**그래도 §4-6 을 면제하지 않는다**★ — paseo 가 겪은 결말이 「데몬이 죽는다」라 §4-5 와 같은 판정이다(D6) |
| `ThreadItem` **20 변형 목록** · 알림 **81 종 전체 명단** | `[미확인]` — 관측된 알림은 11 종이다(L7) | ★**codex 를 돌릴 필요 없는 판독이다**★ — 스키마를 로컬에서 생성할 수 있다(L14 · 네트워크·모델 호출 없음). §9-1 첫 게이트의 항목 |
| ★**「응답 수령 → persist 호출」 사이 크래시의 복구**★ | `[미확인]` — 설계도 없다 | ★**2a 가 닫지 않는다**★. §6-3 의 게이트는 「persist 전에 턴이 나가는 것」을 막고 **그 창에서 죽는 것은 막지 못한다**. 저장된 sid 가 이미 있으면 결말이 **조용한 옛-스레드 재부착**이다 |
| **쓰기 쪽 정지(stall)** 가 이 조합에서 실제로 일어나나 | `[미확인]` | 열린 채. §4-9 는 그것을 전제하지 않고 **탐지 수단을 남기는 것**만 정한다 |
| ★★**그 정지가 길어져도 화면에는 아무 신호가 없다 — 그리고 죽이지 않기로 했다**★★ | ★**알려진 한계**★ — `[미확인]` 이 아니라 **고른 결과**다(§10-21 · 사용자 결정) | 프로세스는 살아 있고 EOF 는 안 오고 세션 상태도 안 바뀐다. 사용자가 보는 것은 「응답이 안 오는 에이전트」 하나이고, 그 원인이 **쓰기가 막힌 것**인지 **모델이 오래 생각하는 것**인지 가를 표면이 없다. ★**우편 쪽만 풀린다**★ — `BUSY_MAX_TURN` 30분 fail-open 상한과 그 sweep 이 그 수신자 앞 파킹을 깨운다(`crates/engram-dashboard-messaging/src/busy.rs:44-57` · `:136-146`). ★**강제 종료 감시는 만들지 않는다 — 그 동사의 소유자가 없다**★. 겪거나 재고 나면 그때 값과 함께 만든다 |
| ★★**담긴 뒤에 실패한 write 는 그 호출자에게 돌아갈 길이 없다**★★ | ★**알려진 한계**★ — 큐를 앞에 세운 대가이고 **해결하지 않는다**(§10-20 · 사용자 결정) | 오늘은 write 가 **호출 안에서** 일어나 실패가 `Err` 로 그 호출에 돌아간다(`crates/engram-dashboard-agent/src/session.rs:150-156`). 큐가 서면 그 호출은 이미 `Ok` 로 돌아간 뒤다. ★**보이는 것은 있다**★ — 통로 구현체가 출력 스트림에 `Error{message}` 를 낼 수 있다(§4-10). ★**그러나 그것은 화면·LLM 이 뒤늦게 보는 것이고 호출자도, 이미 성공으로 적힌 배달 관측 레코드(ADR-0088)도 고쳐지지 않는다**★. ★정정 채널을 발명하지 않는다★ |
| ★**writer 스레드를 기다리는 동사가 없다 — 큐에 남은 미전송 입력은 조용히 사라진다**★ | ★**알려진 한계**★ — 구조적 사실 | `OutputCore` 는 pump 핸들·done 채널을 **하나씩만** 갖고(`crates/engram-dashboard-agent/src/output_core.rs:68-69`) `join_pump` 는 그 하나를 기다린다(`:383-392`). writer 스레드는 첫째 동사가 **인과로** 끝낼 뿐이다(§5-7). ★kill 경로에서는 맞는 결말이지만(세션이 끝난다) **그 사실이 어디에도 안 보인다**★. ★**셋째 동사를 만들어 닫지 않는다**★ — ADR-0001 |
| ★**입력 큐 *길이* 상한 값**★ | ★**미정 — 정할 근거가 실측 0 이다**★ | §4-10. 규율은 확정이다(초과 = `Err`, 호출자가 그 자리에서 듣는다). 값은 구현 시 정하고 **근거를 코드 주석에 남긴다** |
| ★**입력 큐 *대기 시간* 상한을 두나**★ | ★**두지 않는 것이 이 문서의 기본 — ADR-0190 과 부딪힌다**★ | §4-10. 시간으로 자르면 **정당하게 긴 턴을 기다리던 항목**을 `Err` 로 떨어뜨려 「입력을 버리지 않는다」에 걸린다. 그리고 「긴 턴」과 「영영 안 풀리는 턴」을 가를 수단이 오늘 없다(위 stall 행). ★**연결이 끊기는 경우는 이미 `Err` 로 덮이므로, 남는 것은 살아 있는 연결의 긴 턴뿐이다**★ |
| ★**「대기 중」을 화면에 알릴 수단**★ | ★**없다 — 프론트에 그 개념이 0 이다**★ | §4-10 · §10-17. 없으면 담긴 입력이 **즉시 나간 것과 구별되지 않는다**. 범위·모양은 후속 |
| ★**입력 큐와 우편 파킹의 중첩**★ | ★**의도적으로 미해소**★ — 대기 자리가 둘이 된다 | §4-10 · ADR-0190. **지금은 둘을 따로 둔다**(사용자 결정). ★그 판단 전까지 「편지가 두 번 대기한다」가 실재할 수 있다★ |
| ★**직렬 턴 정책이 「json 모드 공통」으로 적혀 있으나, 실제로 매달린 것은 wire 형식이 아니라 백엔드다**★ | ★**알려진 한계**★ — 구조적 사실 | §4-10. 「한 번에 한 턴」이 지금 참인 이유는 **오늘 가진 json 백엔드 둘이 그렇기 때문**이다(claude json 은 1 호출 = 완결 턴 1개 · codex 는 `turn/steer` 를 이번 범위에서 뺐다). ★**JSON 이면서 동시 턴이나 steer 를 지원하는 셋째 백엔드에는 이 정책이 안 맞는다**★ — 그때 갈라야 하는 축은 「wire 가 JSON 이냐」가 아니라 **「그 백엔드가 턴을 겹쳐 받나」**다. ★지금 그 축을 만들지 않는다 — 그런 백엔드가 없고 만들 근거가 실측 0이다★. **이 절의 이름을 근거로 「JSON = 직렬」을 강요하지 말 것** |
| 두 연결이 한 스레드의 **턴을 몰 수 있나** | `[미확인]` — 조사 보고서가 미결로 남겼다 | 열린 채. 우리 모델은 대화마다 프로세스 하나라 지금은 안 걸린다 |
| **`extra_args` 로 같은 플래그를 또 넣으면 뒤 값이 이기나** | `[미확인]` — 양쪽 백엔드 다 | ADR-0188 의 값 설계가 「우리 값 → 인자」를 만들 때 여기 걸린다 |
| app-server 경로의 **신뢰 모달 유무** | `[미확인]` | ★§10-9 는 이 관측을 **기다리지 않고** 닫혔다★ — 「지금 안 연다」의 사유가 **권한 축**이라 다른 축이다. 그래도 열 때는 이것을 봐야 한다 |
| 0.153.4 ↔ 상류 `main` **드리프트** | `[미확인]` | 열린 채. §4-7 의 버전 기록이 최소 대비다 |
| 죽는 **사유별** 복구(로그인 만료 · MCP · 서브에이전트) | `[미확인]` | ★이 문서가 설계하지 않는다★(§3 끝) — **EOF 처분 하나로 흡수한다.** ★ADR-0192 뒤로 그 흡수가 더 강해졌다★ — 사유별로 갈릴 여지가 있던 자리(「어느 사유면 다시 붙나」)가 통째로 없어졌다. 구별이 필요해지면 `resume_failure_kind`(ADR-0172)가 그 자리다 |
| 「첫 턴 전 persist」가 **불변식인가** | ★**아니다 — 아직 요구사항이다**★ | ADR-0185 가 「성립한 불변식으로 인용하지 말 것」으로 도장 찍었다. §6-3 이 설계를 놓고 §9-3 이 자동 단언을 놓으면 그때 불변식이 된다 |
| capability 16 칸의 **프론트 독자** | 없음(실측) | ★§10-3 이 「2a 는 만들지 않는다」로 닫혔으므로 **열린 채로 남는다**★ — Phase 3. ★그중 `canInterrupt` 절반은 **T-34** 가 진다★ |
| 병렬 세션의 파일 경계가 아직 유효한가 | `[미확인]` — 이 세션이 재확인하지 않았다 | 착수 전 확인(§0) |
| ★**모르는 메서드에 우리가 낸 오류 응답을 app-server 가 어떻게 받나**★ | `[미확인]` — 그 봉투를 우리가 정의하므로(S1) 코드도 우리가 고른다 | 열린 채. §5-2 는 그것을 전제하지 않고 ★**「버리지 않는다」만 정한다**★ — 버림의 결말이 **영구 정지**라 §4-5 와 같은 판정이다 |
| ★**codex 가 유저 메시지를 replay 로 되울리나**★ | `[미확인]` — ★단 `userMessage` 아이템이 실재하는 것은 실측이다★(L8) | ★**§10-15(합성 에코의 shape)가 이것에 매달린다**★ — 에코 shape 는 그 백엔드 decoder 가 replay 로 만드는 것과 같아야 하고(`crates/engram-dashboard-agent/src/backend/mod.rs:204-205`) 어긋나면 화면에 두 개로 남는다. §10-2 가 「우리가 보낸 것」 표시를 요구하므로 그 표시가 양쪽에 같아야 한다 |
| ★**중단된 아이템이 끝내 안 닫힌다**★ | ★**미확인이 아니라 실측이다**★(L10) — `item/started` 가 열린 채 남는다 | ★**번역기가 「시작한 아이템은 반드시 닫힌다」를 전제하면 그 자리에서 깨진다**★ — §6-1 이 그 전제를 세우지 않는 것이 처분이고, §9-3 의 골든이 중단 케이스를 함께 재야 한다 |
| ★**「persist 됐다」를 확인할 수단이 없다**★ | ★**미확인이 아니라 구조적 사실이다**★ — `ProfileStore::save` 가 `()` 를 돌려주고 실패를 삼킨다(`crates/engram-dashboard-agent/src/profile.rs:310-311` · `crates/engram-dashboard-agent/src/persistence/mod.rs:102-103`) | ★**2a 가 닫지 않는다**★ — 닫으려면 그 trait 의 반환형과 명시된 계약 문장을 뒤집어야 한다. §6-3 이 게이트 조건의 정직한 이름을 「반영 호출이 돌아왔다」로 내렸고 §9-3 의 단언 천장도 그것이다 |
| ★**`observe_session_id` 에 화신 가드가 없다**★ | 구조적 사실 — `incarnation` 인자를 받지 않는다(`profile.rs:629`). 같은 레지스트리의 `set_last_failure` 는 받는다(`:575-583`) | ★**2a 가 닫지 않는다**★(수령 배선 = Phase 2b · §0). 죽은 화신의 늦은 관측이 산 화신의 sid 를 덮을 수 있다 — §6-3 |
| ★**프론트에 미지를 안전하게 받는 문이 없다**★ | 구조적 사실 — 누산기 switch 에 `default:` arm 이 없고(`src/components/slot/structuredAccumulator.ts:66-138`) `src/api/wsFrame.ts:35` 가 `0`·`1` 외 태그를 로그도 카운터도 없이 버린다 | ★**§10-6 이 이것을 비용으로 바꿨다**★ — 타입 하나를 더하기로 했으므로 **그 가지를 안 만들면 새 타입이 조용히 사라진다.** 전면 회수는 여전히 Phase 3 이지만 **이 한 가지는 2a 가 진다** |

### 스키마 판독만으로 주장하는 것 — 목록으로 남긴다

★**이 목록이 짧아졌다**★ — 알림 이름 11 종 · `item/started`/`item/completed` · `agentMessage` 델타 · `ThreadStartResponse` 필드 · `turn/interrupt` 파라미터 · `TurnStatus` 중 `completed`·`interrupted`·`inProgress` 는 **실측으로 올라갔다**(§2 L4~L11).

**아직 `[스키마]` 이고 실행 근거가 0 인 것:** 선언된 알림 **81종 중 관측되지 않은 70종** · `ClientRequest` **155종 중 우리가 안 부른 151종** · `ServerRequest` **11종 전부** · `TurnStatus` 의 `failed` · `ThreadActiveFlag` 두 값 · `turn/steer` 의 파라미터 · `ThreadItem` **20 변형 목록** · `ThreadStartParams` 27 속성에 id 계열이 0개라는 것. ★**§6-1 번역 표에서 이 등급이 남은 행은 「표가 틀리면 번역기가 아니라 표부터 다시 짠다」가 그대로 유효하다**★.

★**그리고 「선언 ≠ 관측」을 이 자리에서 큰 소리로 남긴다**★ — 스키마는 **클라 요청 155종 · 서버 알림 81종 · 서버 요청 11종**을 선언하고, 실제로 온 것은 **알림 11종**이며 **서버 요청은 0 건**이다(`approvalPolicy:"never"` + read-only 조건). ★**스키마에 있다는 이유로 「온다」고 쓰지 말 것**★ — 이 문서의 등급 체계가 존재하는 이유가 정확히 그것이다.

★**그리고 초판이 이 목록에 잘못 넣었던 것 넷을 여기서 갈라 둔다 — `[스키마]` 가 아니라 `[문서]` 다**★(§2 의 해당 행이 정본):

- `turn/start`·`turn/steer`·`turn/interrupt` 가 **stable** 이라는 것 — 상류의 stable 표식·주석 판독(S6 · F10). ★단 그중 `turn/start`·`turn/interrupt` 는 **실제로 돌았다**★(L5 · L9) — stable 표식의 등급과 동작의 등급은 다른 축이다.
- `Thread.id` 가 **UUIDv7** 이라는 것 — 1·2 판에서는 문서 문장이었다(S2b). ★**지금은 `[실측]` 이다**★ — 받은 값의 버전 니블·variant·ms 접두를 디코드해 확인했다(L4). `profile.rs:165` 의 `Option<Uuid>` 타입이 이 한 줄에 매달려 있어서 등급이 중요했고, **그 다리가 문서에서 실측으로 바뀌었다.**
- `ThreadListParams.cwd` 가 **정확 일치** 필터라는 것 — 상류 소스 판독(S9).
- 승인 요청 **다섯의 이름**과 `availableDecisions` 의 존재 — ★**출처 표기를 고친다: 이 등급은 「이 문서의 판정」이고 보고서의 분류가 아니다**★. 1·2 판이 「보고서 F9 가 `[문서]` 로 분류한다」로 적었는데 ★**그 보고서에는 이 문서의 등급 체계가 없다**★ — 확실 / 가능성 높음 두 축을 쓰고 **F9 를 `확실` 로 태그한다**. 두 체계는 **다른 것을 재고 있다**: 보고서의 `확실` 은 「판독한 근거가 흔들리지 않는다」이고 이 문서의 `[문서]` 는 「우리가 돌려 보지 않았다」다. **결론은 안 바뀐다** — D1 행의 등급은 그대로 `[문서]` 다. ★같은 부류의 잘못된 귀속을 다시 만들지 말 것 — 보고서에서 등급을 「인용」할 수 없다.★
  - ★**단 이름 목록은 이제 스키마 생성물로 확인됐다**★ — 11 종의 전체 이름이 §2 L14 의 경로로 나온다. **여전히 왕복은 0 이다.**

그리고 `ThreadListParams.cwd` 의 **Windows 저장형이 UNC 확장**이라는 것은 반대 방향으로 갈린다 — 그것은 `[실측]` 이다(이 PC 의 로컬 sqlite 판독 — §2 L18. **왕복 없이 생긴 실측**이라 축이 다르다).

### ADR-0186 이 닫지 않은 둘 — 이 문서의 처분

- **앵커 부재** — `rg "ADR-0185|ADR-0186" crates/ src/ src-tauri/` = 0줄. ★**이 문서가 §8 로 닫는다**★(codex 백엔드 주석 둘 + `// ADR-0185` 앵커).
- **설계 문서 표면** — 「회귀 테스트는 *구현* 을 잡고 *제안* 은 못 잡는다. 다음 세션이 TRD 에 「resume 실패하면 새로 파자」를 쓰면 빨개지는 것이 없다.」 ★**이 문서도 그것을 닫지 않는다**★ — 할 수 있는 것은 §3 의 EOF 절에 그 금지를 적고 회귀 테스트 이름을 함께 박아 두는 것까지다. ★**그리고 이 판이 그 한계의 실물 사례를 하나 얻었다**★ — 3판의 「EOF 뒤 재부착」과 「계약에 읽기 함수 하나」는 **어느 테스트도 빨갛게 하지 않은 채** 두 판을 살아남았고, 잡은 것은 적대 리뷰였다.

**이 문서가 근거로 삼은 것 — ★가리키기만 하고 베끼지 않는다★:**

- ★**실측 정본**★ = `.claude/handoff/attachments/codex-measurements-2026-09-09.md` — codex-cli 0.153.4 · Windows 11 의 실행 스냅샷. §2 의 `[실측]` 행 전부가 여기서 왔다. ★**버전이 바뀌면 그 문서가 낡고, 그러면 이 문서의 `[실측]` 도 함께 낡는다**★.
- ★**소유권 지도**★ = `docs/reference/structure/session-path-ownership.md` — 세션 경로의 객체별 소유·수명 추적(2026-09-09 실측 스냅샷). ADR-0189 가 「막힌 자리」를 확정한 근거이고, §3·§5 의 배치가 이 위에 선다. ★**쓸모 있는 부분은 PART B**(설계가 실제로 답해야 하는 질문 열)**와 PART C**(「문이 없다」 목록)다★ — 통째로 읽지 말 것.
- **판독 보고서** = `.claude/handoff/attachments/codex-app-server-survey.md`(조사 보고서 + 그 적대 리뷰 — BLOCKER 2 · HIGH 6). ★본문은 적대 리뷰를 반영하지 않은 상태이고, §2 「뒤집힌 자리」가 그 정정이며 **보고서 본문보다 이쪽이 우선한다**★.
- **결정** = ADR-0187(통로 확정) · ADR-0188(권한 중립 축) · ADR-0189(통로의 자리) · ADR-0190(입력 큐) · ADR-0191(통로 생성 seam) · ADR-0192(claude 기준 기본값) · ADR-0193(큐 해제 판정의 주인) · ADR-0088(배달 관측) · ADR-0185 · ADR-0186. ★**그리고 사용자 결정 셋**★ — §10-20(영수증 하나) · §10-21(강제 종료 감시 없음) · §10-C(7 ① 을 구현 진입 앞으로).
- **코드** = 이 저장소의 소스·주석·생성 바인딩 **정적 판독**(줄 번호는 2026-09-09). `trd.md`(Phase 0·1).

**이 문서가 실행하지 않은 것:** 빌드 · 테스트 · 앱 기동 · codex 호출 — 이 작업은 문서 하나만 쓴다. ★§2 의 `[실측]` 은 **이 문서가 아니라 위 실측 문서가 만든 것**이다★.
