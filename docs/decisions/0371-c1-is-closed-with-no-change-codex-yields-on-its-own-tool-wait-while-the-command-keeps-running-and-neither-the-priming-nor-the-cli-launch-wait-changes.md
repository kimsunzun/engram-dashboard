# ADR-0371: C1 is closed with no change: codex yields on its own tool wait while the command keeps running, and neither the priming nor the CLI launch wait changes

- 상태: 확정 (2026-11-16, 근거: user decision 2026-11-16 「ㅇㅇ 이대로 닫아」 · `docs/research/codex-tool-yield-limits-2026-11-16.md` (source at codex `rust-v0.167.0` + S25 step 8 QA L8))
- 관련: ADR-0360 (launch budgets) · ADR-0369 (the derived launch-wait chain, CLI launch wait 94 s) · ADR-0327 (the built-in priming is the one editable source) · TRD S25 `trd-daemon-wiring.md` §8 C1 · §10 K5 · parent TRD `trd.md` step 10a

## 맥락
A codex agent launches another agent with `engram agent spawn …`, which waits up to 94 s (the CLI launch wait of the ADR-0360 / ADR-0369 chain). The step-8 TRD held a conditional item: if codex's own shell tool gives up on a call before 94 s, a codex agent's spawn call is cut by its own tool while the launch goes on, and it may report a failure for an agent that does start (C1, risk K5; measured by L8). The step-8 QA measurement and a read of the codex source answered it: codex's tools do return before 94 s, but returning is a yield, not a failure.

## 결정
No change. No priming line and no CLI launch-wait change (and no codex config). Codex yields after the time its own tool call chose, the command keeps running, and the model polls for the result. Waiting or polling is the backend's own tool behaviour.

## 거부한 대안
- **(1) A priming line telling the agent a launch can take up to 94 s and to run it in the background** (step-8 TRD C1 option (1), main's own first recommendation — it touches only `builtin\instructions\priming.md`) — 사용자 직접 기각: 「구지 지시문에 하드코딩 해야됨? 공용룰같은 느낌인데?」, then 「ㅇㅇ 이대로 닫아」 (2026-11-16).
- **(3) Shortening the CLI launch wait** (step-8 TRD C1 option (3)) — TODO(main): the research note gives no reason for rejecting this option; the step-8 TRD §8 C1 lists its cost as "breaks ADR-0360 d1".

## 근거
`docs/research/codex-tool-yield-limits-2026-11-16.md`:
- Measured (S25 step 8 QA L8, codex-cli 0.167.0, JSON mode, real kits): a 100 s sleep through the JS `exec` tool with `yield_time_ms: 10000` returned after 10.0 s with no output and a live cell id; the command kept running; the model polled with `wait` and received `done` at about 102.6 s. Nothing was cut.
- Source (`rust-v0.167.0`): the shell tool `exec_command` is capped at 30 s per call (hard-coded, no config); the JS `exec` / `wait` tools have no cap by default and use the value the model writes per call; a config default does not override a value the model writes.
- So a codex call never waits the full 94 s in one go, and yielding does not cut the command: a later poll returns the result, and the spawned agent comes up either way.

## 영향 / 불변식
- Step 10a, which edits the priming (the R5 sentence), must not add the rejected launch-wait line.
- The CLI launch wait stays the derived 94 s (ADR-0369).
- Revisit only if a codex agent is seen misreporting a spawn it did not poll for; per the research note the remedy would then be on Engram's side or in the user's shared agent rules, not in the built-in priming.
