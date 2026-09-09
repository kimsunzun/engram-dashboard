# codex 실측 기록 — 2026-09-09 (codex-cli 0.153.4 · Windows 11)

> **이 문서는 실행 결과 스냅샷이다.** 옆 문서 `codex-app-server-survey.md` 는 **스키마·문서 판독**이고 이 문서는 **실제로 돌려서 본 것**이다. 둘이 어긋나면 이 문서가 이긴다 — 단 ★버전이 바뀌면 이 문서가 낡는다★(측정 대상 = `codex-cli 0.153.4`, 측정 시각 = 2026-09-09 17:18~17:29 KST).
>
> 원시 캡처(stdout/stderr 스트림·드라이버 스크립트·생성된 스키마)는 세션 스크래치패드에 있었고 **repo 에 들이지 않았다** — 여기 실린 수치가 그 캡처에서 뽑은 것이고, 재현 명령은 각 절에 적었다.

---

## 1. 세 모드 중 무엇으로 조종하나 — 실측 비교

`codex` 는 실행 파일 하나에 모드가 붙는다: 인자 없이 = 사람용 대화 화면(Phase 1 이 쓰는 것) · `codex exec` = 한 번 시키고 끝 · `codex app-server` = 다른 프로그램이 JSON-RPC 로 조종.

### 1-1. `codex exec --json` — ★글자가 흐르지 않는다★

```
codex exec --json --sandbox read-only --skip-git-repo-check "say the word BANANA and nothing else" </dev/null
```

- 종료코드 **0** · stdout **345 바이트 / 4 줄** · 프레이밍 = 줄단위 JSON(`Content-Length` 없음).
- **도착한 이벤트 전부 넷, 이 순서:** `thread.started` → `turn.started` → `item.completed` → `turn.completed`. 그 앞뒤로 아무것도 없다.
- **대화 id 는 `thread.started` 에만** 실린다 — 필드 `thread_id`, 값 예 `01a0853f-a030-7373-b5c8-a5aef3287cb8`. **UUIDv7 확인**(버전 니블 7 · RFC4122 variant · 48비트 ms 접두가 측정 시각으로 디코드). 뒤 세 줄엔 id 가 없다.
- `item.completed` 페이로드 = `{"item":{"id":"item_0","type":"agent_message","text":"BANANA"}}` — 그 `item_0` 은 **실행 내 카운터**이고 모델의 메시지 id 가 아니다.
- `turn.completed` 페이로드 = `usage{input_tokens, cached_input_tokens, cache_write_input_tokens, output_tokens, reasoning_output_tokens}`.
- **추론 이벤트가 한 건도 없다**(`reasoning_output_tokens: 0`). **증분 이벤트 자체가 없다.**
- **stdout 은 전부 유효한 JSON**이었다. ★비-JSON 한 줄은 **stderr** 에 있다★ — `Reading additional input from stdin...`, stdin 을 `/dev/null` 로 줘도 나온다(프롬프트를 인자로 준 경우. `exec resume` 에선 안 나옴).
- 저장된 rollout(`~/.codex/sessions/2026/09/09/rollout-<ts>-<threadid>.jsonl`)이 기록한 `duration_ms: 4327`. 모델 = `gpt-5.6-sol`, reasoningEffort `high`, 컨텍스트 창 258400.
- ★**rollout 파일의 어휘가 stdout 어휘와 다르다**★ — snake_case 계열(`task_started`·`item_completed`·`token_count`·`task_complete`·`world_state`·`turn_context`·`token_usage_record`). 즉 「디스크 기록」과 「stdout 이벤트」는 별 어휘다.
- 바이너리 문자열 증거(라이브 관측 아님): `codex.exe` 안에 serde 태그 표가 연속으로 있다 — `ThreadStarted|thread.started` · `TurnStarted|turn.started` · `TurnCompleted|turn.completed` · **`TurnFailed|turn.failed`** · **`ItemStarted|item.started`** · **`ItemUpdated|item.updated`** · `ItemCompleted|item.completed`. **선언 7종 중 관측 4종.**

### 1-2. `exec` 로 이어가기

