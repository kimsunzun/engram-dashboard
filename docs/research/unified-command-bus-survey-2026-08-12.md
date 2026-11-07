# Unified command bus — precedent survey (design-decision mode)

| Item | Value |
|---|---|
| Status | **adversarial review complete · verdict BLOCK · edition with corrections applied** ★Review correction★ (first edition's label = "draft — before grounding and adversarial review") |
| tier | deep (5 main-family tracks + 1 parallel cross-family collection on the peer track) + 1 cross-family adversarial review |
| Method | collectors spawned in parallel per track (12–18 web searches + primary-source fetches) → main aggregation → cross-family adversarial review |
| Date | 2026-08-12 |
| Incomplete | the open items raised by the review (§8 higher-ranked alternatives · §9 missing contract clauses) are unresolved · exhaustive grounding not done |
| Confidence legend | ★Review correction★ The tags are no longer a collector self-report (first-edition legend = "collector self-report, the final tag derived from grounding + review"). **A claim that passed the review keeps its original tag (certain / likely / uncertain) · a refuted claim is marked `반증` (refuted) · a claim downgraded for lack of sample is marked `표본 한정` (sample-limited).** |

★Review correction★ **Until the open items of §8 and §9 are closed, this report cannot be used as grounds for rejecting an alternative in an ADR.** The verdict is BLOCK, and because the classification of the closest precedents (Zellij · OBS · LSP) and the most heavily used "no precedent" claim were refuted, the grounds for the §4 fit table and the §5 draft of rejected alternatives do not stand yet.

## 0. Our constraints (axes for the fit decision)

| # | Constraint | Basis |
|---|---|---|
| C1 | **The daemon does not know the client shell** — isolation is the invariant | ADR-series decisions · code measurement |
| C2 | The cost of introducing a new crate dependency or a new external toolchain is high (isolation gate · dependency cap) | the CLAUDE.md gate list |
| C3 | There is already a derivation path we trust (Rust types → 23 TS generated outputs + a CI drift gate) | measured |
| C4 | An LLM caller is a first-class consumer — it joins under the same contract as a human click | 「LLM-우선 제어」 (LLM-first control) |
| C5 | Do not tie code to the transport (swappable) | 「아키텍처 원칙」 (Architecture principles) |
| C6 | The executing functions are scattered across three processes and cannot be shared — only the contract is shared | measured |
| C7 | A list filter is a means of education, not a means of enforcement | existing decision (mail isolation) |

## 1. Four strategies on which the precedents split

The collection found no single dominant pattern, and it split into four groups.

| Strategy | Representatives | Characteristics | Fit for us |
|---|---|---|---|
| **A. Self-describing bus + runtime lookup** | D-Bus (Introspect), gRPC reflection, Kubernetes discovery | a living process spits out its own commands and arguments as XML/schema | partial — the best as a lookup model, but the aggregator problem (§4-③) |
| **B. Capability negotiation + runtime dynamic register/unregister** ★Review correction★ (first edition's name = "capability negotiation at session start + static list") | LSP, DAP, WebDriver BiDi | the spec statically defines commands, arguments and results, and "what is supported now" changes through the initialization declaration **+ runtime register/unregister** | partial — `반증` The first edition's "declared once in `initialize`, then the list is fixed" is wrong (LSP `client/registerCapability`·`client/unregisterCapability`). "Argument schemas do not enter the negotiation" is also true only for the arguments of LSP user-defined commands. Sources (given by the review): <https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.18/specification.md> · <https://github.com/microsoft/vscode-languageserver-node> |
| **C. Daemon + thin client, a single typed vocabulary** | **WezTerm**, Zellij (partial precedent), tmux, sway/Hyprland, Docker/K8s | one command enum feeds several input paths — but there is no guarantee that this enum covers *every* surface ★Review correction★ (first edition = "feeds keybindings, CLI and plugins all") | **partly best** — closest to our shape, but we did not secure a precedent for a "sole contract" (§1 peer section) ★Review correction★ (first edition = "best — isomorphic to our shape") |
| **D. In-process registry, unreachable from outside** | **VS Code** ★Review correction★ The first edition also put OBS here, but `반증` — see the separate item below | there is a list and lookup works, but an external caller cannot reach it | **our current state = not the goal (cautionary example)** |

### The two closest peers — 1 downgraded to a partial precedent

- **Zellij — a partial precedent.** `반증` ★Review correction★ The first edition wrote "a single `Action` enum is the only command vocabulary · keybindings, CLI and plugins all converge on that enum · **a real precedent for our plan**". In fact, in the source `Action` is the set of **actions that can be bound to keys**, and the plugin API has a large **separate function surface** such as `open_file`·`switch_session`·`cli_pipe_output`·`reconfigure` (`run_action` is just one of them). The top-level CLI too has commands outside `Action`, such as `list-sessions`·`kill-all-sessions`·`setup`. → **It is a precedent of reusing the enum on some action paths, not a precedent of a sole contract for GUI, CLI and plugins.** What remains valid: the CLI parses arguments, serializes them into a client→server message (protobuf, length prefix) and passes it to the server process.
  Sources (given by the review): <https://github.com/zellij-org/zellij/blob/main/zellij-utils/src/input/actions.rs> · <https://zellij.dev/documentation/plugin-api-commands> · <https://zellij.dev/documentation/commands>
- **WezTerm** — a 3-way split of mux-server (daemon)/GUI/CLI, with its own Rust PDU codec + request serial numbers to pair responses. The shape is almost the same as our daemon protocol. (likely) Additional confirmation by the review: it uses an explicit correlation structure that connects serial numbers to waiting promises, and it also keeps an **explicit codec version** raised on incompatible changes (§9). <https://github.com/wezterm/wezterm/blob/main/wezterm-client/src/client.rs> · <https://github.com/wezterm/wezterm/blob/main/codec/src/lib.rs>

### ★Review correction★ OBS does not belong to family D (unreachable from outside) — `반증`

The first edition put **VS Code and OBS (single process)** in family D. OBS Studio, from version 28 on, **includes by default a WebSocket control API to which external applications and scripts attach**, and there is also an extension API through which plugins add vendor requests/events. → OBS is not a cautionary example but a **precedent of an externally callable command surface**. Only **VS Code** is left as the example of family D.
Sources (given by the review): <https://obsproject.com/kb/remote-control-guide> · <https://obsproject.com/kb/developer-guide> · <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>

### 2 missing precedents pointed out by the review (review sources — no independent verification by the original collectors)

- **A mobile-OS typed-action framework (Apple App Intents).** When typed actions are **declared** in app code, **the compiler generates discovery metadata**, and system-level assistants, shortcuts and search consume it. The user composes actions from several apps into one workflow. → **The closest precedent secured so far in which GUI, automation and LLM-type callers share one contract.** (given by the review · not verified by this report's collectors)
  <https://developer.apple.com/documentation/appintents> · <https://developer.apple.com/documentation/appintents/creating-your-first-app-intent>
- **An agent-protocol extension (MCP Apps).** A sandboxed view **calls server-side tools through the host**, tools can be exposed to the model, to the app, or to both, and **host context** such as theme, locale and display mode **is passed to the view**. It is directly adjacent to our problem, where a webview + chat/LLM + tool visibility are in one place. However, **it does not automatically solve the problem of handlers owned by the shell.** (given by the review · unverified)
  <https://apps.extensions.modelcontextprotocol.io/api/documents/Overview.html>

### Cautionary example (the Unreal precedent)

**Unreal Engine's command surface split in two and did not converge** — the console command registry and the remote-control HTTP/WS API are **different declaration mechanisms**, and a console command is not automatically exposed remotely. There is also no enumeration path that spans both. A concrete case of "this is what happens if you do not merge." (certain — they are separate systems / likely — absence of a bridge)

## 2. Comparison with our paper design (relay) — what was right and what to look at again

### ★1 item to look at again (a confirmed decision clause)

**"The daemon does not open the envelope" matches the precedents only by half.** ★Correction (cross-family cross-collection refutation)★ — the first edition generalized to "every mature implementation we surveyed", but **relayers split into two kinds**:

- **(a) Transport-layer relay — reads only the outer cover and does not open the inside.** The table below. The side that our paper design assumed.
- **(b) A facade daemon that holds policy — opens up to the body.** A package-management daemon calls the backend **after queueing, authorizing and normalizing** the request (= an inspecting and translating facade, not an opaque pass-through). A desktop-portal front tracks even the caller identity, the request/session objects, permissions and response signals. An image-editor core registry **stores and enforces the registered types, defaults, run modes and return values**. All three are mature implementations and are counterexamples to "does not open" (cross-family collector: certain).

**Our daemon is already close to (b)** — it does credential authorization and the list check itself. So "outer cover only" is not the one right answer, and one more option has appeared (§6 issue A).

The outer-cover/inside boundary of the transport-layer relay (a). ★Review correction★ **"Inside" means the *default routing path* and does not mean that the body is never read under any condition** (see the D-Bus row):

| System | Outer cover (read) | Inside (not opened on the default path) |
|---|---|---|
| D-Bus | PATH/INTERFACE/MEMBER/DESTINATION/SENDER | marshalled body — **not unconditionally** ★Review correction★: a match rule can **inspect body arguments** with `arg0`·`argNpath`·`arg0namespace` and so on, and the reference daemon implements this. The accurate wording = "default routing and ACLs are mainly header-based, but some match features interpret the body" (<https://dbus.freedesktop.org/doc/dbus-specification.html>) |
| AMQP | exchange/routing-key (correlation_id is copied by **the reply worker** — not by the broker, ★Review correction★ see the pairing-number item below) | body |
| NATS | subject string | payload bytes |
| JSON-RPC family (LSP·MCP) | `method`, `id` | `params` |

**There was no precedent that put even the command name inside an opaque envelope.** If the name is on the outer cover, **command-level permission and observability** are possible even when the daemon does not know the arguments (D-Bus policy applies ACLs exactly at the interface and method level), and if it is hidden all of that becomes impossible. → **An item for the user's decision.**

### What was confirmed as well chosen

- **The caller fixes the request pairing number once and keeps it unchanged to the end** — in JSON-RPC/LSP/MCP the response **returns the same `id` that the caller made, as is**, so this is exactly that approach (certain). It matches our choice of not having a separate relay_id.
  - ★Review correction★ **The attribution of the mechanism was wrong.** The first edition lumped together "AMQP `correlation_id`, JSON-RPC/LSP/MCP `id`, Jupyter `msg_id`/`parent_header` all work **in this way**". In fact ㉠ in the message-broker family (AMQP/RabbitMQ RPC) the party that copies `correlationId` is **not the broker but the worker that makes the reply message**, and ㉡ the notebook protocol (Jupyter) **issues a new `msg_id` for each message** and copies **the whole request header into `parent_header`**. → **They share the correlation *principle*, but the wire mechanism and the responsible party differ.** The sentence "the precedents return the caller's number as is" can be applied only to the JSON-RPC family.
    Sources (given by the review): <https://www.jsonrpc.org/specification> · <https://www.rabbitmq.com/tutorials/tutorial-six-kotlin> · <https://jupyter-client.readthedocs.io/en/latest/messaging.html>
- **If there is one connection, global uniqueness is unnecessary** — D-Bus SERIAL requires uniqueness only per connection, because the topology guarantees the return path (certain). The same argument as our case of a single daemon↔shell connection.
- **Do not execute at the place that is waiting** — a classic failure that several families rediscovered independently (actor self-call deadlock, synchronous IPC freezing the whole renderer → always switch to async, formalizing the actor re-entrancy mode as policy). A mechanical grounding on the Rust side also appeared: with a bounded channel, send blocks so a cycle deadlocks, but **a oneshot's send does not block, so it cannot take part in a deadlock cycle** (likely). Our "apply outside the connection task + oneshot correlation" is exactly this prescription.
- **Handling an absent entry after role registration** — it splits three ways and all three exist: an immediate typed failure (no-responders/UnknownMethod), queueing + timeout, on-demand launch. Our "role registration after authentication" model most resembles the shape of D-Bus's name request/owner-change notification.
- **Checking permissions on every call is the standard in the surveyed sample** — `반증` ★Review correction★ The first edition wrote "it is common to check once at registration/negotiation time … and route by session identifier alone". **The systems the report itself cited as representative precedents say the opposite**: a bus daemon checks policy when a message is routed between connections, a cluster API authorizes request attributes **on every request**, and an RPC interceptor is essentially **per-call**. No frequency or sample grounds were given either. → Corrected statement: **a check on every call is the standard of the surveyed systems**, and "check once and then route only by session identifier" is **an optimization observed in some session-oriented systems**, not a common pattern.
  Sources (given by the review): <https://dbus.freedesktop.org/doc/dbus-daemon.1.html> · <https://kubernetes.io/docs/reference/access-authn-authz/authorization/index.html> · <https://grpc.io/docs/guides/interceptors/>

## 3. Grounds by fork

### ① Generation scope — names only vs down to the arguments

**Conclusion: down to the arguments. But the grounds were narrowed** ★Review correction★ — the first edition's ground was "there is no precedent evidence on the other side", but in that form it is not supported. The statement that holds is only **"every surveyed sample that matches our condition (a fixed contract between two processes) generates down to the arguments"**.

- **In the surveyed sample we did not find a mature case that "has a generator and generates only names".** `표본 한정` ★Review correction★ First edition's wording = "0 mature cases". The cases inside the sample (protobuf/gRPC, the LSP metamodel, WebIDL/Blink, tauri-specta) all generate name + arguments + return together (certain). But **because the search scope and inclusion criteria were not given, the global claim of "0 cases" is not supported.** No counterexample was secured either, so it is not a refutation, but **this one sentence alone is insufficient as a reason to reject in an ADR.**
- The places that are names only are all cases **with no generator at all** — VS Code contribution points (only `command`/`title`, no argument field exists in the schema), bare Electron IPC (string channel + `any[]`), bare Tauri `invoke`, Slack slash commands (arguments are free text). The reason is not economy but a **structure where arguments are not fixed at declaration time** (human clicks · free input) (certain + likely).
- We are in the opposite condition (a fixed contract between two processes), so there is no ground inside the sample that supports choosing names only.
- **The cheap path:** instead of introducing a new IDL or toolchain (violates C2), **widen the scope of the derivation family we already trust**. The current tool derives only the data shapes and does not make the commands themselves (name + arguments + call shell), and a sibling tool of the same family (`tauri-specta`, `specta`) covers exactly that scope. However, it is tagged pre-1.0 (rc) so the API may change, and the generation time is "when running a debug build" so **we have to attach a CI diff gate ourselves, as we do now** (likely).
  - ★Review correction★ **"Covers exactly that scope" cannot be used as it is.** ㉠ **The output of that tool directly calls the app framework's invoke path (`__TAURI_INVOKE`)**, so it is not compatible with C5 (do not tie code to the transport) as it is — the C5 `○` in §4 was lowered to `△`. ㉡ The fact that the current deriver is of the same family does not make that sibling tool **stop being a new crate and a new gate** (C2 still applies). → The shape the review gave: **reuse only the command metamodel and generate the transport adapter separately.**
    Sources (given by the review): <https://github.com/specta-rs/tauri-specta/releases> · <https://docs.rs/tauri-specta/latest/tauri_specta/>
- **Warning 1 (drift):** the prose spec of the LSP metamodel says "every enum must allow unknown values", but the metamodel omitted that marker in places, so a strict generator produced **closed enums** and broke real clients (likely — a single reporter). → Whether the generator is **faithful to the extension rules of the contract itself** matters as much as the existence of the generator.
- **Warning 2 (drift gate):** what teams actually trust is a **CI diff/compatibility gate** on the generated output (the same as what we already do). A runtime handshake has a different purpose (tolerating partial implementations by several independent servers) and is not for drift detection.

### ② Is a query also a command

**Open issue — the evidence splits both ways (convergence failed).**

- **The merging side:** LSP·D-Bus·K8s·Docker·sway/Hyprland·WezTerm·Zellij·CDP·OBS·mpv — reads and writes ride the **same envelope/registration mechanism**. The peer track saw "merging is normal" (certain).
- **The splitting side:** GraphQL split query/mutation even in **execution semantics** (mutations run serially), and MCP split reads (Resources) and execution (Tools) into different protocol namespaces (certain). In D-Bus too, property reads are a separate interface, and in sway RUN_COMMAND vs GET_* are separate message types (certain).
- **How the flat-merge side handles it in practice:** VS Code keeps one namespace and **curates palette exposure with a condition clause** — a metadata layer, not a second registry (certain). Redux does not put pure reads in the action list in the first place (selectors are a separate mechanism).
- **Failure cases:** macro recording excludes non-mutating operations as "not recordable" (Photoshop), and high-frequency actions choke the time-travel log so a blacklist is recommended (Redux DevTools) — both are real cases of "reads pollute the execution history" (likely/certain).
- **Is the read/write marker advisory or enforced:** MCP's read-only hint is explicitly a **hint with no guarantee** ("an untrusted server can lie"), and HTTP's safe methods are only descriptive (certain). By contrast, K8s makes the read/write verbs **an enforced permission category** (certain). → Consistent with our C7: the marker is for education, enforcement is the server's.
- **No migration cases:** we did not find a migration story of "merged and then split / split and then merged" in command-palette-type systems (unknown).

### ③ Where screen-only commands are declared · the list aggregator

**★Review correction★ "An open problem unique to our structure — there is no directly portable precedent" was refuted.** `반증` — precedents of a mechanism where a non-central party holds the list really exist (counterexamples (a) and (b) below). What remains is not "there is no precedent" but **"which precedent shape to choose"**.

- In the sample, the party that merges the commands of many owners into one list was in most cases **already structurally central** (bus daemon, permission daemon, cluster aggregation controller) (certain). ★Review correction★ The first edition's "**without exception**" is withdrawn — counterexample (a) below is the exception.
- Without a center, **the calling side asks each server separately and merges them itself** (an MCP client receives the per-server lists, attaches namespaces and merges; there is no inter-server aggregator in the protocol) (certain).
- If C1 (the daemon does not know the shell) holds, the daemon is not qualified to be the aggregator — this part is valid. To this the first edition attached the **independent abstentions** of two tracks (runtime lookup · declaration derivation) and concluded "no precedent of a mechanism where a non-central party merges lists on behalf of others (unknown)". → **`반증`. Withdrawn by the two counterexamples below.** ★Review correction★
- **★Counterexample (a) — a true refutation: a decentralized discovery-type robotics middleware (ROS 2).** **There is no central master**, and any local node **queries all service names and types from the graph it has discovered** (the official API states that it returns all service names and types that the given local node has discovered so far — back-translated). Because it is a mechanism where a non-central party holds the list without any central party, it **directly refutes "no precedent".**
  <https://docs.ros.org/en/ros2_packages/rolling/api/rmw/generated/function_get__service__names__and__types_8h_1abf495afaafacac33473d5f6c9c451ab4.html> · <https://docs.ros.org/en/lyrical/Concepts/Intermediate/About-Domain-ID.html>
- **★Counterexample (b) — a partial counterexample: mobile-OS intent enumeration (Android).** An arbitrary application uses `queryIntentActivities()` to **enumerate the handlers of other apps it did not know in advance** and builds its own selection UI. **However, this does not completely refute "no precedent"** — the package/intent registry **itself is a central OS service** and the app only queries that central registry. What it actually proves is not decentralized aggregation but **caller-side merging and selection on top of a central registry**, a **different mechanism** from (a). (This distinction was not made by the review and was added in this correction — the review lumped (a) and (b) together as equal "exact precedents".)
  <https://developer.android.com/guide/components/intents-filters> · <https://developer.android.com/reference/android/content/pm/PackageManager.html>
- **Partial counterexample (c) — CDP.** The Chrome DevTools Protocol has **some domains implemented by the renderer process and some by the browser process**, yet to the caller it looks like **one protocol surface**, and a live browser serves the whole schema through `/json/protocol` (certain/likely). That is, "handlers scattered across several processes with a single contract + single lookup" really exists. **However, that browser process is a center that knows the renderers, so it does not get around C1.** → A target for precise confirmation in grounding.
- Note: in Neovim **the runtime lookup returns not only names but also function signatures** (certain), and remote plugin hosts (separate processes) **register** their own commands into the core namespace so that, to callers, they are indistinguishable from built-ins (certain). One of the workable shapes of ③ — whether "the shell registers its own commands into the daemon namespace" is possible without breaking C1 is a subject for review.
- **An additional candidate (cross-family collection) — an explicit owner-selection layer.** The desktop portal has **the front hold the public contract while delegating the real implementation to one of several backend processes**, and when several backends claim the same interface, it **picks the first match from an ordered preference list in a config file** (certain). Since it is the only concrete mechanism that touches ③ (declaration location · aggregation) and ④ (multiple owners) at once, it is priority 1 for follow-up review.

### ④ Can one command have several owners

**Conclusion: forcing a single owner + loud failure is the majority but not unanimous. Fan-out is a property of events.** ★Correction★ — the first edition's "mainstream" is weakened by 2 counterexamples (the correction items at the very bottom).

- A command/query has exactly one handler, **events fan out** — several frameworks draw the same line (an exception on duplicate registration, an error if there is no handler). In D-Bus too, a method is unicast (an explicit error if unowned) and a signal is broadcast (zero subscribers is also normal) (certain).
- **3 forms of handling an unregistered name:** ① a protocol error (gRPC UNIMPLEMENTED, JSON-RPC -32601, VS Code "command not found" — but it shows up **only at call time**) ② silent ignoring (Redux — but this is a broadcast structure to begin with) ③ disabled/grayed out (WPF — but this is a known command that is **impossible in context**, not a globally unregistered name). → The one that fits our scenario is ①.
- **Our "fail loudly" is supported.** A stronger option also appeared: **build-time enforcement**. gRPC Go catches it as a compile error if the forward-compatible embed is left out, while Rust tonic has a default implementation so even an empty impl compiles, and **maintainers have left a removal discussion open on the grounds that it kills the compiler signal after a schema change** (certain — the issue and its argument exist, likely — the conclusion is unchecked).
- **"Loud ≠ easy to debug":** there is a real case where VS Code throws on duplicate registration but the error blows up in **the innocent party that registered later**, making the cause hard to trace (likely).
- **A migration case away from silent overwrite:** Blender used to overwrite the same identifier silently (accidents happened), and then moved toward **blocking overwriting of built-in operators altogether** (likely). The same movement as our chosen direction.
- **The real cost of priority shadowing (Unreal precedent):** in Unreal Enhanced Input, several mapping contexts can bind the same action and only the highest priority takes it, and a documented pitfall is that **a leftover context silently intercepts input even after the owning UI is gone** (likely). If we choose a silent shadow, we pay the same price.
- 1 counterexample: a DOM single-slot handler silently lets the last one win (certain) — but the domain is different.
- **★Correction — "no precedent of silent overwrite in the command-dispatch family" was wrong.** The cross-family collection produced a counterexample: the procedure registry of an image editor is a command dispatch on which GUI, batch and plugins all converge, and **re-registering the same name silently overwrites the earlier one even now** (the docs only warn not to do it, certain). That is, a precedent exists, and **that precedent itself does not recommend it** — the argument must change from "no precedent" to "a precedent exists but the party warns against it".
- **★Correction — "forcing" a single owner is not unanimous either.** A robotics middleware's service is an addressed request/response, but **several servers can register under the same name and who receives is undefined by the spec** (certain). The collector itself classified it as a "policy to avoid", but the factual claim that "all precedents enforce it" does not hold.

## 4. Constraint fit table

| Candidate | C1 isolation | C2 new dependency | C3 reuse of existing derivation | C4 LLM first-class | C5 transport swap | C6 contract-only sharing | Overall |
|---|---|---|---|---|---|---|---|
| **Family C: a single typed command vocabulary + daemon relay** (WezTerm type · Zellij is a partial precedent) | △ aggregator undecided | ○ | ○ | △ ★Review correction★ sharing types ≠ an LLM contract (runtime-enumerated name · description · input schema · side-effect metadata are also needed) — first edition `○` | ○ | △ ★Review correction★ the contract is shared but **who owns, is alive and routes right now** is unresolved — first edition `○` | **second choice — pushed down by §8 first place (hybrid)** ★Review correction★ (first edition "main candidate") |
| Widen the derivation scope (sibling tool of the current family) | ○ | △ pre-1.0 **+ it stays a new crate and a new gate** ★Review correction★ | ◎ same family | ○ | △ ★Review correction★ the output directly calls the app framework's invoke path → conflicts with transport swap (first edition `○`) | ○ | **the means of carrying out ① — but in the form of reusing only the metamodel + generating the transport adapter separately** |
| A new IDL toolchain (protobuf/WebIDL type) | ○ | ✕ new toolchain and gate | ✕ | ○ | ○ | ○ | rejection candidate |
| Family A: self-description + runtime Introspect | △ ★Review correction★ a central aggregator is not inevitable — a decentralized discovery-type precedent exists (§3-③ counterexample (a)). First edition `✕ 중앙 취합자 전제` | △ | △ | ◎ | ○ | ○ | borrow the lookup model + **a discovery layer** |
| Family B: capability negotiation + **runtime dynamic register/unregister** ★Review correction★ (first edition "static list") | ○ | ○ | △ | △ ★Review correction★ combining a static protocol schema with runtime capability bits can serve LLM callers, so the `✕` has no basis. The first edition's `✕ 인자 스키마 없음` generalized the looseness of LSP user-defined command arguments to the whole strategy (WebDriver BiDi statically defines each command's method·params·result in the spec: <https://w3c.github.io/webdriver-bidi/>) | ○ | ○ | partial borrowing → the source of the registration layer of §8 first place |
| Family D: keep the in-process registry (VS Code type — OBS excluded ★Review correction★) | ○ | ◎ | ◎ | ✕ unreachable from outside | ○ | ○ | **status quo = falls short of the goal** |

★Review correction★ **The whole table does not stand as decision grounds yet.** The review decided that granting `○` on C4·C6 while admitting the aggregator is undecided pre-empts the conclusion — an LLM caller needs runtime-enumerated names, descriptions, input JSON Schema, (optional) output Schema and side-effect metadata, and agent protocols also require a valid input schema in a tool definition. <https://modelcontextprotocol.io/specification/2025-11-25/server/tools>

## 5. Rejection candidates → draft of the ADR's rejected alternatives

1. **Generate names only** — `표본 한정` ★Review correction★ First edition's reason = "0 mature cases that have a generator". It is lowered to **not found in the surveyed sample**, and **this reason alone is insufficient as grounds for ADR rejection** (§3-①). The remaining valid argument: the names-only cases are only structures where arguments are not fixed at declaration time, and we are in the opposite condition.
2. **Introduce a new IDL or toolchain** — violates C2 (isolation gate · dependency cap). The derivation path we already trust does the same job. ★Review correction★ However, a sibling tool of the same family is also a **new crate and a new gate**, so the C2 argument applies to that candidate too (§3-①).
3. **Promote the daemon to list aggregator** — breaks C1 (this part is valid). ★Review correction★ The "**all precedents are central parties**" that the first edition added is `반증` (§3-③ counterexample (a) = decentralized discovery with no central master). The reason for rejection is narrowed to a single conflict with our constraint (C1).
4. **Use a list filter as a means of enforcement** — violates C7 + many precedents keep the list and the check as separate mechanisms (the permission daemon's separation of enumeration and checking; cluster discovery is the same regardless of permissions).
5. **Add a second control surface to the frontend** — Unreal's two split surfaces are the real cautionary example.
6. **Silent overwrite · silent ignore** — ★Basis replaced★ a precedent exists (the image editor's procedure registry), but **the party's own documentation warns against it**, another mature case **migrated from silent overwrite to blocking** (Blender), and the case that left routing undefined was classified by the collector as a "policy to avoid". The real cost of a silent shadow was also confirmed in Unreal. → The reason for rejection is not "no precedent" but **"a precedent exists and that precedent regrets it"**.

## 6. Open issues · limits · abstentions

- **Usage limit of this report (warning):** ★Review correction★ **do not use as grounds for ADR rejection** until the open items of §8 and §9 are closed (the same as the sentence under the meta header).
- **Issue A (awaiting the user's decision):** how much the relayer reads — it is entangled with the "does not open the envelope" clause of the confirmed ADR. Because the precedents **split into two kinds, there are three options**: ㉠ fully opaque (no precedent) ㉡ only the name + pairing number on the outer cover (the **default path** of a transport-layer relay — ★Review correction★ the first edition's "all transport-layer relays" is an overstatement. A bus daemon can inspect body arguments with a match rule (§2). The advantage that command-level permission and observability open up for free is valid) ㉢ authorize and normalize down to the body (facade daemons that hold policy — the closest to the current nature of our daemon). The cost of reversing is high because ㉡→㉢ is a format change.
- **Issue B (fork ②):** whether to put queries in the same list — the evidence split both ways. The side that split them structurally (GraphQL·MCP) did so because the execution semantics differ, and the side that merged them shares the envelope but curates exposure. **The main model does not pick a winner.**
- **Cross-collection result (limit 1 resolved):** the cross-family peer collection was run BLIND and compared. **Neither side is a superset of the other** — the other family did not see our 2 closest peers (the terminal-multiplexer family) or the whole code-generation·IDL family, and the main family did not see the policy-facade-daemon family, the robotics middleware, the owner-selection layer or the plugin-process registry. **3 corrections came out of this comparison** (the §2 outer-cover generalization · the §3-④ no precedent of silent overwrite · the §3-④ forced single ownership being mainstream). ★Review correction★ **From the mobile-OS inter-app command surface, which the first edition left as a "gap unexplored by both sides", a key counterexample and the closest missing precedent later came out** (§3-③ counterexample (b) · §1 two missing precedents) — it was a priority subject to investigate, not optional. The build/CI and DAW families are still unexplored.
- **Limit 2:** ★Review correction★ The adversarial review **was carried out and its verdict is BLOCK** (first edition = "adversarial review not carried out · confidence is collector self-report"). **Exhaustive** grounding (claim ↔ source implication) is still not done, and of the review's 5 spot checks, 4 came out NOT SUPPORTED and 1 PARTIALLY SUPPORTED.
- **Abstentions (stated by the collectors):** ~~the aggregation mechanism of a non-central party's list~~ → ★Review correction★ **abstention withdrawn — refuted** (§3-③ counterexamples (a)·(b)). Remaining abstentions: the documented pattern of "A declares and B implements" · migration cases between merging and splitting in command-palette-type systems · details of unregistered-name handling in some systems.
- **Unexplored world:** ★Review correction★ The mobile-OS inter-app command surface (Android intents · iOS App Intents) **was filled in by the review** (§1·§3-③ — but it is a review source and unverified by the original collectors). Remaining unexplored: the build/CI family, DAW plugin parameter automation. Whether to add a new group is unchecked.

## 7. Next steps

1. Retrieve the cross-family peer collection → update the §1 peer section (fill in omissions). **Done** (3 corrections applied).
2. Exhaustive grounding of load-bearing claims (especially the §2 outer-cover table, §3-① "0 cases of a generator with names only", §3-③ the CDP partial counterexample). **Not done — only the review's spot checks exist.**
3. Cross-family adversarial review (levels 4–5) → derive confidence tags. **Done — verdict BLOCK, this document is the edition with corrections applied.**
4. **★New — fill the contract clauses of §9 into the design** (protocol version · unknown command · field-addition rule · owner absence · cancel/progress · timeout/retry · typed errors). If these are empty, none of the §8 proposals is a complete architecture.
5. **★New — give the user §8 first place (hybrid) vs second place (the report's proposal) as options.** The basis for the split is "whether to have a registration/discovery layer".
6. Give the user issues A·B as options → on decision, `/adr` (making 0022 confirmed or a follow-up) + moving the §5 rejected alternatives. **However, §5 is not used as grounds for rejection until the corrections above are applied.**

## 8. Higher-ranked alternatives given by the review

3 alternatives ranked by the adversarial review. **First place ranks ahead of this report's own proposal (second place)** — see the basis for the split below.

1. **★New leading proposal — a static typed catalog + a hybrid of runtime owner/capability registration.** In Rust, **declare the command name · input · output · description · errors · side-effect metadata once**, and **generate the TS bindings and the LLM schema together**, while each process at startup registers with a generic broker **only the subset it implements right now**. The daemon needs to know only the owner token + the connection, without knowing the **concrete concept** of a "client shell". It keeps C3 (the derivation path we already trust) alive while combining the proven availability models of decentralized discovery (robotics middleware) and runtime dynamic registration (LSP). Review verdict: on the current evidence it has the strongest constraint fit and precedent support.
2. **The report's proposal — a static Rust vocabulary → TS generation + daemon relay.** The direction of drift prevention and transport independence is sound. But **Zellij is not the fully isomorphic precedent that was claimed** (§1), and unless **owner binding · runtime availability · LLM schema · version negotiation** are attached, it is not a complete architecture. The grounds are stronger if the relay envelope carries the command name, caller identity, request ID, target/owner and protocol version.
3. **Caller/shell-side discovery + aggregation.** A CLI·LLM host or a client shell asks each reachable owner for its list and merges the namespaces. It is supported by the robotics middleware's local graph query, mobile-OS intent matching, and the MCP multi-server/client model. It **preserves the daemon's ignorance of the shell most cleanly**, but the discovery and merge logic is duplicated in every caller or a shared client library is needed, and the policy for shell-owned commands in headless situations has to be decided separately.

**The evidence that separates first and second place:** the precedent that a distributed system whose **owners and availability change at runtime has a separate registration/discovery layer**, not a "static typed contract" (robotics middleware · LSP dynamic registration). **In the report's proposal that layer is undecided.**

## 9. Contract clauses missing from the design

**The report's proposal (a single enum + daemon relay) has no evolution or failure contract.** The minimum required clauses the review pointed out:

| Clause | Why it is needed |
|---|---|
| Protocol version | independent processes meet as builds from different points in time |
| Unknown-command handling | even if the name is on the outer cover, the receiving side may not know that name |
| Field-addition rule (additive) | an extension must not break existing callers |
| Owner absence | the shape of the response when a registered owner has disappeared |
| Cancel · progress reporting | interruption and intermediate state of long-running commands |
| Timeout · retry | when the relay path does not respond |
| Typed error model | failure as a contract, not as a string |

- **Self-contradiction pointed out:** while warning in §3-① warning 1 about the closed-enum accident of LSP, the report **does not present the extension rule of its own contract.**
- **Precedent:** even the closest peer (WezTerm) keeps an **explicit codec version raised on incompatible type/protocol changes**. <https://github.com/wezterm/wezterm/blob/main/codec/src/lib.rs>
