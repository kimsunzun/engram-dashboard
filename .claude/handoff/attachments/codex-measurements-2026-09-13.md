# codex 실측 기록 — 2026-09-13 (codex-cli 0.154.0 · Windows 11)

> **이 문서는 실행 결과 스냅샷이다.** 재는 범위가 좁다 — `codex app-server` 가 로컬에서 내보내는 JSON 스키마의 `ThreadItem` 변형 **수와 목록** 하나, 그리고 그 옆에 함께 나온 최상위 선언 수 넷이다. 옆 문서 `codex-app-server-survey.md` 는 **스키마·문서 판독**이고 이 문서는 **실제로 돌려서 본 것**이다. 둘이 어긋나면 이 문서가 이긴다 — 단 ★버전이 바뀌면 이 문서가 낡는다★(측정 대상 = `codex-cli 0.154.0`, 측정일 = 2026-09-13).
>
> ★**형제 문서 `codex-measurements-2026-09-09.md` 와 버전이 다르다 — 두 문서의 수치를 한 스냅샷으로 섞어 읽지 말 것**★. 그쪽은 `codex-cli 0.153.4` 다. 플래그를 맞춘 like-for-like 비교에서 `ClientRequest` 선언 수가 **155 → 159** 로 움직였다(§3) — 두 버전이 교환 가능하지 않다는 직접 증거다.
>
> 원시 캡처(생성된 스키마 트리)는 세션 스크래치패드에 있었고 **repo 에 들이지 않았다** — 아래 명령이 재현 경로다.

## 0. 측정 환경

- **버전** — `codex --version` → `codex-cli 0.154.0`.
- **플랫폼** — Windows 11 Enterprise 10.0.26200 (x86_64).
- **측정일** — 2026-09-13.
- ★**네트워크·모델 호출 0 · app-server 왕복 0**★ — 스키마 생성은 순수 로컬 판독이다. 그래서 이 문서의 수치는 **선언**이고 **관측이 아니다**.

## 1. 명령

```
mkdir -p <dir>/codex-schema
codex app-server generate-json-schema --out <dir>/codex-schema
```

- 종료코드 **0**. 출력 디렉터리 최상위 = **`.json` 37 개 + 하위 디렉터리 둘(`v1/`·`v2/`) = 39 항목**.
- ★**`--experimental` 유무가 `ThreadItem` 을 바꾸지 않는다**★ — 같은 명령을 `--experimental` 을 붙여 따로 생성해도 변형 수는 **19** 로 같고 이름 목록·순서도 같다. 단 최상위 선언 수 둘은 바뀐다(§3). (`--help` 가 이 서브커맨드 전체를 `[experimental]` 로 적는다.)

## 2. `ThreadItem` — 어디에 정의되고 몇 개인가

- ★**자기 파일이 없다**★ — `ThreadItem.json` 은 생성되지 않는다. 정의 자리는 집계 파일 **`codex_app_server_protocol.v2.schemas.json`** 의 **`definitions.ThreadItem`** 이다. 그 파일은 draft-07 이라 키가 `$defs` 가 아니라 `definitions` 이고(총 정의 628 개), 제목은 `CodexAppServerProtocolV2` 다.
- 형태 = **`oneOf` 배열이고 길이 19**. 변형마다 `properties.type` 이 **한 값짜리 `enum`** 이다 — 예 `{"enum":["userMessage"],"title":"UserMessageThreadItemType","type":"string"}`. ★`const` 가 아니다★(태그를 `properties.type.const` 로 읽는 판독기는 이 번들에서 빈손으로 돌아온다).
- **번들 안 21 개 파일이 같은 정의를 인라인으로 싣고 전부 19 로 일치한다** — 이 union 을 나르는 메시지 파일들(`ServerNotification.json` · `v2/ItemStartedNotification.json` · `v2/ItemCompletedNotification.json` · `v2/Thread*Response.json` 계열 등)과, v1 집계 파일 `codex_app_server_protocol.schemas.json` 의 **`definitions.v2.ThreadItem`**. 즉 19 는 파일 하나를 집어 센 값이 아니라 번들 전체에서 일관된 값이다.

### 변형 19 개 — 스키마 선언 순서 그대로

`userMessage` · `hookPrompt` · `agentMessage` · `functionCallOutput` · `plan` · `reasoning` · `commandExecution` · `fileChange` · `mcpToolCall` · `dynamicToolCall` · `collabAgentToolCall` · `subAgentActivity` · `webSearch` · `imageView` · `sleep` · `imageGeneration` · `enteredReviewMode` · `exitedReviewMode` · `contextCompaction`

★**선언 ≠ 관측**★ — `item/started`·`item/completed` 로 실제 도착한 것을 본 아이템 타입은 09-09 스냅샷의 셋(`userMessage` · `agentMessage` · `reasoning`)뿐이다.

## 3. 함께 나온 최상위 선언 수 — ★09-09 와 견줄 때 플래그를 맞출 것★

| 선언 | `--out` 만 | `--experimental` 함께 | 09-09(0.153.4 · `--experimental`) |
|---|---|---|---|
| `ClientRequest` | 99 | **159** | 155 |
| `ServerNotification` | 81 | **81** | 81 |
| `ServerRequest` | 10 | **11** | 11 |
| `ClientNotification` | 1 | **1** | 1 |

- ★**`--experimental` 이 `ClientRequest` 를 99 → 159 로 바꾼다**★. 09-09 측정은 그 플래그를 붙인 것이므로, 플래그를 맞추지 않은 비교는 버전 드리프트로 오독된다.
- **플래그를 맞추면 움직인 것은 `ClientRequest` 하나(155 → 159)이고 나머지 셋은 그대로다.** 이것이 위 머리글의 「교환 가능하지 않다」의 실물 근거다.
- 조사 보고서가 적은 「알림 수는 실험 opt-in 유무에 동일하다」는 이 버전에서도 성립한다(81 = 81). ★단 그 개수는 보고서의 83 이 아니라 81 이다★ — 09-09 스냅샷과 같은 값이다.

## 4. 이 문서가 재지 못한 것

- **변형별 페이로드** — 이름과 개수만 셌다. 각 변형이 무슨 필드를 싣는지, 우리 중립 어휘의 어느 칸에 드는지는 판독하지 않았다. ★그래서 번역 표를 채우는 일은 이 문서로 끝나지 않는다★.
- **관측** — app-server 를 띄워 19 종이 실제로 도착하는지 보지 않았다.
- **0.153.4 의 `ThreadItem` 수** — 그 버전이 이 PC 에 없어 되돌려 재지 못했다. 조사 보고서가 적은 20 도 어느 릴리스의 값인지 확정되지 않는다 — 그 보고서 자신이 「로컬 설치는 0.153.4 이고 판독한 상류 소스는 `main`」이라 적는다(`codex-app-server-survey.md:96`).
