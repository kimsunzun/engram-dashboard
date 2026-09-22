# ADR-0213: MCP 도구 이름에 eg_ 접두 — 에이전트 내장 도구와의 이름 충돌 차단

- 상태: 확정 (2026-09-20, 근거: 사용자 결정 + codex 도구 목록 실측 + 코드 실측) · 부분 폐기 by ADR-0220 (지시서의 engram help mail 금지)
- 관련: ADR-0094(발신 grant seam — 「단일 출처」 불변식이 이름의 정본을 컨트롤 채널 입구로 박았다) · ADR-0004(백엔드 지식 격리) · ADR-0097(bypassPermissions auto mode — grant가 오늘 NO-OP인 사유) · `crates/engram-dashboard-daemon/src/control/mcp_server.rs:330,335` · `crates/engram-dashboard-agent/src/backend/claude/mod.rs:259`(내장 `SendMessage` deny 근거 주석) · `docs/process/S18-messaging-v1/spec/messaging-v1-spec.md` §6 · `docs/research/command-discovery-survey-2026-09-20.md` · step-log S21 · Amends ADR-0126 (프라이밍 에스컬레이션 pin 제거) · Amended by ADR-0220 (지시서의 engram help mail 금지)

## 맥락

데몬이 MCP로 광고하던 도구 이름이 `send_message` · `messages` 였다. 앞의 것은 **에이전트 런타임이 자기 내장 도구로 이미 쓰는 이름**이다.

- **codex 실측(2026-09-20)** — codex 본인에게 자기 도구 목록을 물었더니 `exec_command` · `exec` 외에 ★`send_message` · `spawn_agent` · `list_agents`★ 가 내장으로 있었다. 즉 codex 에 우리 MCP 를 붙이면 **같은 목록에 `send_message` 가 둘** 놓인다.
- **claude 에서는 이미 한 번 터졌다** — `crates/engram-dashboard-agent/src/backend/claude/mod.rs:259` 의 주석이 `--disallowedTools SendMessage` 를 넣은 사유를 「내장 `SendMessage`(PascalCase) 와 우리 `send_message`(snake_case) 가 이름이 겹친다」로 적어 두었다. **충돌은 codex 만의 문제가 아니었고, claude 쪽에선 deny 플래그로 막아 둔 상태였다.**
- 우리 CLI 쪽은 이 문제가 없다 — 전부 `engram` 실행 파일 뒤에 있어 실행 파일 이름 자체가 접두다. 문제는 **도구 목록에 나란히 서는 MCP 표면 하나**뿐이다.

이름은 지금 바꾸는 것이 가장 싸다. MCP 도구 이름은 세션마다 `tools/list` 로 새로 조회되므로 저장된 상태가 없고, codex 에 아직 붙이지 않아 충돌이 실제로 난 적도 없다.

## 결정

1. **MCP 도구 이름에 짧은 접두 `eg_` 를 단다** — `send_message` → **`eg_send`** · `messages` → **`eg_messages`**.
2. **CLI·카탈로그 이름은 접두하지 않는다** — `engram` 실행 파일이 이미 접두이고, `agent.spawn` 류 카탈로그 이름은 프론트 레지스트리·골든 테스트·문서가 함께 쓰는 계약이다.
3. **상수 식별자(`SEND_MESSAGE_TOOL`·`MESSAGES_TOOL`)는 그대로 둔다** — 역할(「발신 도구」)을 가리키는 이름이라 여전히 정확하고, 참조 21곳을 건드리지 않아 diff 가 작아진다. 바뀌는 것은 **값**과, rmcp 가 와이어 이름을 메서드 식별자에서 그대로 뽑으므로 `#[tool]` **메서드 이름**이다.
4. **claude 의 `--disallowedTools SendMessage` 는 유지한다** — 이 결정이 그 deny 의 *명시된 사유*(이름 겹침)를 없애지만, 관측된 실패는 claude 가 **자기 내장 도구를 오발**한 것이라 이름이 달라졌다고 재발하지 않는다는 근거가 없다. 뗄지는 실측 후 별도 결정.

## 거부한 대안

