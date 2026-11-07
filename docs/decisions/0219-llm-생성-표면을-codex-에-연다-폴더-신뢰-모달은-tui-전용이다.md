# ADR-0219: Open the LLM creation surface to codex — the folder-trust modal is TUI-only

- 상태: 확정 (2026-09-22, basis: codex-cli 0.155.1 measured — see 「근거」 below) · 부분 폐기 by ADR-0270 (the shell gate_backend door of decision 2 and the shell rejection-arm guard in the consequences) · 부분 폐기 by ADR-0279 (the claude-pinned door axis of decision 5)
- 관련: Amends ADR-0215 (the clause blocking codex on the LLM creation surface) · ADR-0187 (app-server confirmed as the codex path of its two) · ADR-0194 (terminal vs JSON is a backend-neutral axis) · ADR-0078 (the output mode is fixed at creation time) · ADR-0155 (declarations sit next to the producer) · CLAUDE.md 「LLM-우선 제어」 · `crates/engram-dashboard-agent/src/commands.rs`(`LLM_BACKEND_POLICY`) · `src-tauri/src/layout/apply.rs`(`gate_backend`) · `src/commands/registry.ts` · `docs/process/S21-codex-backend/trd.md` §6-G · step-log S21 · Amended by ADR-0270 (the shell gate_backend door of decision 2 and the shell rejection-arm guard in the consequences) · Amended by ADR-0279 (the claude-pinned door axis of decision 5)

## 맥락

On 2026-09-07 the user narrowed the Phase 1 scope to 「그냥 딱 codex 띄우는 정도만」 (just launching codex, nothing more) and chose **§6-G = 「안 연다」** (do not open) — a decision that the LLM control surfaces (`agent.new` · the shell `agent.spawnInto` · the frontend registry) are left **unable to create** codex. The fact that held up that decision was one: ★codex shows its own **folder-trust confirmation modal** in a folder it has not seen before, and a caller that is not a human cannot get past that modal★. So opening it now would produce 「만들 수는 있는데 쓸 수는 없는 에이전트」 (an agent that can be created but cannot be used) — the time to open was postponed to 「신뢰 창 처리가 정해질 때」 (when the trust-window handling is settled).

After that, Phase 2 confirmed `app-server` as the path (ADR-0187), and the path that launches codex through the app-server actually started to run. ★**That made it possible to measure the premise above directly, and when measured, that path had no modal.**★ The premise died, so the rejection standing on it loses its basis too.

And **real confusion occurred while the rejection was alive** — the description of `agent.spawnInto` in the command list was advertising 「`"codex"` 는 아는 낱말이지만 이 표면으로는 지금 만들지 않는다」 (codex is a known word but is not created through this surface for now), and a caller that read it judged that codex could not be created through this door and worked around it.

## 결정

1. ★**Open the backend vocabulary of `agent.new` to `Codex` — both output modes.**★ The codex line of `LLM_BACKEND_POLICY` becomes `refusal: None`, the `Codex` variant stands in the declaration vocabulary `AgentBackend`, and `backend_command` fills in its pair. `output_format` is carried regardless of the backend, so **opening one word opens `Terminal` and `StreamJson` together** (the axis of ADR-0078/0194 as is).
2. **The other two doors that look at the policy table are opened together on the same day** — the shell `gate_backend` (`agent.spawnInto`) and the `humanOnly` reason of the frontend registry. The home of the policy is still the single `LLM_BACKEND_POLICY`, and no door carries its own list.
3. ★**No code that checks trust is placed.**★ There is no gate to check on this path in the first place, so building a check that pretends there is one only adds dependency while guarding nothing.
4. **The argument surface is not increased** — `extra_args` is left empty. We do not put in a field for choosing the model, reasoning effort and the like this time (the user limited the scope to spawn).
5. **Two doors are deliberately left pinned to claude** — `create_and_start` (core) and the frontend `agent.spawn`. This is to gather the place where the backend and mode are chosen into **one entrance**. These two have no field that accepts the word in the first place, so they are on a different axis from 「골라도 정책이 닫는다」 (even if chosen, the policy closes it), and if the two axes are mashed together, what has to be touched when opening some door next time is lost.