- 형태 = **`codex exec resume [SESSION_ID] [PROMPT]`**(그 외 `--last`·`--all`). 최상위 `codex resume` 는 **대화형 TUI** 라 다른 명령이다.
- ★`codex exec resume` 에는 `-s/--sandbox` 와 `-C/--cd` 가 **없다**★(있는 것 = `--skip-git-repo-check`·`--json`·`-o`·`--ephemeral`·`--thread-source`).
- 실제로 통한 명령:
  ```
  codex exec resume 01a0853f-a030-7373-b5c8-a5aef3287cb8 "what word did you just say?" --json --skip-git-repo-check </dev/null
  ```
  종료코드 0 · 7.6초 · 같은 네 이벤트 · `thread.started` 가 **같은 `thread_id`** 를 되돌려 줬다.
- **맥락이 실제로 유지됐다** — 답이 `"BANANA"` 였고 `cached_input_tokens: 16512 / input_tokens: 16733`.
- id 출처 = 1-1 의 `thread.started` 줄(rollout 파일명 끝 성분과 같다). **resume 은 같은 rollout 파일에 이어 붙인다** — 새 파일을 만들지 않는다.

### 1-3. `codex app-server` — 조각이 흐르고 중단이 된다

★**PATH 의 `codex` 는 shim 이라 프로그램에서 직접 띄우면 `ENOENT` 다**★(Node `spawn('codex')` 실패). 실측은 실 바이너리로 했다:
`…\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe app-server --stdio`. (우리 코드가 이미 `cmd.exe /c` 로 감싸는 것이 같은 이유의 처리다.)

- 프레이밍 = 줄단위 JSON-RPC 2.0. ★**서버 응답은 `jsonrpc` 필드를 생략한다**★ — `{"id":N,"result":{…}}` / `{"error":{…},"id":N}`.
- `initialize` params `{clientInfo:{name,version}}` → `result{userAgent, codexHome, platformFamily, platformOs}`, 약 130ms.
- **`thread/start` 응답 모양**(보낸 params = `{cwd, sandbox:"read-only", approvalPolicy:"never"}`):
  `result.thread{ id, extra, sessionId, forkedFromId, parentThreadId, preview, ephemeral, section, sectionEnteredAt, projectId, historyMode, modelProvider, model, reasoningEffort, createdAt, updatedAt, recencyAt, status{type:"idle"}, path, cwd, cliVersion, source, canAcceptDirectInput, threadSource, agentNickname, agentRole, gitInfo, name, turns[] }` + 형제 `result.{model, modelProvider, serviceTier, cwd, runtimeWorkspaceRoots, instructionSources, approvalPolicy, approvalsReviewer, sandbox, activePermissionProfile, reasoningEffort, multiAgentMode}`.
  ★`thread.id` == `thread.sessionId`★ = 예 `01a08544-94ef-7043-8a1e-87d22dbf8b49` — **UUIDv7**. 보낸 sandbox 는 `{"type":"readOnly","networkAccess":false}` 로 되돌아왔다. **git repo 아닌 cwd 에서 불평이 없었다.**
- `turn/start` → `result.turn{id, items, itemsView, status:"inProgress", error, startedAt, completedAt, durationMs}`. ★**턴 id 는 `result.turn.id` 이고 `result.turnId` 가 아니다**★.
- **턴 중 도착한 알림, 첫 등장 순서:** `remoteControl/status/changed` · `thread/started` · `mcpServer/startupStatus/updated` · `thread/status/changed` · `turn/started` · `item/started` · `item/completed` · **`item/agentMessage/delta`** · `thread/tokenUsage/updated` · `account/rateLimits/updated` · `turn/completed`. 전부 `emittedAtMs` 를 싣는다. `item/agentMessage/delta` params = `{threadId, turnId, itemId, delta}` — 조각 하나씩 오고, 「1부터 30까지 세라」에서 중단 전까지 **10개** 도착.
- `item/started`·`item/completed` 에서 본 아이템 타입 = **`userMessage`** · `agentMessage` · `reasoning`. ★`reasoning` 아이템은 `summary: []` · `content: []` 로 닫혔다★ — `reasoningOutputTokens: 34` 인데도 비어 있고, `item/reasoning/textDelta` 계열은 **한 건도 안 왔다**(기본 설정에서).
- ★**서버→클라 요청이 0건이다**★ — 두 실행에서 들어온 116줄 중 `method` 와 `id` 를 함께 가진 메시지가 없다(`approvalPolicy:"never"` + read-only 조건).
- ★**`turn/interrupt` 가 동작한다**★ — `{threadId, turnId}` → `{"result":{}}` **18ms**(첫 조각 도착 300ms 후 발사). 조각이 즉시 멈추고, `thread/status/changed` + `turn/completed` 가 `turn.status:"interrupted"` · `items: []` · `itemsView:"notLoaded"` · `durationMs: 4437` 로 왔다. **중단된 메시지의 `item/completed` 는 오지 않는다** — 그 `item/started` 가 닫히지 않은 채 남는다.
  - 인자 실수 기록: `turnId: null` 을 보내면 `{"error":{"code":-32600,"message":"Invalid request: invalid type: null, expected a string"}}`.
