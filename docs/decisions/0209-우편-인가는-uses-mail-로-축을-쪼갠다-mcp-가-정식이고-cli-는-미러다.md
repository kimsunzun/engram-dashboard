# ADR-0209: 우편 인가는 uses_mail 로 축을 쪼갠다 — MCP 가 정식이고 CLI 는 미러다

- 상태: 확정 (2026-09-19, 근거: commit `75bdfda` · 사용자 결정 2026-09-18 2건)
- 관련: CLAUDE.md 「백엔드 확장」 · ADR-0133(표식 + 단일 파생점) · ADR-0132(제어 평면 CLI) · ADR-0128(채널 하드 단일화) · ADR-0004(백엔드가 자기 사실을 말한다) · `crates/engram-dashboard-agent/src/backend/mod.rs:451` · `crates/engram-dashboard-agent/src/backend/codex/mod.rs:240` · `crates/engram-dashboard-daemon/src/control/mod.rs:179` · step-log S21

## 맥락

codex 에 제어 평면 입구를 열려면 `supports_control_channel` 을 켜야 한다(ADR-0208 — 조립점이 그 칸으로 provision 을 건너뛴다). 그런데 그 칸을 켜는 순간 **우편 보내기 인가까지 함께 열렸다.** 우편 가부를 데몬이 `!accepts_mcp_config` **한 축으로** 파생했기 때문이다.

결과는 반쪽이다 — codex 는 `reads_messages=false`(터미널 모드에 턴 신호가 없어 「바쁜 때」를 못 가린다)인 채로 **보내기만** 열렸다. 그 비대칭은 **아무 선언에도 적혀 있지 않았고**, 데몬은 「CLI 우편을 가르쳤다」고 기록하는데 실제 스폰은 그 교육을 한 글자도 받지 않는 상태로 갈렸다.

사용자 판단은 ★반쪽으로 열린 상태가 가장 나쁘다★였다(「나중에 하는 거 아니냐」, 2026-09-18) — 지금 되돌린다.

그리고 어느 쪽이 정식 경로인가에서 어시스턴트가 **거꾸로 알았고 사용자가 정정했다**(2026-09-18): ★우편은 MCP 가 정식이고 CLI 는 미러다★. 근거는 코드에 이미 있었다 — CLI 헤더가 `mail` 계열을 「MCP 툴 2종의 미러」로 적고(`crates/engram-dashboard-daemon/src/bin/engram.rs:2-3`), ADR-0132 의 「CLI 로만 낸다」는 짝이 되는 MCP 툴이 **없는** `agent`(제어) 계열에만 해당한다(같은 파일 `:3-4`). 기존 파생식(`mail_allowed = !accepts_mcp_config`)이 이미 그 정책이었다 — **MCP 가 있으면 MCP, 없으면 CLI.**

## 결정

1. **우편 인가 축을 `uses_mail()` 로 쪼갠다 — backend 가 스스로 말한다.** 어느 프로그램이 우편 평면에 드는지는 **프로그램별 사실**이므로 그 선언은 backend 가 진다(ADR-0004). 데몬은 이 값과 `accepts_mcp_config` 둘로 **한 값**(`mail_allowed`)을 파생한다 — ★ADR-0133 결정 2 의 단일 파생점은 그대로이고, 갈린 것은 재료가 하나에서 둘로 는 것뿐이다★(`crates/engram-dashboard-daemon/src/control/mod.rs:179` = `needs.uses_mail && !accepts_mcp_config`).

2. **codex 는 `uses_mail() -> false`** — 받기 축(`reads_messages`)과 **같이** 닫는다. 이 칸이 닫는 것은 우편 입구(`/control/send`·`/control/messages`) 뿐이고, **CLI 입구(토큰·주소)와 제어 라우트는 `supports_control_channel` 이 그대로 연다** — `false` 는 「우편을 못 쓴다」이지 「제어를 못 쓴다」가 아니다(제어 동사는 전원 개방 — ADR-0132).

3. **codex 우편의 정식 경로는 codex 에 MCP 를 붙이는 것이고, 그것은 별개 덩어리다.** CLI 우편을 지금 여는 것은 미러를 정식으로 착각하는 것이다.

4. **여는 순서를 못 박는다 — 받기 축을 먼저 연다.** 보내기만 먼저 열면 답장을 못 받는 발신자가 생기고, 그것은 우편 장부에 **영원한 미결**로 남는다. codex 받기 축을 여는 조건은 별도로 적혀 있다(그 축을 모드별로 가르는 것 — 이 메서드가 `command` 를 안 받아 두 모드를 못 가르고, 터미널 모드에는 턴 신호가 하나도 없다).

## 거부한 대안