- **`engram_` 긴 접두(`engram_send_message`)** — 뜻은 더 분명하지만 길다. 「괜히 길면 좋을 거 없다」(사용자 결정 2026-09-20). 접두의 목적은 **같은 목록에서 구분**이고 그건 두 글자로 충분하다. *기각 근거 = 사용자 결정.*
- **CLI·카탈로그 이름까지 접두(`engram eg.agent.spawn`)** — ① `engram` 실행 파일이 이미 접두라 같은 말을 두 번 한다 ② 카탈로그 이름은 프론트 레지스트리(`src-tauri/src/layout/commands.rs`)·골든 테스트·문서가 공유하는 계약이라 전부 따라온다 ③ 도구 호출과 셸 명령은 애초에 다른 이름 공간이라 **충돌이 성립하지 않는다**. *기각 근거 = 코드 실측.*
- **이름은 그대로 두고 지시서 문장으로만 가른다**(「네 내장 도구 말고 engram 것을 써라」) — ★ADR-0094 가 같은 논리로 이미 기각한 갈래다★: 권한·도구 선택은 런타임에서 일어나고 **프롬프트는 enforcement 가 아니다**(ADR-0093 C0~C3 실측 — 프라이밍이 발신법을 알려줘도 런타임 게이트가 이겼다). 모델이 **이름을 보고 고르는 자리**에서 문장은 보조일 뿐이다. 실제로 claude 에서는 문장이 아니라 **deny 플래그**로 막았다는 것이 방증이다. *기각 근거 = 선행 ADR 실측 + 코드 실물.*
- **codex 에 MCP 를 붙일 때 그때 바꾼다** — 그때는 돌고 있는 것을 고치는 일이 되고, 이미 스폰된 에이전트가 옛 이름의 grant 를 들고 있다. 지금은 저장된 상태가 없어 비용이 0 이다. *기각 근거 = 코드 실측(아래 「영향」의 전이 노출).*

## 근거

- **codex 도구 목록 실측(2026-09-20)** — codex CLI 0.155.0 에 자기 도구 이름을 직접 물어 받은 답: 셸 실행은 `exec_command`(「`Bash` 는 도구 이름이 아니다 — `exec_command` 를 부르고 그 안에서 셸로 bash 를 고른다」), 그리고 내장으로 `send_message`·`spawn_agent`·`list_agents`·`wait_agent` 등.
- **claude 쪽 선례** — `backend/claude/mod.rs:259` 의 deny 사유 주석이 같은 종류의 충돌을 이미 기록하고 있다.
- **와이어 실측 — 단 무조건적 증거는 하나다.** 이름 변경 후 `d_mcp_and_cli_entrances_return_identical_json_for_messages_and_group` 이 `tools/list` 를 읽고 새 이름으로 호출해 통과했다(상수만이 아니라 와이어에서 확인). ★**`mcp_send_message_tool_happy_and_error` 도 로컬에서는 통과했지만 이것을 증거로 세지 말 것**★ — claude 가 없으면 단언 전에 `skip_no_claude` 로 빠져나가고, CI 는 그 테스트를 이름으로 제외한다. 즉 **claude 가 깔린 기계에서만 도는 증거**다. (적대 리뷰가 잡은 과장 — 초판은 둘 다 무조건적 증거인 것처럼 적었다.)
- **ADR-0094 「단일 출처」 불변식은 지켜지고 있었다** — 이름 변경 착수 시 `backend/claude/mod.rs` 의 `"send_message"` 리터럴 3곳을 불변식 위반으로 의심했으나, ★전부 `#[cfg(test)]` 안의 테스트 픽스처였다★. 프로덕션 경로는 `format!("mcp__{server}__{tool}")` 뿐이라 백엔드는 이름을 타이핑하지 않는다. 그 의심은 기각됐고, 픽스처는 실물을 계속 비추도록 함께 갱신했다.

## 영향 / 불변식