- ★**중단 뒤 같은 대화가 그대로 쓰인다**★ — 같은 `threadId` 로 두 번째 `turn/start` 가 새 턴 id 를 받아 `status:"completed"` 까지 갔다. 단 **끊긴 부분은 맥락에 안 실렸다** — 「멈추기 전까지 몇까지 셌나」에 `"0"` 이라고 답했다.
- **stdin 을 닫으면 종료코드 0**, 46ms(2회차) / 118ms(1회차).
- ★**stdout 에 비-JSON 줄이 0**, stderr 0 바이트★(두 실행 모두).
- **로컬 스키마 생성이 된다** — `codex app-server generate-json-schema --experimental --out <dir>` 종료코드 0, **네트워크·모델 호출 없음**. 선언 규모 = **`ClientRequest` 155종 · `ServerNotification` 81종 · `ServerRequest` 11종** · `ClientNotification` 1종(`initialized`). 그 11종 = `item/commandExecution/requestApproval` · `item/fileChange/requestApproval` · `item/tool/requestUserInput` · `mcpServer/elicitation/request` · `item/permissions/requestApproval` · `item/tool/call` · `account/chatgptAuthTokens/refresh` · `attestation/generate` · `currentTime/read` · `applyPatchApproval` · `execCommandApproval`. ★선언 ≠ 관측 — 11종 중 도착한 것은 0★. (`generate-ts` 도 있다.)

### 1-4. 캔슬 비교

- **`exec`**: 「1부터 30까지 한 줄에 하나씩 세라」를 4.5초에 `taskkill /F /T` 로 죽였다(자식 프로세스가 있어 `/T` 가 함께 죽였다). 래퍼 종료코드 **1**, 5.12초. ★**그 4.5초 시점까지 나온 것은 `thread.started`·`turn.started` 두 줄뿐**★ — 같은 시각 app-server 는 이미 조각을 보내고 있었다. 이것이 「exec 는 글자를 흘리지 않는다」의 직접 증거다.
- **강제 종료 후 이어가기가 됐다** — `codex exec resume <id> …` 종료코드 0, 같은 `thread_id`, 같은 rollout 파일에 이어 붙었다.
- ★**끊긴 턴은 반쪽만 남는다**★ — rollout 에 그 턴의 `task_started` 와 **사용자 메시지**는 있고, **어시스턴트 출력도 `task_complete`/중단 표식도 없다**. 이어서 물으니 `"0"`.

### 1-5. 공식 SDK

`codex --help` 에서 `sdk|library|npm|typescript|python` 검색 0건. 설치된 `@openai/codex@0.153.4` 는 `bin/codex.js` + README 뿐이고 `sdk` 검색 0건. **CLI 도움말에 없다.** 가장 가까운 것 둘은 라이브러리가 아니라 코드 생성·프로토콜이다 — `codex app-server generate-ts` · `codex mcp-server`.

---

## 2. 권한·승인 값 목록 실측

**도움말 텍스트와 생성된 스키마에서 뽑은 것이고, 값별 「의미」는 도움말이 적은 문구 그대로만 옮겼다.**

### 2-1. codex — 축이 둘 + 이름 프로필

| 축 | 플래그 | 받는 값 | 기본값 | 도움말이 말하는 것 |
|---|---|---|---|---|
| 샌드박스 | `-s, --sandbox <SANDBOX_MODE>` | `read-only` · `workspace-write` · `danger-full-access` | 도움말에 없음 | "Select the sandbox policy to use when executing model-generated shell commands" — **값별 설명 없음** |
| 승인 | `-a, --ask-for-approval <APPROVAL_POLICY>` | `on-request` · `never` | 도움말에 없음 | `on-request` = "The model decides when to ask the user for approval" · `never` = "Never ask for user approval Execution failures are immediately returned to the model" |
| 이름 프로필 | `-P, --permission-profile <NAME>` | 자유 문자열 | 도움말에 없음 | "Named permissions profile to apply from the active configuration stack" |

