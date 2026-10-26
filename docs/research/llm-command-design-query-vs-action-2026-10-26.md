# LLM 제어 명령 설계 — 조회 vs 명령 (T-65 적용) — 2026-10-26

- **상태:** 적대 리뷰 반영(판정 FIX → 수정) · 결정 없음 — T-65 PRD 의 입력.
- **방법:** `/research` medium · 서브클래스 engram · 설계-결정 모드. 수집 4갈래(쓰기 명령 피어 · 조회 결과 인라인/밖으로 · 출력 형식 · 내부 제약) → 메인 grounding(핵심 인용 원문 확인) → codex 적대 리뷰(레벨 2~3 · 웹 검색) → 리뷰 반박 3건 메인 원문 재확인 후 반영.
- **확신도 범례:** 확실(독립 교차확증) · 가능성 높음(근거 지지 · 단일 출처) · 불확실 · contested.
- **질문(사용자 틀 2026-10-26):** 조회와 명령을 가른다. 명령 = 가능한 한 한 번 호출로 끝낸다. 조회 = ① 핀포인트 vs 전체 ② 결과를 컨텍스트에 싣나 vs 밖(파일)으로 내나 — 「보통은 외부에서 찾게 한다」가 업계 관행인가.

## 결론 (먼저)

1. **밖으로 내보내기는 두 모양으로 실제로 쓰인다 — 어느 쪽이 업계 「기본값」인지는 아무도 재지 않았다.**
   - **크기 문턱형:** 결과가 커지면 파일로 넘기고 경로 + 앞부분만 준다 — LangChain Deep Agents(20,000 토큰 넘으면 경로 + 앞 10 줄 · 도구 종류 무관) · Cursor(긴 셸·MCP 출력을 파일로) · Claude Code(기본 25,000 토큰 상한).
   - **전체 상태는 늘 파일형:** Playwright CLI 는 **명령마다 페이지 정보와 스냅숏 파일 링크를 돌려준다**(자동 · 문턱 언급 없음). 에이전트가 UI 를 조작하는 CLI 라 우리와 가장 닮은 모양이다.
   - 그래서 사용자 직관(「전체는 밖으로 내고 찾게, 하나하나는 핀포인트」)은 **전체 상태 조회에 한해 실제 선례가 있다.** 확신도 = 가능성 높음(각 선례는 원문 확인 · 「보편 관행」 여부는 불확실).
2. **명령(쓰기)은 여러 단계를 한 호출로 묶는 것이 공급자 권고이고, 이름 지목은 그 권고와 맞는 선택지다 — 단 「이름 지목이 일반 규칙」까지는 근거가 안 간다.** Anthropic 은 자연어 이름이 암호 같은 id 보다 낫다고 쓰지만 주로 **돌려주는 문맥** 쪽 이야기이고, 이름으로 찾은 뒤 id 로 행동하는 흐름도 허용한다(리뷰어 지적 · 메인 미확인). 확신도 = 가능성 높음(묶기) · 불확실(이름 지목의 일반성).
3. **핀포인트·필터 조회는 권고다**(「넓은 검색 한 번보다 작고 겨냥한 검색 여러 번」 · 페이지 · 범위 · 필터 · 잘라내기 + 합리적 기본값). 확신도 = 가능성 높음.
4. **형식은 보편 승자가 없다 — 모델·과제마다 다르다.** MCP 가 구조 결과와 텍스트 블록을 나란히 두지만 그 텍스트는 **하위 호환용 JSON 직렬화**라 「구조 계약 + LLM 용 렌더」를 받치는 근거는 아니다(전송 호환일 뿐). LLM 용 렌더가 낫다는 것은 우리 데이터로 재야 안다. 확신도 = 가능성 높음(승자 없음).
5. **우리 레이아웃 조회 결과의 크기를 재지 않았다** — 「작으니 파일로 내면 호출만 는다」는 미측정 추정이다. 결정 전에 실제 크기를 재는 것이 먼저다.

## A. 명령(쓰기) — 피어와 권고

