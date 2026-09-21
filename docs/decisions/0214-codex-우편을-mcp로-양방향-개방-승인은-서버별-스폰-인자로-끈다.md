# ADR-0214: codex 우편을 MCP로 양방향 개방 — 승인은 서버별 스폰 인자로 끈다

- 상태: 확정 (2026-09-20, 근거: 사용자 결정 + codex 0.155.0 실측 3경로 + 코드 실측)
- 관련: ADR-0116(턴 신호 없음 → 게이트 없이 즉시 주입 · 결정 7) · ADR-0209(보내기·받기를 함께 연다) · ADR-0133(우편 표면 게이트) · ADR-0213(`eg_` 접두) · ADR-0094(grant 단일 출처) · ADR-0097(claude 는 bypassPermissions) · ADR-0210(훅 신뢰 — 별개 축) · `crates/engram-dashboard-agent/src/backend/codex/mod.rs` · `crates/engram-dashboard-daemon/src/control/mod.rs:184` · step-log S21

## 맥락

codex 는 우편에서 통째로 빠져 있었다 — `uses_mail`·`reads_messages` 가 둘 다 `false` 였고, 그래서 살아 있는 codex 에이전트에게 보낸 편지는 **큐에 쌓이지도 않고 「수신자 없음」으로 입구에서 튕겼다**(`messaging_host.rs:165` 의 명부 필터).

그 `false` 에 적힌 사유는 「바쁜 때를 못 가린다 — 터미널 모드에 턴 신호가 없다」였다. ★**그런데 그 사유는 터미널 claude 에도 똑같이 적용되는데, 터미널 claude 는 이미 받고 있다**★. 배달은 PTY 에 바이트를 밀어 넣는 것이고(`session.rs:251` 본문 → `confirm_written` → 500ms → 제출 바이트), claude·codex 가 **같은 코드**를 탄다. 그리고 ADR-0116 결정 7 이 이미 「턴 신호 없음 → 게이트 없이 즉시 주입」을 정책으로 박아 두었으며, 그 ADR 의 거부한 대안에 **「관측할 수 없으니 배달할 수 없다」가 명시적으로 기각**돼 있다. 즉 막고 있던 것은 기계적 제약이 아니라 **선언 두 줄**이었다.

남은 진짜 문제는 **보내는 쪽** 하나였다. 받기는 우리가 미는 것이라 게이트가 낄 자리가 없지만, 보내기는 **에이전트가 우리 도구를 부르는** 것이라 codex 의 승인 정책에 걸린다.

## 결정

1. **codex 우편을 양방향으로 연다** — `uses_mail`·`reads_messages`·`accepts_mcp_config` 를 모두 `true`. ADR-0209 가 요구한 「보내기·받기를 함께」를 지킨다.
2. **우편 수단은 MCP 다(CLI 미러 아님)** — CLI 호출이 **호출당 약 0.3초** 느리고(사용자 실측), 우편은 요청·답장·확인으로 왕복이 잦아 그 차이가 누적된다.
3. **MCP 도구 승인은 스폰 인자로 끈다** — 서버 오버라이드에 `default_tools_approval_mode='approve'` 를 더한다. **범위는 `mcp_servers.engram` 하나**이고, `approvalPolicy: on-request` 와 `sandbox: workspace-write` 는 그대로 둔다 — 셸·exec 승인은 살아 있다.
4. **「파일을 쓰나」 축을 분리한다** — 새 칸 `writes_mcp_config_file`(기본 `false`, claude 만 `true`). `accepts_mcp_config` 는 우편 라우팅(`mail_allowed = uses_mail && !accepts_mcp_config`)만 파생한다.
5. **MCP 를 끄는 기존 스위치가 codex 에도 닿게 한다** — 백엔드는 환경변수를 다시 읽지 않고, 데몬이 내린 결정이 남긴 **단 하나의 관측 가능한 신호**(엔드포인트가 우리 서버의 MCP grant 를 들고 있나)를 본다. grant 가 없으면 오버라이드를 **안 내보낸다.**
6. **입구를 만들 수 없으면 codex 스폰을 닫는다** — 「엔드포인트가 있고 · MCP grant 가 있고 · 그런데 오버라이드 문자열을 만들 수 없다」일 때만 실패시킨다. **엔드포인트가 아예 없거나 grant 가 없는 것은 정상 스폰**이다(우편 없이 도는 codex · 스위치로 끈 경우). claude 는 데몬의 파일 쓰기 `?` 가 이미 같은 일을 하므로 이 seam 은 **파일을 안 쓰는 백엔드**만 덮는다.