- ★**`-a` 는 최상위에만 있고 `codex exec` 에는 없다**★ — `codex exec --ask-for-approval` 은 `error: unexpected argument` 로 죽는다. `-s` 는 양쪽에 다 있다. `-P` 는 **`codex sandbox` 서브커맨드에만** 있다.
- 값을 안 받는 플래그(양쪽 다): `--approve-for-me`("Route approval requests through automatic review using the workspace-write sandbox") · `--dangerously-bypass-approvals-and-sandbox` · `--dangerously-bypass-hook-trust`.

**생성된 스키마에서(`v2/`):**

- `ThreadStartParams` `AskForApproval` = enum `"untrusted"`·`"on-request"`·`"never"` **또는** 객체 `{granular:{mcp_elicitations, rules, sandbox_approval, request_permissions(기본 false), skill_approval(기본 false)}}`. ★`"untrusted"` 는 스키마에 있는데 CLI 가 거부한다★(`[possible values: on-request, never]`).
- `SandboxMode` = `"read-only"`·`"workspace-write"`·`"danger-full-access"`(CLI 와 동일).
- `properties.permissions` = `string|null`, "Named profile id for this thread. **Cannot be combined with `sandbox`**."
- `ApprovalsReviewer` = `"user"`·`"auto_review"`·`"guardian_subagent"`(설명 = 기본 `user`, `guardian_subagent` 는 호환용).
- `ThreadStartResponse` `SandboxPolicy` = `{type:"dangerFullAccess"}` · `{type:"readOnly", networkAccess}` · `{type:"externalSandbox", networkAccess}` · `{type:"workspaceWrite", networkAccess, writableRoots, excludeSlashTmp, excludeTmpdirEnvVar}`. `NetworkAccess` = `"restricted"`·`"enabled"`. 설명이 "Legacy sandbox policy retained for compatibility. Experimental clients should prefer `activePermissionProfile`" 라고 적는다.
- `ConfigReadResponse` `AppToolApproval` = `"auto"`·`"prompt"`·`"writes"`·`"approve"`.
- `ActivePermissionProfile` = `{id(필수), extends}`. id 설명 = "Identifier from `default_permissions` or the implicit built-in default, such as `:workspace` or a user-defined `[permissions.<id>]` profile." ★**id 집합은 프로토콜이 고정하지 않는다**★ — `PermissionProfileSummary{id, allowed, description}` + `nextCursor` 로 **런타임에 열거**한다.

### 2-2. claude — 축이 하나, 값 여섯

| 축 | 플래그 | 받는 값 | 기본값 | 도움말이 말하는 것 |
|---|---|---|---|---|
| 권한 모드 | `--permission-mode <mode>` | `acceptEdits` · `auto` · `bypassPermissions` · `manual` · `dontAsk` · `plan` | 도움말에 없음 | "Permission mode to use for the session" — ★**여섯 값 전부 개별 설명이 없다**★ |

잘못된 값을 주면 같은 목록을 되돌려 준다(세션은 시작되지 않는다): `Allowed choices are acceptEdits, auto, bypassPermissions, manual, dontAsk, plan.`

그 외 권한 관련 플래그: `--allowedTools`/`--disallowedTools`(도구 이름 목록, `Bash(git *)` 꼴 패턴) · `--tools`(`""`=없음, `default`=전부) · `--add-dir` · `--dangerously-skip-permissions`·`--allow-dangerously-skip-permissions` · `--restricted`(명령 실행 도구·WebFetch 제거, `bypassPermissions` 거부, 파일 도구를 작업 디렉터리로 한정) · `--permission-prompts <host|none>`(기본 `host`) · `--settings` · `--setting-sources` · `--safe-mode` · `--bare`.

### 2-3. 우리 코드가 지금 넣는 값

- **claude** — `crates/engram-dashboard-agent/src/backend/claude/mod.rs:128-129` 가 `--permission-mode` + `bypassPermissions` 를 **맨 앞 두 인자로 무조건** 넣는다(그 위 주석: 「control endpoint 유무·spawn 모드와 무관하게 **무조건** 주입」).
- **codex** — `crates/engram-dashboard-agent/src/backend/codex/mod.rs:104-107` 이 `-s workspace-write -a on-request` 를 `--cd <cwd>` 뒤에 넣는다(상수 `:46-51`). ★대화형 `codex` 에 붙는다 — `exec` 에는 `-a` 가 없으니 그 조합이 맞다★.
- **덮어쓸 수 있나:** 뒤에 붙이는 것만 가능하다(두 백엔드 다 호출자 `extra_args` 를 우리 인자 뒤에 잇는다 — `codex/mod.rs:108`). ★**뒤 값이 이기는지는 미검증**★ — 우리 소스가 그 공백을 이미 적어 뒀다(`claude/mod.rs:574`).