- **한 축(`accepts_mcp_config`)을 그대로 두고 둘 다 굴린다** — 그러면 codex 가 **반쪽으로 열린 채** 남는다(보내기 인가 · 받기 불가, 어느 선언에도 안 적힌 비대칭). ★사용자가 그 상태를 가장 나쁘다고 판정했다★(2026-09-18). 기각 근거는 코드에도 남아 있다 — grant 목록이 `uses_mail == false` 면 한 줄도 내지 않도록 게이트를 받은 것이 그 자리다(`crates/engram-dashboard-daemon/src/control/mod.rs:72-74`): 그 게이트가 없으면 데몬이 「발신을 허가했다」고 기록하는데 같은 자격증명의 우편 요청은 거절되는 상태가 난다.
- **codex 에 CLI 우편을 지금 연다** — CLI 는 **미러이고 정식 경로가 아니다**(사용자 정정 2026-09-18 · 코드 근거 = 위 「맥락」의 CLI 헤더 두 줄). 정식 경로는 MCP 이고, codex 의 MCP 주입은 전역 TOML 오버라이드(`-c mcp_servers.<name>={…}`)라 claude 의 `--mcp-config <path>` 와 **기제가 다르다**(실측) — 그 배선은 이 단계의 범위가 아니다.

## 근거

- **사용자 결정 2건(2026-09-18)** — ① codex 에 딸려 열린 CLI 우편 인가를 되돌린다(반쪽 상태가 최악) ② 우편은 MCP 가 정식, CLI 는 미러(어시스턴트의 반대 방향 이해를 사용자가 정정).
- **코드 근거** — CLI 헤더의 「MCP 툴 2종의 미러」(`crates/engram-dashboard-daemon/src/bin/engram.rs:2-3`) · ADR-0132 의 CLI-only 가 `agent` 계열 한정이라는 같은 헤더의 다음 줄 · 기존 파생식이 이미 「MCP 있으면 MCP, 없으면 CLI」였다는 사실.
- **조사 정본 = `docs/research/codex-session-id-recovery-survey-2026-09-19.md`**(이 축이 드러난 것은 그 조사가 codex 제어 평면 입구를 열게 만든 결과다 — ADR-0208).
- **커밋 = `75bdfda`.** 회귀 = 워크스페이스 54 바이너리 · 2465 통과 · 0 실패. ★**claude 는 모든 식에서 무변화다**★(기본값 `uses_mail() = true` 를 그대로 받는다).
- ★**게이트 열화는 ADR-0208 과 공유한다**★ — 같은 커밋이라 2차 리뷰 단독 family · 마지막 수정분 미재리뷰가 이 결정에도 걸린다.

## 영향 / 불변식

- **`uses_mail` 의 기본값은 `true`(fail-open)다** — 모른다고 우편을 끊으면 편지가 조용히 사라지므로, **우편 평면 밖에 있는 backend 만 스스로 false 를 선언한다**(`crates/engram-dashboard-agent/src/backend/mod.rs:451`).
- ★**잔여 부채: `shell` 이 `uses_mail` 을 선언하지 않는다**★ — `reads_messages=false` 인데(입력이 명령으로 **실행되기** 때문 — codex 와 분류 사유가 다르다) 기본값이 fail-open 이라, 이 ADR 이 세운 규율(「두 축이 갈린 행은 사유를 그 backend 폴더에 적는다」)을 즉시 어긴다. **오늘 무해한 이유는 `supports_control_channel=false` 하나뿐이고**(`crates/engram-dashboard-agent/src/backend/shell/mod.rs:29-31`), 그것은 이번에 고친 버그와 **정확히 같은 모양**이다 — 선언 없는 보내기 인가가 다른 칸의 부수효과로 열리는 것. 그 칸을 켜는 날 같은 결함이 되돌아온다.
- **ADR-0133 결정 1 의 숨김 부류가 둘이 됐다** — 그 결정이 가르는 축(`engram help` 에서 우편 계열이 보이나)의 옛 부류는 **MCP 가능 스폰** 하나였고, 이제 **우편 평면 밖 스폰**(`uses_mail=false`)이 둘째다. ★**ADR-0133 을 폐기하지 않는다**★ — 단일 파생점도 「표식은 교육, 강제는 데몬 거절」도 그대로 서고, 바뀐 것은 그 축에 드는 부류 수뿐이다.
- **단일 파생점은 하나로 유지한다** — `mail_allowed` 를 만드는 자리는 데몬의 그 한 줄뿐이다(ADR-0133 결정 2). 재료가 둘로 늘었다고 파생을 소비처마다 다시 적지 말 것.
- **`uses_mail=false` 스폰에는 프라이밍 우편 교육도, 발신 입구 grant 도 나가지 않는다** — grant 는 위 게이트가, 프라이밍 변형은 `needs.uses_mail.then_some(…)` 이 가른다(`crates/engram-dashboard-daemon/src/control/mod.rs:214`). 그 셋(인가·교육·grant)이 갈리면 「가르쳤는데 거절된다」가 다시 난다.
