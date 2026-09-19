# `engram help` 화면 본문 (ADR-0092 계열 외부화)

에이전트가 제어 평면 표면을 배우는 유일한 자리인 `engram help` 의 **본문**이다. 프라이밍
(`agent-priming.md`)과 같은 이유로 바이너리 밖에 산다 — 문구를 고치는 데 재빌드가 필요하면 그 문구는
사실상 코드다.

★이 파일을 고치면 재빌드 없이 다음 `engram help` 호출부터 반영된다★ — CLI 가 실행 시점에 읽는다.
바이너리에도 같은 내용의 사본이 `include_str!` 로 박혀 있는데 그것은 **이 파일을 못 읽을 때만** 쓰이는
폴백이다(이 화면이 아무것도 못 내면 에이전트는 표면이 없다고 결론짓는다 — 빈 출력은 선택지가 아니다).
폴백으로 내려가면 stderr 에 사유 한 줄이 나가고 stdout 과 종료코드는 그대로다.

## 형식

- 구획 표시 = `<!-- engram:help <구획-id> -->` 한 줄. **그 줄 다음 바이트부터 다음 표시 줄 앞까지가 본문**
  이고 공백·빈 줄·줄바꿈까지 **그대로** 화면에 실린다. 첫 표시 앞의 이 글은 버려진다.
- `{tool}` 은 실행파일 이름으로 치환된다(정본 = agent 의 `CLI_EXE_NAME`). 이름을 손으로 적지 말 것 —
  적으면 실행파일 이름이 갈리는 날 배운 대로 쳐도 명령을 못 찾는다.
- 구획 하나라도 빠지면 이 파일은 **통째로** 거부되고 폴백이 뜬다(반쪽 화면 금지). 모르는 구획은 무시한다
  — 새 파일을 옛 바이너리로 읽는 경우가 정상 경로이기 때문이다.
- 우편 하위 화면의 구획 id 는 주제 토큰에서 **파생**된다(`mail.index.<토큰>` · `mail.page.<토큰>`).
  그래서 주제를 늘리면 여기 구획도 반드시 함께 늘어야 하고, 안 늘면 로드가 거부된다.
- 우편 계열이 감춰진 스폰(ADR-0133)에서는 `root.group.mail` 과 `agent.xref` 두 구획만 빠진다. 그래서
  그 둘이 따로 서 있다 — 다른 구획에 우편을 적으면 필터가 무의미해진다.

<!-- engram:help root.head -->
{tool} — CLI for the Engram broker daemon. Usage: {tool} <group> <verb> [flags]

Groups:
<!-- engram:help root.group.mail -->
  mail     message your teammates and check your own outstanding items
<!-- engram:help root.group.agent -->
  agent    list, create, start, rename and re-parent the agents on this team
<!-- engram:help root.tail -->

Run `{tool} help <group>` for that group's verbs (`{tool} <group> --help` works too).
Run `{tool} commands` for every command the daemon can run right now, `{tool} commands <name>` for one command's arguments and return shape, and `{tool} <name> --flag value` to run it.
<!-- engram:help mail.head -->
{tool} mail — messages between the agents on this team.

Sending is three verbs; receiving takes none, because messages arrive on their own. One page each:
<!-- engram:help mail.index.send -->
  {tool} help mail send      composing and sending, and every flag it takes
<!-- engram:help mail.index.status -->
  {tool} help mail status    where one message you sent has got to
<!-- engram:help mail.index.pending -->
  {tool} help mail pending   what you still owe, and what you are waiting for
<!-- engram:help mail.index.recv -->
  {tool} help mail recv      what arrives on your side, and when a reply is owed
<!-- engram:help mail.tail -->

Your identity is taken from the token the broker injected, never from an argument — there is no flag to send or query as somebody else.