### 2-4. 매칭 판정 — ★값끼리 짝지을 근거가 없다★

- **도움말 문구만으로 대응이 서는 것은 하나뿐이다** — 「전부 무시」 계열: codex `--dangerously-bypass-approvals-and-sandbox` ↔ claude `--dangerously-skip-permissions`(그리고 claude 자신의 `--restricted` 설명이 `bypassPermissions` 를 지목하므로 그 값이 축 안의 짝이다).
- 그 밖에 문구가 겹치는 것: `--add-dir`(codex "additional directories that should be **writable**" ↔ claude "additional directories to allow tool **access**" — 문구가 다르다) · codex `--approve-for-me` ↔ codex 스키마 `approvalsReviewer:"auto_review"`(같은 도구 내부 짝, claude 쪽 대응 없음).
- ★**codex `-s` 세 값과 claude 여섯 값은 어느 쌍도 도움말로 지지되지 않는다**★ — 양쪽 다 값별 설명이 없다.
- **대응 없는 것(요약):** codex 의 샌드박스 축 전체 · 네트워크 접근 · 쓰기 가능 경로 · 이름 프로필 · 승인 세부 토글 다섯 · `approvalsReviewer` 의 자동 검토 값 ↔ claude 의 `acceptEdits`·`plan`·`auto`·`manual` · 도구별 허용·차단 목록 · `--restricted`·`--safe-mode`·`--bare`.
- **축 개수 불일치:** codex CLI = 값 받는 축 둘(샌드박스 3값 × 승인 2값, 서로 독립) + 이름 프로필 축 하나. app-server 프로토콜 = 셋(`approvalPolicy`·`sandbox`·`permissions`, ★`permissions` 와 `sandbox` 는 상호 배타★). claude = 한 축 6값. ★**claude 의 한 축이 못 덮는 것은 샌드박스 축**★ — 실행되는 명령의 파일 쓰기·네트워크 범위를 `--permission-mode` 값에 묶는 문장이 claude 도움말에 없고, 가장 가까운 것들(`--add-dir`·`--restricted`)은 축의 값이 아니라 별 플래그다.

---

## 3. 이 문서가 재지 못한 것

- **도구를 실제로 실행하는 턴** — 읽기 전용·승인 없음·자잘한 프롬프트로 돌려서 명령 실행이 일어나지 않았다. 그래서 `exec` 의 `item.started`/`item.updated` 가 언제 오나, 명령 출력이 조각으로 흐르나, **승인 요청 11종이 실제로 오나**가 전부 미측정이다.
- **추론 텍스트** — 기본 설정에서 `reasoning` 아이템이 빈 내용으로 왔다. 어떤 설정을 켜면 `item/reasoning/textDelta` 가 나오는지 안 봤다.
- **턴 도중에 stdin 을 닫으면** 어떻게 되나 — 측정한 것은 「턴 없을 때 닫으면 46ms 에 종료 0」이다.
- **프로세스가 죽은 뒤 다시 붙었을 때** 서버가 진행 중이던 턴 상태를 다시 알려 주나.
- **살아 있는 대화의 승인 정책을 바꿀 수 있나** — `ClientRequest` 155종 중에 그런 메서드가 있는지 안 봤다.
- **`"untrusted"`** 를 app-server 가 받나(스키마엔 있고 CLI 는 거부).
- **`GranularAskForApproval` 객체 꼴에 CLI 표면이 있나** — 도움말에 없다.
- **codex 이름 프로필 id 집합** — 프로토콜이 고정하지 않고, `-P __probe__` 는 `Error: default_permissions requires a [permissions] table` 만 냈다.
- **네 기본값**(codex `-s`·`-a`·`-P`, claude `--permission-mode`) — 도움말도 오류 메시지도 기본값을 안 적는다.
- **`extra_args` 로 같은 플래그를 또 넣으면 뒤 값이 이기나** — 양쪽 다 미검증(우리 소스도 그렇게 적어 뒀다).
- **공유 데몬 모드** — `codex app-server daemon start|stop` · `codex agents` · `--listen unix://|ws://` · `codex app-server proxy` 를 안 건드렸다. 측정한 서버는 stdin 과 함께 죽는 평범한 자식 프로세스였다.