## 거부한 대안

- **(a) Read codex's `config.toml` before spawn to check the trust state** — dropped. ① TOML direct dependency grows in the core crate ② the **path-key matching rules** of that file (case · separators · UNC · variable expansion) have to be measured separately before the decision stands ③ the failure mode it pays to block **does not exist on this path, as the measurement result shows**. It is a gate that guards nothing.
- **(b) Leave the rejection as it is** — dropped. It blocks a feature the user requested, and **the reason that rejection put forward is no longer true**. If it is kept closed on a reason that is not true, callers believe that advertisement and work around it (it actually happened — 「맥락」 above).
- **(c) Open the model and reasoning-effort argument surface in the same round** — dropped. ★The user explicitly set this round's scope to **spawn only**★. Deciding the argument vocabulary is a separate decision, and squeezing it in here changes the contract shape twice.

## 근거

★**Measurement = codex-cli 0.155.1, 2026-09-22.**★ We sent `thread/start` through the `app-server` path to a **folder never seen before**.

- **It came back as success** — about **338 ms**, **no** error response, **no** inbound trust request.
- And codex **wrote that folder into its own `config.toml` as `trust_level = "trusted"` by itself**.
- That is, ★**the folder-trust modal was a TUI-only device**★ — in that mode the modal is **visible on the screen and a human answers it**. The app-server path does not have that device.

The decision of 2026-09-07 was made without this measurement, reading the modal seen in the TUI as a property of codex as a whole. That the reading was wrong is the measurement above, and so it is **not the decision itself but the premise of the decision that died**.

## 영향 / 불변식

- ★**TUI mode is opened together — there the trust modal is real.**★ It is the cost we accept: that modal is **visible on the screen and a human can answer it**. That is the reason it does not become 「LLM 이 만들었는데 아무도 못 쓰는 에이전트」 (an agent the LLM created that nobody can use), and this sentence replaces the old rejection reason.
- **The policy table is not emptied** — today this table closes no word, but when the next backend comes, 「선언 없음 = 닫힘」 (no declaration = closed) (`NO_POLICY_DECLARED`) must still stand, and this is the place that keeps the three creation doors from each carrying their own list.
- **The vocabulary and the table must open the same set** — three places measure it: `commands::tests::new_creates_exactly_what_the_llm_backend_policy_opens` of `agent` · `src-tauri/tests/layout_apply.rs::every_creation_door_reads_one_backend_policy` · `src/commands/agentCommands.test.ts`.
- ★**The policy arm of `agent.new` is not reached today — still, do not delete it.**★ The **reason it is not reached has changed with this decision**: before it was 「선언 어휘가 표보다 좁아서」 (because the declaration vocabulary is narrower than the table), and now it is 「어휘와 표가 여는 집합이 정확히 같아서 + 표가 아무 낱말도 안 닫아서」 (because the vocabulary and the table open exactly the same set + the table closes no word). The edit that arm blocks is **widening the vocabulary without widening the table**, and on that day the arm comes alive.
- **The rejection arm of the shell `gate_backend` is not reached at execution today** — the wire vocabulary is only `claude` and `codex` and both are opened, so the remaining rejection is the typo net of `parse_backend`. That arm is left as **a guard for the next backend** (the comment at that spot carries that fact).
- **This is not a disk-compatibility migration** — `AgentCommand::Codex { extra_args, output_format }` existed from before this decision and the human menu was already writing it into `agents.json` in exactly that shape. ★Whether the next word has the same worth is decided by **whether a counterpart already exists**★ — `Gemini` has no counterpart, so a disk migration has to be designed along with it.
- **Raise the command-list generation** — both the shell catalog (`layout::commands`) and the core catalog (`agent::commands`). If not raised, the build that creates codex and the build that rejects it report the same generation, and the diagnosis lies.
- **What remains unverified** — this measurement is one combination: **a folder never seen before · the app-server path · 0.155.1**. If codex in another version brings a trust request back into that path, the premise of this decision dies again. The coordinate then is alternative (a) above, and to revive it, **measuring the path-key matching rules** is a prerequisite.