| # | 클레임 | 출처 | grounding | 확신도 |
|---|---|---|---|---|
| A-1 | 에이전트는 암호 같은 식별자보다 자연어 이름을 훨씬 잘 다룬다 — "Agents also tend to grapple with natural language names, terms, or identifiers significantly more successfully than they do with cryptic identifiers." **이 문장의 자리는 주로 돌려주는 문맥(응답에 무엇을 싣나) 쪽이다**(리뷰어). | [Anthropic — Writing tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents) | 문장 지지(메인 확인) · 쓰기 지목으로의 일반화는 리뷰어가 과장으로 지적 | 가능성 높음(문장) |
| A-2 | 여러 단계를 한 도구로 묶으라 — `list_users`·`list_events`·`create_event` 대신 `schedule_event`. Amplitude 는 엔드포인트별 도구를 객체별로 묶었다(코호트 9→1 · 차트 16→4 — **노출 도구 수**이지 실행 단계 수가 아니다). | Anthropic 같은 글 · [Amplitude](https://amplitude.com/blog/lessons-from-consolidating-our-mcp-server) | Anthropic 지지 · Amplitude 수집자 요약 | 가능성 높음 |
| A-3 | 이름 지목의 성숙 선례 = tmux: 대상 창은 특수 토큰 → 인덱스 → id → 정확한 이름 → 이름 앞부분 → glob 순으로 풀고, 여럿이 맞으면 **오류**(추측하지 않는다). `=` 접두는 정확 일치만. | [tmux(1)](https://man.openbsd.org/tmux.1) | 지지(메인 확인) | 가능성 높음 |
| A-4 | 가장 가까운 피어 herdr: pane id 는 자리 기반(`w1:p2`)이라 옮기면 바뀌고, `pane move` 뒤 새 id 를 `.result.move_result.pane.pane_id` 에서 읽는다(옛 id 는 `previous_pane_id`). 에이전트 이름은 살아 있는 동안의 별칭(`[a-z][a-z0-9_-]{0,31}` · 살아 있는 것끼리 유일)이다. | [herdr agent automation](https://herdr.dev/docs/agent-automation/) | 지지(메인 확인) | 가능성 높음 |
| A-5 | 응답은 다음 단계에 필요한 최소 — 만드는 명령은 새 id 를 돌려준다(herdr `pane split` → `.result.pane` · zellij `new-pane`·`new-tab`). i3 IPC 의 명령 응답은 `{success}` 뿐. 「간결/상세」 enum 예: 72 토큰 vs 206 토큰. | Anthropic · herdr · [zellij CLI](https://zellij.dev/documentation/cli-actions.html) · [i3 IPC](https://i3wm.org/docs/ipc.html) | Anthropic·herdr 지지 · zellij·i3 수집자 요약 | 가능성 높음 |
| A-6 | 오류는 고칠 방법을 말해야 한다 — "prompt-engineer your error responses to clearly communicate specific and actionable improvements, rather than opaque error codes". | Anthropic | 지지 | 가능성 높음 |
| A-7 | 모호한 이름에 후보 목록을 돌려주는 선례 = CeylonCharts MCP(부분·퍼지 이름 풀기 + 모호하면 JSON 후보 목록). 작은 피어이고 널리 쓰인다는 근거는 아니다. | [CeylonCharts MCP docs](https://github.com/CreonSolutions/ceyloncharts-mcp-docs/blob/master/docs/mcp-tools.md) | 리뷰어 제시 · 메인 미확인 | 불확실 |
| A-8 | herdr `pane.move` 가 갱신된 레이아웃까지 돌려준다는 서술 | herdr socket API(수집자 요약) | agent-automation 페이지엔 없음 · 전체 트리인지 일부인지 미확인 | 불확실 |

- **Playwright MCP 는 반대 쪽이다** — 대상을 스냅숏 ref 로 지목하므로 행동 전에 읽기가 하나 앞선다(수집자 요약 · 미확인).

## B. 조회 — 인라인 vs 밖으로, 핀포인트 vs 전체

| # | 클레임 | 출처 | grounding | 확신도 |
|---|---|---|---|---|
| B-1 | Claude Code 는 도구 응답을 기본 25,000 토큰으로 제한한다. 권고 = 페이지 · 범위 선택 · 필터 · 잘라내기 조합과 합리적 기본값. | Anthropic | 지지 | 가능성 높음 |
| B-2 | Cursor 는 긴 셸·MCP 출력을 자르는 대신 **파일로 쓰고** 에이전트가 `tail` 로 끝을 본 뒤 필요하면 더 읽게 한다(자르면 정보가 사라지므로). 크기 문턱은 밝히지 않았다. | [Cursor — dynamic context discovery](https://cursor.com/blog/dynamic-context-discovery) | 지지(메인 확인) | 가능성 높음 |
| B-3 | 넓은 검색 한 번보다 작고 겨냥한 검색 여러 번을 권한다 — "make many small and targeted searches instead of a single, broad search". | Anthropic | 지지 | 가능성 높음 |
| B-4 | LangChain Deep Agents: 도구 결과가 20,000 토큰을 넘으면 저장소로 넘기고 **파일 경로 + 앞 10 줄 미리보기**로 바꾼다 — 도구 종류(웹·DB·API)를 가리지 않는다. | [Deep Agents — context engineering](https://docs.langchain.com/oss/python/deepagents/context-engineering) | 지지(메인 확인 — 리뷰어 반박 반영) | 가능성 높음 |
| B-5 | **Playwright CLI: 모든 명령이 페이지 정보와 스냅숏 파일 링크를 돌려준다**(`.playwright-cli/` 아래 시각 붙은 `.yml`) — 자동이고 크기 문턱 언급이 없다. 전체 UI 상태를 늘 파일로 내는 선례. | [Playwright CLI — snapshots](https://playwright.dev/agent-cli/snapshots) | 지지(메인 확인 — 리뷰어가 찾은 누락) | 가능성 높음 |
| B-6 | 그 밖의 내보내기: Manus(「파일 시스템을 궁극의 컨텍스트로」 · URL·경로만 남기는 복원 가능 압축) · Playwright MCP(`browser_snapshot` 의 `filename` 인자 — 선택). | [Manus](https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus) · [Playwright MCP](https://github.com/microsoft/playwright-mcp) | 수집자 요약 | 가능성 높음 |
| B-7 | 내보내기가 늘리는 호출 수와 아끼는 컨텍스트를 수치로 맞바꾼 출처는 **못 찾았다**. Cursor 의 −46.9% 는 MCP 도구 설명의 동적 발견 몫이지 도구 결과가 아니다. | Cursor | 지지(그 수치의 대상 확인) | — |
| B-8 | 결과를 컨텍스트에 닿기 전에 걸러 내는 예 — 코드 실행 환경에서 10,000 행 표를 걸러 「에이전트는 10,000 행 대신 5 행을 본다」. ★같은 글의 150,000 → 2,000 토큰은 **필요한 도구 정의만 읽어 들인 몫**이지 결과 걸러내기가 아니다★(초안의 오귀속 — 리뷰어 반박 · 메인 원문 확인). | [Anthropic — code execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp) | 지지(메인 확인) | 가능성 높음 |
| B-9 | 큰 컨텍스트는 정확도를 깎는다(context rot — 18 개 모델, 집중 프롬프트 ~300 토큰 vs 전체 ~113k). 장문 검색 연구이고 도구 결과에 옮긴 것은 추론이다. | [Chroma — context rot](https://trychroma.com/research/context-rot) | 수집자 요약 | 가능성 높음(발견) · 적용은 추론 |

## C. 출력 형식 — 텍스트 vs JSON

| # | 클레임 | 출처 | grounding | 확신도 |
|---|---|---|---|---|
| C-1 | 보편 승자 없음 — Anthropic: 응답 구조(XML·JSON·Markdown)가 평가 성능에 영향을 주고 최적은 과제·에이전트마다 다르니 평가로 고르라. 프롬프트 형식 논문: GPT-3.5 는 형식에 따라 크게 흔들리고 GPT-4 는 덜하며 최적 형식이 모델 계열 사이에 잘 안 옮겨진다. | Anthropic · [arXiv 2411.10541](https://arxiv.org/abs/2411.10541) | Anthropic 지지 · 논문 수집자 요약 | 가능성 높음 |
| C-2 | 토큰: 평평한 표 데이터는 압축 형식이 JSON 보다 훨씬 짧다(TOON 자체 벤치: JSON 6,326 vs TOON 2,537 vs CSV 2,336). 중첩 데이터에서는 **데이터마다 갈린다** — deep config 는 JSON-compact 562 < TOON 669 이지만 nested-contact 는 TOON 1,444 < JSON-compact 2,357(리뷰어). 정확도는 강한 모델에서 몇 점 차, 작은 모델에서 큼. | [TOON 벤치](https://toonformat.dev/guide/benchmarks)(형식 홍보측) · [Improving Agents — TOON](https://www.improvingagents.com/blog/toon-benchmarks)(독립) | 수집자·리뷰어 요약 · 메인 미확인 | 불확실 |
| C-3 | 중첩 데이터 형식 비교 연구가 있다 — 6~7 단 중첩 설정에서 JSON·YAML·XML·계층 Markdown 비교(작은 모델 GPT-5-nano: YAML 62.1 · Markdown 54.3 · JSON 50.3 · TOON 43.1). 창 레이아웃 데이터는 아니다. 복잡한 JSON 도구 응답 처리를 15 개 모델로 잰 EACL 2026 논문도 있다(리뷰어 제시 · 미확인). | [Improving Agents — nested format](https://www.improvingagents.com/blog/best-nested-data-format) · [EACL 2026](https://aclanthology.org/2026.eacl-long.134/) | 리뷰어 제시 · 메인 미확인 | 불확실 |
| C-4 | MCP 는 구조 결과(`structuredContent`)와 텍스트 블록을 나란히 둔다 — "For backwards compatibility, a tool that returns structured content SHOULD also return the serialized JSON in a TextContent block." **하위 호환용 JSON 직렬화**이지 LLM 용 렌더가 아니다. | [MCP 2025-06-18 tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools) | 지지(메인 확인) | 가능성 높음 |
| C-5 | CLI 선례: `gh --json/--jq` · `kubectl -o` · `docker --format` — 사람용 기본 화면 + 기계용 모드. | 지식 기반(이번에 출처 안 봄) | 미확인 | 불확실 |

## D. 내부 제약 (코드 실측)

- **지금 레이아웃 명령은 전부 불투명 id 로 지목한다** — 탭 `view_id` · 칸 `slot_id` · 분할 `split_id` · 창 `window_id`/같은 실행 label(번호는 거절 · ADR-0302). 에이전트 이름을 받는 레이아웃 명령은 없다. `slot.assignAgent` 에 이름을 넘기면 거절되지 않고 칸이 빈 채로 보인다(레이아웃 층은 에이전트 존재를 모른다 · ADR-0035). (`src-tauri/src/layout/commands.rs`)
- **조회는 작고 범위가 좁다** — `tab.list{window}` · `window.list` · `split.list{view_id}` · `slot.resolveSpatial{token}`. **칸 내용을 되읽는 명령과 전체 배치 조회는 없다**(실측 — 명령 이름 목록에 없음).
- **쓰기 응답은 작다** — 만드는 명령은 id(`tab.create` → `{view_id}` · `slot.split` → `{slot_id}`), 창 상태 명령은 작은 상태 칸(`window.maximize` → `deferred` · `window.raise` → `shown_all`·`focused`·`pending` — 리뷰어 정정), 나머지는 `{}`.
- **CLI 는 응답 몸통을 그대로 찍는다**(`crates/engram-dashboard-cli/src/bin/engram.rs:264`) — 잘라내기 · 페이지 · 텍스트 렌더 · 파일 내보내기 없음. 텍스트로 그리는 것은 `help` 와 명령 목록 화면뿐.
- **데몬은 UI 명령 몸통을 해석하지 않는다**(ADR-0081 · ADR-0156) — 응답 모양을 바꾸려면 셸 처리기나 CLI 에서 해야 한다.
- **이름 풀기 선례가 이미 있다** — agent crate `resolve_in`(`crates/engram-dashboard-agent/src/commands.rs:1686`): id 먼저, 그다음 정확한 이름, 중복 이름은 CONFLICT, 없으면 NOT_FOUND(tmux 와 같은 「모호하면 오류」). **단 셸은 agent crate 를 의존하지 않는다**(ADR-0270) — 셸에는 데몬 목록 사건으로 채우는 id→이름 캐시(`layout/manager.rs` 의 `agent_names`)가 있고 지워지지 않아 낡은 이름이 남을 수 있다.
- **도킹 적용 서비스는 이미 한 번에 받는다** — `apply::dock_tab(DockRequest{view, to: DockDest{Seam{target, anchor, side, share} | TabBar{window, index} | NewWindow{..}}, focus, mv})`(`src-tauri/src/layout/apply.rs`). `NewWindow` 는 main 스레드 · 동기 처리기에서 부르면 교착한다. TRD 슬롯 도킹 §3-6 이 버스 이름을 `tab.dock`(포커스 유지)으로 예정해 두었다.
- **사용자 기준(옛 메모 · T-65)** = 흔한 요청은 1~2 호출 · 「LLM 시나리오는 여러 번 거치지 않아야 실수가 적다」. 옛 메모 후보 = 읽기 `layout.get`(전부) · 쓰기 `tab.dock`/`tab.move`/`tab.detach`(에이전트 이름 지목 · 모호하면 후보 목록과 함께 거절). 「명령 하나 + 모드」 안(TRD v4)은 조합 실수 여지로 밀려났다.
- **공간 지목은 논리 핸들**(방향 · 이웃 · 순번 · tmux 식 토큰)이고 픽셀 노출은 미뤘다(ADR-0068). 값 읽기는 v1 필수(ADR-0156 — 사용자 결정).

## 후보 × 제약 적합도

호출 수는 같은 과제(「에이전트 X 를 Y 옆으로」 같은 흔한 요청) 기준이고 **크기를 재지 않아 컨텍스트 칸은 상대 비교뿐**이다(리뷰어 지적 — 같은 과제 · 같은 크기로 견줘야 순위를 낼 수 있다).

조회 쪽:

| 후보 | 호출 수 | 컨텍스트 | 우리 제약 적합 | 선례 |
|---|---|---|---|---|
| Q1 전체를 인라인(JSON 그대로) | 1 | 레이아웃 크기에 비례 | 쉬움(지금 CLI 그대로) | — |
| Q2 전체를 늘 파일로 · 경로(+ 요약)만 인라인 | 1 + 필요한 만큼 읽기 | 최소 | CLI·셸에 파일 쓰기 경로 신설 | Playwright CLI(B-5) |
| Q3 핀포인트 조회(「에이전트 X 어디」 · 필터 인자) | 1 | 최소 | 셸 처리기에서 가능 | Anthropic 권고(B-3) |
| Q4 전체 인라인 + 상한, 넘치면 파일 + 미리보기 | 1(넘치면 1 + 읽기) | 상한 안 | 셸 처리기에서 렌더 · 넘침만 파일 | Deep Agents · Claude Code(B-1 · B-4) |

명령 쪽:

| 후보 | 호출 수 | 실수 여지 | 우리 제약 적합 |
|---|---|---|---|
| W1 이름 지목 + 한 번에(조회 없이) · 응답은 바뀐 것(새 id 포함) · 모호하면 오류(+ 후보) | 1 | 이름 모호성 → 오류로 막힘 | 셸에 이름 풀기 필요(캐시 낡음 처리) |
| W2 id 지목(지금 방식) | 조회 1 + 명령 1 | id 복사 실수 | 그대로 |
| W3 명령 하나 + 모드(`tab.dock` 하나에 목적지 종류) | 1 | 조합 실수(옛 판정) | `DockDest` 와 1:1 |
| W4 목적지별 명령 여럿(`tab.dock`/`tab.move`/`tab.detach`) | 1 | 명령 고르기 실수 | 같은 적용 서비스 위 얇은 셋 |

형식 쪽: F1 JSON 그대로 · F2 구조 계약 + LLM 용 텍스트 렌더(셸 처리기 또는 CLI · `format` 인자) · F3 TOON 류 압축 형식 — 셋 다 근거만으로는 우열이 안 갈린다(C-1~C-3). 우리 데이터로 작은 평가가 필요하다.

## 거부 후보(→ ADR 거부 대안 재료)

- **CDP DOM 으로 읽기** — 옛 메모가 이미 거부(인증 없음 · 모델이 아닌 렌더 · 깨지기 쉬움 · 큼).
- (초안의 「Q2 를 기본으로 거부」 · 「F3 일괄 거부」는 리뷰 뒤 걷었다 — 근거가 그 거부를 받치지 못한다.)

## 공백 · 한계

- **레이아웃 조회 결과의 실제 토큰 크기를 재지 않았다** — Q 후보 순위의 전제다.
- 트리 데이터 형식 평가를 우리 레이아웃으로 돌리지 않았다(C-3 은 일반 설정 데이터).
- 「업계 기본값이 인라인이냐 파일이냐」의 보급률은 아무 출처도 재지 않았다(결론 1).
- 메인이 원문을 못 본 리뷰어 제시 출처: CeylonCharts(A-7) · TOON nested-contact 수치와 독립 시험(C-2) · Improving Agents 중첩 연구와 EACL 2026(C-3) · Anthropic 의 「이름으로 찾은 뒤 id 로 행동」 허용(결론 2).
- 못 본 피어: Hyprland · sway · yabai · komorebi · GitHub MCP · OpenAI 함수 호출 권고(수집 예산).
- 이름 지목이 호출 수를 얼마나 줄이는지의 수치 출처 없음.

## 적대 리뷰 결과 (codex · 레벨 2~3 · 판정 FIX)

| # | 대상 | 결함 | 심각도 | 처리 |
|---|---|---|---|---|
| 1 | A-1/A-2 → 결론 2 | 과장 — 이름 문장은 응답 문맥 쪽, Amplitude 수치는 도구 수 | 중 | 결론 2 · A-1 · A-2 완화 |
| 2 | 결론 1 | 일반화 근거 없음 + 반례 누락(Playwright CLI 자동 파일 링크) | 상 | **메인 원문 확인 → 결론 1 재작성** · B-5 신설 |
| 3 | B-5(초안) · Q2 거부 | 「끝없는 결과만」 제한이 틀림 — Deep Agents 는 도구 종류 무관 | 중 | **메인 원문 확인** → B-4 · Q2 거부 걷음 |
| 4 | B-7(초안) 150k→2k | 오귀속 — 도구 정의 몫 | 상 | **메인 원문 확인** → B-8 정정 |
| 5 | C-3 → 결론 4 | MCP 텍스트 블록은 하위 호환일 뿐 | 중 | 결론 4 · C-4 정정 |
| 6 | C-5(초안) 「연구 없음」 | 과한 부재 주장 | 중 | C-3 로 바꿈(미확인 표시) |
| 7 | F3 일괄 거부 | 과일반화 — 중첩에서도 데이터마다 갈림 | 중 | 거부 걷음 · C-2 보강 |
| 8 | 후보 목록 피어 없음 | 반례 누락(CeylonCharts) | 중 | A-7 신설(미확인) |
| 9 | 결론 5 · Q 표 | 미측정 추론 · Q2/Q4 호출 셈 불일치 | 상 | 결론 5 를 「먼저 잴 것」으로 · Q 표 호출 셈 맞춤 |
| 10 | D 「나머지는 `{}`」 | 코드 사실 틀림 | 하 | D 정정 |