## 거부한 대안

- **CLI 미러로 우편을 연다** — `accepts_mcp_config=false` 로 두면 `mail_allowed` 가 켜져 `engram mail send` 가 열리고 `engram help mail` 까지 codex 에겐 작동한다(claude 에겐 안 되는 것). 그런데 **호출당 약 0.3초**가 붙고 우편은 왕복이 잦다. *기각 근거 = 사용자 실측.*
- **app-server(json) 모드 전용으로 연다** — 한때의 권고였다. 근거는 「터미널엔 턴 신호가 없다」였는데 **그 전제가 틀렸고**(위 맥락), 이어서 「터미널에선 승인 다이얼로그에 답할 길이 없다」로 갈아탔는데 **그것도 `approve` 가 세 경로 전부에서 게이트를 없애면서 사라졌다**. *기각 근거 = 실측 2회.*
- **사용자가 스폰마다 한 번 누르게 둔다** — 실제로 성립한다(다이얼로그가 에이전트 터미널에 뜨고 대시보드가 그 화면을 이미 보여 준다). 그러나 `approve` 가 있어 누를 필요가 없고, ★**`codex exec` 경로는 이 방법으로 못 연다**★(사람이 없어 무조건 실패). *기각 근거 = 실측.*
- **`--dangerously-bypass-approvals-and-sandbox` 또는 `-s danger-full-access`** — 게이트는 걷히지만 **전역**이라 셸 명령 승인과 샌드박스까지 함께 열린다. ADR-0210 이 `--dangerously-bypass-hook-trust` 를 같은 이유로 기각한 선례와 같은 부류다. *기각 근거 = 실측 + 선행 ADR.*
- **`default_tools_approval_mode='auto'`** — 유효한 값이지만 **게이트를 안 걷는다**(실측). 유효값은 `auto | prompt | writes | approve` 넷이고 **`approve` 만** 걷는다. ★한때 「설정으로는 못 끈다」로 결론 났던 것이 이 값을 잘못 짚어서였다★ — 같은 오진을 막으려고 여기 적어 둔다.
- **`codex mcp add` 로 등록해 신뢰를 얻는다** — 등록본과 스폰 주입본의 저장 모양이 **바이트 단위로 같고**, 등록해도 **똑같이 묻는다**(실측). 등록은 신뢰를 사 주지 않는다. *기각 근거 = 실측.*
- **다이얼로그를 우리가 감지해 키를 쏜다** — 창 제목이 `Action Required` 로 바뀌는 것으로 감지는 되지만 ★**셸 승인에서도 같은 제목이 뜬다**★(실측). 본문까지 봐야 갈리는데, 애초에 `approve` 로 안 뜨게 하는 편이 싸다. *기각 근거 = 실측.*
- **턴 신호를 먼저 만든다** — ADR-0116 결정 7 이 그 전제를 이미 기각했고, 터미널 claude 가 그 없이 돌고 있다는 것이 방증이다. *기각 근거 = 선행 ADR + 코드 실측.*
- **`accepts_mcp_config` 로 파일 쓰기를 계속 파생한다** — codex 는 그 파일을 안 읽는데(토큰을 env 로 받는다) 켜는 순간 **스폰마다 평문 Bearer 토큰 JSON**이 기본 ACL 로 디스크에 쌓이고, 그 쓰기가 **fail-closed** 라 실패하면 스폰이 통째로 죽는다. *기각 근거 = 코드 실측.*

## 근거

