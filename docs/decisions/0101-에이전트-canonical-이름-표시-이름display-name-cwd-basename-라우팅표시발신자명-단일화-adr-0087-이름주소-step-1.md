# ADR-0101: An agent's canonical name = its display name (display_name ?? cwd basename) — unifying routing · display · sender name (ADR-0087 name addressing, step 1)

- 상태: 확정 (2026-07-23, basis: user decision + a real-code scope survey)
- 관련: ADR-0087(send-message semantics · name rules — this ADR is step 1 that makes concrete its "human-readable name address") · ADR-0061(display_name tree rename) · `crates/engram-dashboard-core/src/agent/manager.rs::agent_info` · `crates/engram-dashboard-daemon/src/control/ingress.rs::resolve_recipient`·`sender_display_name` · `src/util/basename.ts`

## 맥락
When an agent sends a message to another agent (`send_message`/`engram-send`), the recipient address did not work with the name shown in the tree (a real-use observation: when ACB sent to "DEF" it got "no live agent", and the roster had 2 names that were cwd paths). The cause = **the "name you see" and the "routing name" are different fields**:
- Routing (`resolve_recipient`) finds by exact match on `AgentInfo.name`, and this value is computed in `agent_info` as `profile.name` (`profile.name` is the CreateProfile name or, for an ad-hoc spawn, the cwd string — not a "clean display name").
- The frontend tree ignores `profile.name` and draws `display_name ?? basename(cwd)` (`AgentList.tsx` `displayNameOf`).

So the name a person saw and the name the daemon accepts as an address had diverged, and two agents started in the same cwd both had the cwd path as their routing name, so they could not even be uniquely pointed at.

## 결정
Unify on **one canonical name = the display name (`display_name` if present, otherwise the `cwd` basename)**, and **routing · roster · tree · envelope sender name all share** this value.
- The backend `agent_info` computes `AgentInfo.name` as `display_name ?? cwd_basename(cwd)` instead of `profile.name` (porting the same rule as the frontend `basename.ts` to Rust — including the drive root · UNC · empty path fallbacks).
- `resolve_recipient` (= `a.name` matching) · the error roster · `DeliveryObservation.to_name` automatically follow this new value (no code change).
- `sender_display_name` (the envelope sender name) is also aligned to the same display-name computation — because it is consistent only if the sender name the recipient sees is also a human name (a sub-decision, the main agent's judgment · reported).
- **The id is kept as a fallback** (`resolve_recipient` tries an exact id match before the name — blocking hijacking by a name that pretends to be someone else's UUID, ADR-0087).
- `profile.name` is demoted to a spawn seed/persisted field (it steps back from being the routing key). The schema · protocol (`AgentInfo` format) · frontend code are unchanged.
- **Out of scope this time (follow-up = "②"):** enforcing uniqueness of same names (auto suffix `-2`/`-3` · no reusing a number while running · binding the name to the AgentId to keep the epoch). The part ADR-0087 already left as "unimplemented" — this ADR first handles only "make the address work with the display name" out of that.

## 거부한 대안
- **Addressing by agent id (UUID) only** — dropped. A person/LLM cannot use the roster directly and must look up the id first (friction). ADR-0087 also set a human-name address as the intent, noting "UUID addresses adopted in 0 OSS projects". The id is not removed and is kept only as a disambiguation fallback.
- **Keep `profile.name` as the routing key + teach senders to "address by cwd path"** — dropped. It differs from the name a person sees, so it violates intuition, and two agents in the same cwd have the same routing name, so uniquely pointing at one is impossible.
- **Fix only the frontend (align the display to profile.name)** — dropped. Routing is in the backend, so fixing only the frontend does not make "visible name = address" hold. The right direction is the reverse: align the backend canonical to the display name.

## 근거
- User decision (2026-07-23): address = the name a person sees (display), ① (display-name address) first · ② (the uniqueness machinery) later.
- Scope survey (real code): the change is local to the single computation point `agent_info` + a basename helper (S). Routing · roster · tree converge automatically to follow that value, and the protocol · frontend · existing tests are confirmed unchanged (the matching logic is invariant, only the value changes).

## 영향 / 불변식
- **WYSIWYA invariant:** the agent name string shown in the roster/tree = the string that the `to` of `send_message`/`engram-send` matches. If the two diverge again (e.g. the frontend regresses to its own computation, or routing returns to `profile.name`), this bug recurs. Single source = the canonical name computation in `agent_info`.
- **Keep id-first matching:** `resolve_recipient` tries an exact id match before the name (blocking impersonation, ADR-0087) — do not reverse this order.
- **Uniqueness not guaranteed (current scope):** if two live agents have the same name, `RECIPIENT_AMBIGUOUS` still occurs (until ② resolves it with an auto suffix). This is an intended remainder — not silently delivering to an arbitrary one is the safe side.
- **Verification gate:** the completion criterion is a GUI/real-behavior measurement — start two agents with different display names and `send_message` by the name shown in the tree → check that it is actually delivered (code tests alone are incomplete).