- **이름의 정본은 여전히 컨트롤 채널 입구 정의다**(ADR-0094 결정 4·불변식). 이 ADR 은 그 자리의 *값*만 바꾼다 — 백엔드는 형식(`mcp__{server}__{tool}`)만 알고 이름을 재타이핑하지 않는다.
- **저장·외부 계약 파손 없음** — `ToolGrant`·`ControlEndpoint` 는 serde 파생이 없어 직렬화되지 않고, grant 는 스폰마다 재계산된다. TS·JSON·생성 바인딩·wire 골든 어디에도 이 이름이 없다. HTTP 라우트 `/control/send`·`/control/messages` 는 **CLI 입구라 별개 계약**이고 바뀌지 않았다.
- ★**전이 노출 하나**★ — 이미 떠 있는 에이전트는 `mcp__engram__send_message` 로 스폰된 grant 를 respawn 까지 들고 있다. ADR-0097 로 grant 가 오늘 NO-OP 이라 기능 파손은 아니고 **문서 드리프트**지만, grant 층이 언젠가 강제되면 실제 문제가 된다. 데몬 재기동 + 재스폰이면 해소된다.
- ★**지시서에서 에스컬레이션 교육이 사라졌고, 그것을 지키던 pin 도 함께 사라졌다**★ — 같은 세션에 `prompts/agent-priming.md` 를 발견 장치로 다시 쓰면서 행동 규칙 4줄(고장 난 채널 보고·답장 의무 등)을 사용자 지시로 걷어냈다. 그것을 지키던 테스트(`control::priming::tests` 의 채널-실패 에스컬레이션 단언)를 처음엔 새 지시서가 보장하는 것으로 재조준했으나, ★**적대 리뷰 둘이 독립으로 「그 대체 단언은 바로 옆 테스트의 완전 중복이라 커버리지가 0 이다」를 짚었고 그 지적이 옳아 중복 테스트를 지웠다**★. **지금 이 축을 지키는 테스트는 없다.** ADR-0126 결정 2·5 가 「우회 금지·주인에게 보고」로 박아 둔 것이 프라이밍에서 빠진 채 아무 게이트도 안 걸린 상태다(그래서 이 ADR 이 그쪽에 부분 폐기 도장을 박았다). **결정 5 가 「비-MCP 갈래에 에스컬레이션 교육 0줄」로 남겨 둔 구멍이 이제 모든 갈래로 넓어졌다** — 우편을 여는 단계에서 되메울 자리다.
- ★**지시서가 우편을 가리킬 때 `engram help mail` 을 쓰면 안 된다 — 그 줄은 자기 독자에게 항상 실패한다**★. 주입 조건과 노출 조건이 서로 배타이기 때문이다: 지시서는 `wants_priming = uses_mail && accepts_mcp_config`(`control/mod.rs:222`)일 때만 주입되는데, 도움말이 우편을 보이는 조건은 `mail_allowed = uses_mail && !accepts_mcp_config`(`:184`)다. **그래서 지시서를 받은 에이전트는 예외 없이 `MAIL_MARKER_OFF` 이고 `engram help mail` 은 `unknown help topic` 으로 튕긴다.** 이번 초판이 정확히 그 줄을 썼고 적대 리뷰가 잡았다 — 지금은 그 독자가 실제로 가진 것(MCP 도구 `eg_send`·`eg_messages`)을 가리킨다. ★**덤으로, 그 잘못된 줄이 기존 게이트를 통과했다**★: `mentions_mail_cli_surface`(`control/priming.rs:200`)가 `engram` 과 `mail` 의 인접을 보는데 사이에 낀 `help` 가 그 매칭을 깨뜨려, **CLI 표면 금지 게이트가 초록인 채로** 지나갔다.
- **지시서가 조건부 표면을 무조건으로 약속한다(미해결)** — 창·탭·슬롯·`ui.refresh` 는 셸이 등록하는 17개라 **헤드리스 데몬에서는 목록째 사라진다**. 지시서는 그것을 단서 없이 적는다. 도움말 화면 보수와 함께 닫을 자리다.
- **문서 드리프트가 남아 있다** — `docs/` 약 40파일이 옛 이름을 단다. 이번에 고친 것은 소스 주석들이 「이름의 출처」로 가리키는 `S18 spec §6` 뿐이고, ADR·step-log 등 기록 문서는 append-only 규약대로 손대지 않았다(이 ADR 이 이름 변경을 진다).
- **검증(미완 — codex 배선 때 함께)** — ① codex 에 MCP 를 붙였을 때 `eg_send` 가 내장 `send_message` 와 구분되어 선택되는지 ② ★**지시서를 백엔드별로 갈라야 하는지, 아니면 실행 도구 이름을 아예 빼도 되는지**★ — 「Bash 로 실행해라」 그대로 · 도구 이름 없이 · 백엔드별 델타 셋을 같은 조건에서 돌려 **각 에이전트가 실제로 `engram` 을 치는가**로 판정한다. codex 가 도구 이름 없이도 치면 델타는 불필요하고 지시서는 한 파일로 유지된다.