- **배달 경로 실측** — `service.rs:1691` `port.inject` → `messaging_host.rs:133` → `manager.rs:2580` → `session.rs:251`(본문 write → flush → 500ms → 제출 바이트). 백엔드 분기 없음. 당기는 경로는 **없다**(`eg_messages` 는 장부 조회이지 수신함이 아니다).
- **턴 신호 실측** — `output_core.rs:341` 이 `OutputEvent` 를 분류하는데 터미널 모드는 `TerminalBytes` 만 흘러 **claude·codex 둘 다 신호가 0 이다**(`claude/mod.rs:499`·`codex/mod.rs:758`). 바쁨 게이트는 관측이 없으면 **idle 로 떨어진다**(`busy.rs:178`) — 즉 큐에 영원히 남는 일은 없고 즉시 배달된다.
- ★**승인 게이트 실측(codex 0.155.0, 버리는 `CODEX_HOME`)**★ — 스텁 MCP 서버를 세워 세 경로를 각각 돌렸다. `approve` 없이: TUI 는 「Allow the engram MCP server to run tool "eg_messages"?」 다이얼로그, `codex exec` 는 「MCP tool call requires approval, but approval policy is never」로 즉사, app-server 는 `mcpServer/elicitation/request`. `approve` 를 넣으면 **세 경로 전부에서 요청이 사라지고** 도구 호출이 스텁에 도달해 마커가 돌아왔다.
- **「항상 허용」은 디스크에 안 남는다** — 승인 전후로 홈 전체를 해시 비교했는데 승인 흔적이 없고 **새 프로세스는 다시 묻는다**. 그래서 지속 경로는 **스폰 인자뿐**이다.
- **붙이는 데는 승인이 없다** — `initialize`·`tools/list` 는 아무 심사 없이 지난다. 승인은 **부를 때** 붙는다. (훅은 반대 모양 — 붙이는 데 심사가 있다. ADR-0210.)
- **토큰 취급** — 명령줄에는 **env 변수 이름만** 실리고 값은 `ENGRAM_TOKEN` 으로 간다. 실측에서 토큰은 `Authorization: Bearer` 헤더에만 나타났고 URL·본문·argv 어디에도 없었다.

## 영향 / 불변식

- **받기는 백엔드 무관하게 같은 코드다** — 이 결정은 그 경로를 바꾸지 않는다. 바뀐 것은 **명부에 오르느냐**(`reads_messages`)뿐이다.
- ★**감수하는 위험: 봉투가 턴 도중에 꽂힐 수 있다**★ — TUI 가 모달 상태(승인 프롬프트·메뉴·플랜 확인)면 위젯이 먹을 수 있다. **터미널 claude 가 이미 같은 수준으로 감수 중**이고 ADR-0116 결정 7 이 그것을 비용으로 명시했다. codex 가 더 나빠지는 것이 아니다.
- ★**훅 신뢰는 이 결정이 건드리지 않는다**★ — `default_tools_approval_mode` 는 `mcp_servers.<서버>` 아래라 MCP 도구 축 전용이다. resume id 회수용 `SessionStart` 훅의 신뢰는 별개이고 ADR-0210 이 소유한다.
  - ★**단 같은 기법이 훅 축에도 통한다는 것이 이 라운드에 실측됐다 — 다음 덩어리로 넘긴다**★. `hooks.state` 아래에 「훅 출처:이벤트:순번」을 키로 `trusted_hash` 를 심으면 **심사 화면 없이 그 훅이 실제로 실행된다**(실측 2회 — 안 심으면 「Hooks need review」, 심으면 통과 + 마커 생성). 범위는 **그 훅 하나**이고 명령을 바꾸면 해시가 달라져 안 덮이므로, ADR-0210 이 기각한 전역 플래그와 다른 물건이다. ★**대가 = 해시를 계산할 수 없다**★ — 후보 8종을 맞춰 봤으나 실패했고, **codex 를 한 번 돌려 읽어 오는 수밖에 없다**. 즉 훅 명령 문자열이 바뀔 때마다 해시를 다시 떠야 하고, 안 뜨면 **조용히 심사 화면으로 되돌아간다**. 그 동기화 부담이 별도 결정거리라 오늘 범위에 넣지 않았다. **곁가지 실측 하나** — `projects."<경로>".trust_level` 은 실재하는 키이고 이미 `trusted` 였는데도 **훅 심사는 그대로 떴다**(프로젝트 신뢰는 훅을 안 덮는다).
