# ADR-0192: The default of backend behavior is matched to claude — if it cannot be matched, report it

- 상태: 확정 (2026-09-10, basis: user decision)
- 관련: ADR-0188 (permission/approval neutral axis — value design is follow-up) · ADR-0187 (the 2a exemption condition) · ADR-0004 (backend knowledge isolation) · ADR-0005 (finalize once) · ADR-0019 (reaper single consumer) · `.claude/handoff/attachments/codex-measurements-2026-09-09.md` (measured values · combinations) · CLAUDE.md 「아키텍처 원칙」 · step-log S21

## 맥락

While attaching the second backend, questions of the form 「codex 는 어떻게 할까요」 (what should we do for codex) repeated axis after axis (permissions · reconnect after death).

★**The user's point cut that repetition**★ — for such axes, it is not something to decide anew per backend: **matching existing behavior is the default**, and if it cannot be matched, that is **a sign it was not abstracted**.

In fact ADR-0188 had already found the same defect on the permission axis — the two backends each hard-coded their policy as strings in their own files and **they differed from each other**.

## 결정

**The default of a behavior axis that can differ between backends is matched to exactly what claude does today.**

- **If it cannot be matched or has no corresponding value, do not decide arbitrarily but report that fact** — treat it as a sign of an abstraction defect.
- The two values this rule produced this time:
  - **Permission = allow writes + do not ask for approval.** The same posture as claude running today with approvals turned off. ★But matching literally the same would make it a value that even lifts the sandbox on the codex side, which is excessive, so **we matched it one notch narrower, to writing in the working folder**★ (the measurement that the only pair with a correspondence by help text is the 「전부 무시」 (ignore everything) family = `.claude/handoff/attachments/codex-measurements-2026-09-09.md` §2-4).
  - **Reconnect after the process dies = do not.** claude does not either.
- ★**The permission value is provisional**★ — it has to go to the shared declaration form (the declaration is backend-neutral and only the internal execution is moved into each backend's own arguments), and **we leave room for that**. The value design itself is the share of the follow-up ADR of ADR-0188 (user decision).

## 거부한 대안

- **Each backend picks the value natural for its tool** — within one product the policy splits per agent and **the user has no way to know it from the screen** (the measurement in the ADR-0188 맥락 = the two backends' policies already differed).
- **Ask the user per axis** — the questions double every time a backend is added. **This ADR itself came into being to end that repetition.**
- **Attach reconnect only to codex** — it would have to touch 「끝났다는 신호는 정확히 한 번」 (ADR-0005) and 「끝난 세션을 걷어가는 단일 소비자」 (ADR-0019), and then the behavior would split from claude. Each of the two reviewers pointed at this conflict at BLOCK level.

## 근거

- **Precedent = ADR-0188.** The result that came from not having this rule on the permission axis remains as a measurement in the 맥락 of that ADR — the two backends hard-coded their policies as strings on their own and they differed from each other, and the frontend had 0 lines of permission concept.
- **The measured ground of the provisional permission value** = `.claude/handoff/attachments/codex-measurements-2026-09-09.md` §2-4. There is **no ground in the help text** for pairing value names, and the only thing a correspondence stands on is the one pair of the 「전부 무시」 family. So we matched not 「글자 그대로 동일」 (literally the same) but a **value one notch narrower**.

## 영향 / 불변식

- **No reconnect** → the exit and reaping path at the moment the output is cut stays as it is. It does not touch ADR-0005 · ADR-0019.
- **The provisional permission value does not conflict with the exemption of ADR-0187 — but do not read it as 「요청이 아예 안 생긴다」 (requests are not generated at all)** — what the approval policy turns off is **only the approval subset** of the 11 declared server→client request kinds. The other five (`currentTime/read` · `attestation/generate` · `account/chatgptAuthTokens/refresh` · `item/tool/call` · `mcpServer/elicitation/request`) are not approvals but are outside that policy's reach (`.claude/handoff/attachments/codex-measurements-2026-09-09.md:61`). The measured **0** was also not a value produced by 「안 묻기」 (not asking) alone but a result produced together by **read-only + not asking + no tool executed** (the same document `:55` writes the first two as conditions, and §3 `:128` writes no tool execution as the reason for not measuring). ★**Do not write MCP as 「없어서 안 왔다」 (did not come because it was absent)**★ — MCP was alive in that run (the `mcpServer/startupStatus/updated` notification actually arrived, the same document `:53`). The reason `mcpServer/elicitation/request` did not come is **unmeasured**. ★So we do not lean on 「안 온다」 (it does not come) — the disposition that if one comes we answer with method-not-found (ADR-0189) remains valid as is.★ ★But the combination 「쓰기 허용 + 안 묻기」 (allow writes + do not ask) itself is **unmeasured**★ — the no-approval combination that was measured is on the **read-only** side (the same measurement document §1-3).
- **Zero change in claude-side behavior.**
- **Triggers to reopen this decision** — ① when the value design of ADR-0188 comes out, the provisional permission value is absorbed into that axis ② if an axis appears where 「claude 와 동일」 (the same as claude) is **impossible**, report it then and take a separate decision ③ if approvals are actually to be received, the 「서버 요청 0 건」 (0 server requests) exemption of ADR-0187 dies.