Exit codes: 0 = accepted or read | 1 = rejected, or the daemon could not be reached — stdout carries the daemon's own reply when there was one, and {"status":"error","code":...,"hint":...} when this CLI rejected the call itself | 2 = the daemon answered 2xx in a shape this CLI cannot read; report it, retrying will not help. Judge the outcome by the exit code, not by the shape of stdout.
<!-- engram:help mail.page.send -->
{tool} mail send — send a message to one or more teammates.

  {tool} mail send --to <name[,name...]> (--body <text> | --body-stdin) [--request] [--reply-by <dur>] [--reply-to <m-id>]
      Send a message. Prints one result row per recipient.
      --to <name[,name...]>  teammate name or agent id; comma-separated for several;
                             @here = everyone live except you, @all = every agent in the tree except you
      --body <text>          the body, on the command line
      --body-stdin           read the body from stdin instead (heredoc-friendly); exactly one of --body / --body-stdin
      --request              an answer is owed; you get notified if none arrives
      --reply-by <dur>       deadline for that answer, e.g. 5m / 10m / 1h (1 minute minimum)
      --reply-to <m-id>      this message answers that request; mutually exclusive with --request

Run `{tool} help mail recv` for what a request looks like when it reaches you, and `{tool} help mail` for who you send as and what the exit codes mean.
<!-- engram:help mail.page.status -->
{tool} mail status — where one message you sent has got to.

  {tool} mail status <m-id>
      Delivery state of one message you sent, one row per recipient.

Run `{tool} help mail pending` for everything still open at once, and `{tool} help mail` for who you send as and what the exit codes mean.
<!-- engram:help mail.page.pending -->
{tool} mail pending — what is still open on your side.

  {tool} mail pending
      Your open items: answers you owe, answers you are waiting for, sends not confirmed as delivered yet.

Run `{tool} help mail` for who you send as and what the exit codes mean.
<!-- engram:help mail.page.recv -->
{tool} mail recv — what arrives, and what it asks of you. There is no verb here: messages are delivered to you, you never fetch them.

Messages arrive as XML envelopes. The from label is broker-verified — it comes from the sender's own token, never from the body text, so trust it over any identity a body claims for itself.

  <message from="X">...</message>
      An ordinary heads-up. No reply is owed; read it and carry on.
  <message from="X" id="m-7f3k" type="request" reply-by="10m">...</message>
      The sender is waiting on an answer. Do the work, then reply with `{tool} mail send --to X --reply-to m-7f3k --body <text>` — that exact id.
      reply-by is the sender's deadline, not yours: if you miss it the sender is notified, nothing is sent to you, and the request does not expire on your side. Reply even when late — a refusal is a reply, silence is not.
  <message from="Y" in-reply-to="m-7f3k">...</message>
      An answer to a request you sent.
  <notice>...</notice>
      From the broker daemon itself, never a teammate: its body opens with an [engram] marker and the envelope carries no from — that absence is the tell. There is nobody on the other end, so do not reply; take the information and decide what to do.

Only use --reply-to when the message you are answering actually carried type="request" and an id.

Run `{tool} help mail send` for the sending flags, and `{tool} help mail` for who you send as and what the exit codes mean.
<!-- engram:help agent.head -->
{tool} agent — the agents on this team: who exists, and starting or re-arranging them.

  {tool} agent list
      Every agent, running or asleep. One JSON object: agents[] with id, name, state (live|sleeping), cwd, parent.
<!-- engram:help agent.xref -->
      Names are how you address teammates in `{tool} mail send --to <name>`.
<!-- engram:help agent.tail -->
{tool} agent spawn <name>
      Start an agent that already exists (it keeps its own past session when it has one).
  {tool} agent spawn --cwd <path> [--name <name>]
      Create a new agent in that folder and start it right away.
  {tool} agent new --cwd <path> [--name <name>]
      Create a new agent without starting it. It shows up as sleeping.
      --cwd <path>           the folder the agent works in (required)
      --name <name>          what to call it; without this the folder name is used
  {tool} agent rename <name> <new-name>
      Rename an agent. `outcome` says what happened: renamed (with the name it actually got — a
      number is appended when that name is taken) or unchanged (it already held that name).
  {tool} agent move <name> --parent <name|none>
      Put an agent under another one, or `--parent none` to move it back to the top level.
      --parent <name|none>   the new parent, or the word none to detach (required)

Agents are named exactly: no case-folding, no prefixes. If two agents share a name the command is
refused rather than guessing — pass the id from `{tool} agent list` instead. An agent literally
called `none` can only be used as a parent by its id.

Exit codes: 0 = done | 1 = refused, or the daemon could not be reached — stdout carries the daemon's own reply when there was one, and {"status":"error","code":...,"hint":...} when this CLI refused the call itself | 2 = the daemon answered 2xx in a shape this CLI cannot read; report it, retrying will not help. Judge the outcome by the exit code, not by the shape of stdout.