- **`reads_messages()` 가 모드를 못 가른다** — 메서드가 `command` 를 안 받아 터미널과 app-server 를 구분할 수 없다. 지금은 둘 다 같은 답이면 되므로 문제가 없지만, **모드별로 갈라야 할 날이 오면 시그니처를 넓히거나 능력을 세션 쪽으로 옮겨야 한다.**
- **`accepts_mcp_config == true` + `writes_mcp_config_file == false` 조합이 합법이 됐다** — codex 가 그 정당한 경우다. 그런데 새 백엔드가 앞을 켜고 뒤를 **잊으면** 조용히 MCP 가 안 붙는다. 타입은 이것을 못 막고 **선언 표(트립와이어)가 유일한 방벽**이다.
- **문서 드리프트** — `docs/reference/structure/agent-backend.md` 의 codex 열이 실제와 어긋나 있었다(이 라운드가 만든 것). 같은 라운드에서 갱신했고, 그 과정에서 **이 라운드와 무관하게 낡아 있던 셀 둘**(`can_resume_stored_session`·`capabilities().session.resume` — 둘 다 「app-server 만」으로 적혀 있었으나 지금은 두 모드 다 `true`)도 함께 고쳤다.
- ★**지시서는 codex 에게 실제로 안 실린다 — 데몬 장부에는 「가르쳤다」로 남는데**★. `wants_priming` 이 codex 에서 참이 되고 엔드포인트에 `priming_file` 이 실리지만, codex 백엔드는 그것을 argv 로 **번역하지 않는다**(실을 자리가 없다 — 경로를 받는 키는 기본 프롬프트를 대체해 버리고 나머지 후보 키는 전부 거절된다). 그래서 「프라이밍이 가르치는 우편 채널 = 그 스폰이 쓸 수 있는 우편 채널」 불변식이 이 줄에서는 **공허하게만** 참이다. 오늘 그 공백을 메우는 것은 **도구 설명문**(ADR-0211)이고, 그건 「어떻게 부르나」만 지며 「무엇을 해야 하나」는 못 진다(설명문에 심은 행동 규칙이 무시된다는 것이 ADR-0211 의 실측이다). **codex 가 지시서를 받는 길은 미해결로 남는다.**
- **선재 비대칭 하나를 발견했으나 이 라운드에서 건드리지 않았다** — `shell` 백엔드는 `reads_messages=false` 인데 `uses_mail` 은 트레이트 기본값 `true` 를 물려받는다. 보내기만 열리고 받기가 닫힌 모양이라 ADR-0209 가 없애려는 그 형태지만, `supports_control_channel=false` 라 실제로는 우편 평면에 못 들어가서 무해하다. 선언 표에 `shell` 행이 없어 **아무것도 이것을 단언하지 않는다** — 다음에 그 행을 세울 때 함께 볼 자리다.
- **회귀망 하나가 합성으로 바뀌었다** — `a_backend_outside_the_mail_plane_gets_control_but_no_mail`(`daemon/tests/mail_gate.rs`)이 `{false,false,false}` 를 손으로 적어 넣는다. **지금 그 조합을 선언하는 백엔드가 없어서** 그렇게 된 것이고, 결과적으로 이 테스트는 **실 선언의 드리프트를 못 잡는다.** 「우편 평면 밖」 경로는 어떤 백엔드가 다시 그 상태에 들어올 때까지 살아 있는 커버리지가 0 이다.
- **검증(미완)** — 실 claude ↔ 실 codex 왕복 우편을 아직 한 번도 안 봤다. 측정은 전부 스텁 서버와 별도 하네스였다. **그 왕복이 이 ADR 의 진짜 검증이고, GUI 실측과 함께 남아 있다.**
