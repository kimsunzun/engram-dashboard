# Where agent name → id resolution happens — in the relay (daemon) vs the handler (shell) asking back — 2026-10-27

- **Status:** light summary · decision pending (user) — the name-resolution path of S28 T-65.
- **Method:** `/research` light · subclass engram · 1 external collector + 1 investigator of the pros and cons in our code · main grounding (Fowler · Kubernetes originals checked). No adversarial review (light).
- **Two options:**
  - **(A) Ask back (current implementation)** — while handling a command, the shell asks the daemon `agent.resolve` once and applies the id it gets back. The daemon does not open the shell command's arguments.
  - **(B) Relay conversion (user proposal)** — when the shell registers a command, it marks 「이 인자는 에이전트 이름」 (this argument is an agent name), and before passing it on the daemon mechanically replaces only that argument with the id. The shell receives only ids.

## External precedents

| # | Claim | Source | grounding | Confidence |
|---|---|---|---|---|
| 1 | 「smart endpoints and dumb pipes」 — do not put logic in the relay (pipe) (the A side). The same article tells you to make finely split remote calls coarser — 「Remote calls are more expensive than in-process calls, and thus remote APIs need to be coarser-grained」 (against A's extra round trip). | [Fowler, Microservices](https://martinfowler.com/articles/microservices.html) | supported (main checked the original) | likely |
| 2 | In Kubernetes the API server mutates (mutating admission) before storing, so controllers receive a normalized object (the B side). Mutating-webhook recommendations = be idempotent · avoid side effects · do not rely on call order · avoid dependency cycles. | [Kubernetes admission webhook good practices](https://kubernetes.io/docs/concepts/cluster-administration/admission-webhooks-good-practices/) | recommendation wording supported (main checked) · 「B 가 낫다」 (B is better) is an interpretation | likely (fact) · inference (application) |
| 3 | An API gateway · BFF unwraps the auth context at the boundary, injects the user id, and downstream trusts only that (circumstantial for B — it is the auth context, not name resolution, so it is not a close match). | [Zuplo — API gateway patterns](https://zuplo.com/learning-center/api-gateway-patterns) (search summary) | collector summary | uncertain |
| 4 | The router of GraphQL federation resolves entity references and sends them downstream (downstream does not ask back). | collector's prior knowledge · source unconfirmed | — | uncertain |

**Trend (inference):** resolving once at the boundary and sending only ids downstream (B) is the more conventional shape — there is no circular dependency (daemon → shell → daemon) and no extra round trip, and the resolution rule sits in one place, with the registry's owner. But the 「dumb pipe」 principle is violated less only if **the relay does not know what the argument means** — that is, when it substitutes based on a generic marker (「에이전트 이름 인자」 (agent name argument)) alone.

## Pros and cons in our code (measured in code)

- **It collides head-on with existing decisions** — ADR-0081 (「데몬은 payload 를 파싱하지 않는다」 · 「UI payload 를 파싱하지 않는다」) · ADR-0156 (「데몬은 저장·중계만 하고 파싱·검증·분기를 하지 않는다」 · 「데몬 불투명 유지 = 하드 제약」). (B) would have to amend those clauses with a new ADR.
- **The relay point is one place** — `command_delivery::deliver` (CLI · MCP · WS all pass through it). There the arguments are JSON values. No bypass path (the human UI uses invoke with ids).
- **There is currently no way to carry the marker** — the declaration macro accepts only `#[doc]` on a field and does not put it in the schema. The daemon holds the schema only as an opaque string. Needed = a new argument type (e.g. `AgentRef`) + an 「에이전트 인자 이름 목록」 (agent argument name list) field in the registration packet + bindings · goldens · a version bump.
- **Old-daemon risk** — with a new shell + an old daemon the name comes through as is. Unless the shell checks 「이 칸은 반드시 UUID」 (this field must be a UUID) and blocks it, the name is silently written as the agent id — (B) also needs a UUID check on the shell side (on failure 「데몬을 다시 띄우라」 (restart the daemon)).
- **Registry reads can block for a long time** — a doc (`manager.rs`) says path normalization of a sleeping profile can take tens of seconds → `spawn_blocking` is needed in the relay path.
- **Error wording moves to the daemon** — the daemon attaches argument-name headers such as `tab_of 'x': … candidates` (a little semantic coupling arises).
- **Scale (estimate)** — 8~10 files · additions ≈ 500 · deletions ≈ 900 (mostly shell tests). The shell's ask-back · deadline · wait-slot cleanup code goes away (the cleanup code itself is generic hygiene, so it may stay). The `agent.resolve` command may stay as a CLI convenience.
- **TOCTOU** — in both options, name → id is outside the shell's apply lock and id → field is inside. (B) is just one hop wider; no real difference (inference).

## Gaps

- We did not find an exact precedent for 「중계자가 하위 명령의 특정 인자를 치환해 넘긴다」 (a relay substitutes a specific argument of a downstream command before passing it on) (e.g., Envoy filter argument substitution).
- tmux · GraphQL federation primary docs not checked.
