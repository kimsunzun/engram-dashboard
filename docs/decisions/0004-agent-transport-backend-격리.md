# ADR-0004: AgentTransport seam + isolating backend knowledge

- 상태: 확정 (S10)
- 관련: CLAUDE.md 「백엔드 확장」 · `transport/mod.rs` · `backend/{mod,claude}.rs` · `types.rs::CommandSpec`

## 맥락
Multiple backends (claude/codex console + API) must be unified behind one interface. If per-backend argument assembly (claude `--session-id`/`--resume`) and the transport method (PTY/HTTP) leak into the core, coupling results.

## 결정
- **Transport = the `dyn AgentTransport` seam** — start/send_input/resize/interrupt/shutdown/capabilities. PtyTransport (shared by consoles) and ApiTransport (API) plug in through the same trait.
- **Command assembly = `backend/`** — it produces only `CommandSpec{program,args,env,cwd}`. The transport does not know which backend it is. Claude-specific knowledge lives in one place only, `backend/claude.rs`.

## 거부한 대안
- **The manager assembles claude arguments directly** — it becomes coupled to claude, and every new backend means editing the manager. Isolation fails.
- **The transport knows the backend** — claude/codex branches leak into PtyTransport, and swappability breaks.

## 근거
The "swappable" master principle (§0). It was confirmed at the S10 fable gate that ApiTransport plugs in through the same trait.

## 영향 / 불변식
- The manager calls only the `backend::needs_session()`/`build_command_spec()`/`backend_for` dispatch.
- For codex/gemini, after a CLI spike, add an `AgentCommand` variant and wire up the `backend_for` routing (currently a stub, not wired).
