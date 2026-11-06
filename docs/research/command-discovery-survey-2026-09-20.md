# Command discovery structure survey — dividing the work between `engram help` and `engram commands` (design survey)

| Item | Value |
|---|---|
| **Status** | **Version with 1 adversarial review applied** (the draft got a BLOCK verdict and was rewritten). The main session selected the findings, and **the findings rejected are kept in §7 with the reasons** |
| **Date** | 2026-09-20 |
| ★**Correction 2026-09-23**★ | **The description of the current state in §1 is outdated.** Afterwards the `help` body was restructured into **5 sections** (`root` · `mail` · `agent` · `window` · `theme`), and the groups the root names also became **four**. The fixed assembly table is gone and it is now a scheme of picking one section, and the file name is `prompts/engram-help.md`. ★The `파일:줄` anchors in §1 are values from that time and no longer match the current code★ — but the survey's **question** (how to connect the hand-written list and the runtime registry) is still open |
| **Method** | External precedent survey (official docs · issue originals cross-checked) + **reading this repository's code directly** (every line in §1 carries a `파일:줄`) |
| **★No new measurement at all★** | ★**We have never actually driven an agent to measure 「목록을 보고 옳은 명령을 골랐나」**★ — none of the figures in this document is a measurement on our screen. That absence is the largest limit of §5 and is registered as a known gap in §7 |
| **Confidence legend** | ★**Code-measured**★ = this session opened that `파일:줄` and confirmed · **Original cross-checked** = opened that URL and matched the wording · **Secondary source** = saw only a summary · ★**Unchecked**★ = no source could be given (including what the draft wrote as 「확인했다」 but left no link for) |
| **Why this survey was done** | The root help gives only two group names, yet the names that can actually be called number 22. An omission accident really happened once, and 「손으로 적은 목록과 런타임 명부를 어떻게 잇나」 was left open |

> ★**Read first — the evidence grade of this document splits into two layers**★
>
> **§1 (our current state) is code-measured** — reproducible by line number. **§2~§3 (external precedent · figures) are other people's measurements, and none of them was measured at our screen size.** ★**The figures in §3 are background, not grounds for the verdict**★ — the draft used them as grounds for the recommendation and the adversarial review issued a BLOCK on the spot. Do not repeat the same mistake.

---

## 0. Conclusion and scope

**Question:** How do we let agents discover the commands they can call — how do we divide the work between the two surfaces, the hand-written `engram help` and the runtime catalog `engram commands`.

**This round's answer = go small.** Pull the root screen's **group list** from the catalog (option A), and add a parity test **as an after-the-fact detection device** (option B). ★**The large overhaul that renders both surfaces from one model (option E) is deferred, not rejected**★ — the reason is that the user fixed this round's scope as small, not that we decided E is bad.

★**The axis this recommendation stands on is cost and scope, not the token figures of §3**★ — those figures **cannot separate** A · B · D · E (§3).

---

## 1. Our current state — all code-measured

**The two surfaces coexist, and they differ in material, transport and language.**

| | `engram help` | `engram commands` |
|---|---|---|
| Material | The hand-written `prompts/engram-cli-help.md` | A daemon catalog round trip |
| Transport | A local file read (no daemon needed) | `POST /control/commands` on every call |
| Language | English | The summaries are in Korean |
| Does it read the catalog | ★**No**★ | That is all it does |

- **The `help` body is split into 17 sections and a fixed assembly table joins them** — the section id declarations `crates/engram-dashboard-daemon/src/bin/engram.rs:323‥331`, the assembly `:489‥517`, the file path constant `:299`, the built-in fallback `:319` (it is `include_str!`, so it is byte-identical to the file). The 17 lines actually displayed = `prompts/engram-cli-help.md` (122 lines in total).
- **The groups the root names are only two** (`mail` · `agent`) — the two sections `root.group.mail` and `root.group.agent` are all of it. The daemon's own code documents this property: 「그 화면은 **정적**이라 계열 이름 둘만 내고 칸은 하나도 안 낸다」 (`crates/engram-dashboard-daemon/src/control/catalog.rs:420‥422`).
- **The names that can actually be called number 22** — the daemon's own table 5 (`declare_commands!` at `crates/engram-dashboard-agent/src/commands.rs:30` = `agent.list` · `spawn` · `new` · `rename` · `move`) + the 17 the shell registers (`src-tauri/src/layout/commands.rs:68` = `tab.*` 5 · `window.*` 3 · `slot.*` 6 · `layout.setSlotContent` · `agent.spawnInto` · `ui.refresh`). ★The draft's 「약 24 개」 differs from the value this session counted — **22 is the measured value.**★
- **All the material for an automatic list already exists** — `CommandSpec` has `name` · `effect` · `since` · `summary` (one line) · `args_schema` · `ok_schema` · `errors` · `args_type` · `ok_type` (`crates/engram-dashboard-command/src/spec.rs:29‥47`). Every declared command carries one doc-comment line.
- **`engram commands` is already two layers** — the list is 「이름 + 한 줄」 (`bin/engram.rs:2470`), and `engram commands <이름>` gives args · return · error codes (`:2839`). The verb itself is at `:156`, the route constant at `:165`. Many renderer unit tests are attached (`:5735` and below).
- ★**A reachability field already exists**★ — the discovery response is `{name, help, callable}` (`control/catalog.rs:103‥129`), and the CLI splits `callable=false` into a separate area when printing (`bin/engram.rs:232` · `:2486`). 「지금 부를 수 있나」 is **already a contract** (`catalog.rs:84‥92`).
- ★**But `owner` is not carried on the wire**★ — `RosterEntry` has it (`crates/engram-dashboard-command/src/roster.rs:14`), but the list assembly builds only the three fields `name` · `help` · `callable` (`catalog.rs:119‥124`). The draft's 「`owner` 필드가 이미 있다」 is **true only inside the registry.**
- **Four constraints**
  1. **Mail is not in the catalog** — it has separate routes (`/control/send` · `/control/messages` · `bin/engram.rs:975‥977`). If the list is built from the catalog alone, mail disappears.
  2. The `summary` is entirely Korean (`src-tauri/src/layout/commands.rs:108` · `:175` etc.) while the help file is English (`prompts/engram-cli-help.md:21`).
  3. The **description sentences** of args are not in the schema — only the type and whether required.
  4. `since` is a generation stamp per declaring crate, so it is not an absolute release order and cannot serve as material for sorting or filtering.
- **There is no budget problem** — at most 25 lines per screen, 8 lines for the root, 22 commands.
- **There are only two MCP tools** — `eg_send` · `eg_messages` (`control/mcp_server.rs:330` · `:335`). ★**There are no MCP tools in discovery (`/control/commands`)**★ — the machine path is only HTTP POST and the CLI verbs.

---

## 2. External precedent

### 2-1. Root screen policy

- **git = a trimmed root + an escape hatch to the full list.** `git help` with no option gives "a list of the most commonly used Git commands", and `--all` gives "all available commands". (original cross-checked — https://git-scm.com/docs/git-help) But **that document does not explain the category grouping of the output** (it is in the output but undocumented).
- **gh** = divides into named areas but shows all built-ins at the root, and takes extensions out of the root and points them to `gh extension list`. (secondary source — https://cli.github.com/manual/gh)
- **kubectl** = groups by task category + includes most built-ins, with plugins under `kubectl plugin list`. (secondary source)
- **cargo** = built-ins by category + description, external `cargo-*` listed as "installed commands" **without a description** — an open issue. (https://github.com/rust-lang/cargo/issues/10662)
- **There is no convention for a line-count cap** — what clig.dev prescribes is not length but **depth and concise-by-default**. (original cross-checked — https://clig.dev/)

### 2-2. Where the summary wording comes from

- **Cobra**: `Short` is rendered in the list and `Long` in `help <cmd>` — ★**both are struct fields in code, and that behavior is stated explicitly by the Cobra package docs**★. (original cross-checked — https://pkg.go.dev/github.com/spf13/cobra#Command) ★The draft's 「별도 문서 출처가 없다」 was wrong — only the rendering claim was true and the absence claim was false.★
- **clap**: a doc comment becomes `about` (an explicit attribute wins). (original cross-checked — https://docs.rs/clap/latest/clap/_derive/index.html)
- **click**: the first sentence of the docstring is the short help, and when there is no docstring nothing is enforced, so **a command without a description is silently created**. (secondary source)
- → **Our `declare_commands!` doc-comment line = the same thing as Cobra `Short` / a clap doc comment.** It is already the mainstream method.

### 2-3. How dynamic · plugin commands are put in the list

- **docker** — "Plugins are listed in the existing 'Commands' category and not separated out into 'plugins'". ★**This quote comes from a 2018 *proposed design* issue and is not current product documentation**★. It is true that the current source attaches plugin stubs and vendor marks, but **this quote cannot supply grounds that this way of presenting is a stable product contract.** (https://github.com/docker/cli/issues/1534)
- gh · kubectl = take out of the root and use a separate list command. cargo = include but without a description.
- → Directly applicable to our 17 shell-registered commands. But **to use `owner` on the screen, it must first be carried on the wire** (§1).

### 2-4. The surface agents read

- **Two layers is the converged shape** — a short list → detail when needed. We found no precedent that uses 3 or more named layers (**evidence of absence**, not a proof of a ceiling).
- **ACP slash commands** = written by hand and pushed by the agent. A flat single layer, and when availability changes it **pushes the whole list again**. (secondary source — https://agentclientprotocol.com/)
- **herdr** = two layers, `api schema` (short summary) ↔ `--json` (full schema). Even when a feature is off it **does not hide it from the list** and rejects it at call time. (secondary source — https://github.com/herdrdev/herdr) ★Our `callable` field has the same shape★ (§1).
- **OpenAI lazy loading** = splits the text budget in two: a short description read when choosing / a complete description read when using. ★**Unchecked — the draft left no source URL.**★

### 2-5. Techniques to prevent drift

★**Generating at render time removes one *kind* of drift — a hand-copied duplicate splitting apart.**★ ★**And it brings in other kinds**★: bugs in the generator itself · stale runtime state · omission of entries that have no owner · **rendering before registration finishes** · a catalog read failure. ★The draft's 「생성만이 구조적으로 drift 를 없앤다」 was an exaggeration — do not quote that sentence to justify A.★

- **That an accident can happen has been confirmed — how common it is we do not know.** Two samples: a hand-maintained usage string omitted 13 real flags (kazi#1792) · a document calling itself a 「전체 명령 목록」 missed 23 of 219 (Soup#822). ★**Neither URL was recorded**★ — the draft's collector wrote that it opened them directly but left no link. **There is no denominator and no sample design, so do not read it as 「흔하다」.**
- What each technique **cannot catch**:
  - Build generation + `git diff --exit-code`: cannot catch **the generator itself being wrong** — if there is a bug the gate stays green forever.
  - Golden/snapshot: **an omission that already existed** is enshrined in the first snapshot.
  - Parity test: aims squarely at this failure pattern but **can itself become vacuous** — a case where a regex matched 0 items after a wording change and `all()` over an empty list is true, so it was **green while checking nothing**. (https://github.com/KCNyu/clawock/issues/1607 · the property of Python `all()` = https://docs.python.org/3/library/functions.html#all)
- **A generated/hand-written split** (the structural list generated, the narrative by hand): clig.dev and a few frameworks recommend that shape. ★**There are no grounds to call it 「통설」, and we found no source that prescribes this split directly for command discovery aimed at an LLM.**★

---

## 3. ★Background figures — not used for the verdict★

The figures below are **quoted accurately and the sources are right.** But ★**they cannot separate our options**★ — all of them measure **tens to thousands of tools carrying full schemas**, while our surface is **22 commands · one-line summaries · at most 25 lines per screen**. Since §1 already measured that 「예산 문제는 없다」, these figures support none of A · B · D · E.

- Anthropic Tool Search: Opus 4 49% → 74%, Opus 4.5 79.5% → 88.1%. Context about 77K → about 8.7K. A case where in-house tool definitions reached 134K tokens. (original cross-checked — https://www.anthropic.com/engineering/advanced-tool-use)
- code execution with MCP: tool definitions of a real workflow 150,000 → 2,000 tokens. (original cross-checked — https://www.anthropic.com/engineering/code-execution-with-mcp)
- RAG-MCP: full-catalog selection accuracy 13.62% → 43.13%. ★**The experimental conditions are 20 web-search tasks · qwen-max-0125 · 1 relevant MCP among the distractors · an automatic judge**★ — not grounds for a 24-item CLI list. (https://arxiv.org/abs/2505.03275)
- ★Rejected figures★: per-model curves like "Haiku 4.5 91%→87%" — **there is no such figure** in the cited source (a fabricated attribution in a search summary). Claims like "the knee is near 40 items" = vendor internal trace claims, not treated as measurements.

---

## 4. Options

| | What | What it gains | What it loses / cost |
|---|---|---|---|
| **A. Generate the root from the catalog** | Pull the root's group list from the catalog and render it. Keep the hand-written fragments for group descriptions · mail | The path by which the hand-copied group list splits apart disappears | Special handling for mail. Mixed Korean/English (§1 constraint 2). Renderer work. ★**New failure modes come in**★ — the list in §2-5 |
| **B. Keep the trimmed root + a parity gate** | Add a test 「카탈로그의 모든 계열이 루트에 이름이 있나」 | Small. Keeps the same orthodox shape as git | After-the-fact detection. ★**Not an independent backstop**★ — §5 |
| **C. Keep as is** | Do nothing | 0 | The same omission repeats — it already happened once |
| **D. Everything automatic, abolish the hand-written help** | Absorb `help` into `commands` | One surface | ★**What the draft wrote, 「우편이 사라진다·한글만 남는다·인자 설명이 없다」, is not a decisive disqualification**★ — all three are **properties of the current schema** and can be fixed (register mail in the catalog or lay it on as a union · a summary locale field · add an arg description field). The remaining real cost is that schema repair and the loss of narrative |
| **E. Two surfaces, one model** (new) | `engram commands` = **the canon for machines · agents**, `engram help` = **the guide for humans**. Both render from one versioned model and the hand-written notes are laid on in explicit fields | The path by which the two surfaces could split disappears in principle. A routing rule for which one is authoritative stands | The largest — a model · version · locale · note fields must be newly designed. Outside this round's scope (§9) |

★**That E was missing from the draft was the adversarial review's biggest finding**★ — 「가장 뻔한 후보가 표에 없다」. It is put here.

---

## 5. The recommendation and its limits

**This round = A + B.** The root's **group list** is pulled from the catalog, and the **one-line group descriptions and mail** are carried by the hand-written fragments. **E is a candidate for the next round** (not rejected).

★**But do not call B 「생성기의 독립 백스톱」 — it is false**★. If the parity test **draws its expected values from the very same catalog · `owner` mapping that the generator uses, it is a common-mode tautology.** What it catches is only the case where the output vanished entirely, and **it cannot catch a wrong mapping that the two share.** To be a real backstop it needs ① an independent oracle (a hand-fixed expected list, etc. — which itself brings back the hand-written drift of §2-5) or ② a mutation test (inject a deliberate fault into the generator and check that the gate turns red). **If neither is built this round, B is a detection device, not a backstop.**

★**And first name what this design is trying to improve**★ — the goal is not 「drift 최소화」 but **「에이전트가 옳은 명령을 찾아 정확히 호출한다」**. ★**There is no evaluation set that measures that goal**★ — we have never once pitted A · B · D · E against each other on representative prompts · confusable command pairs · absent commands · multilingual queries. **So this recommendation is a cost and scope judgment, not a measurement.**

---

## 6. ★Open risks — what this design does not address★

1. ★**The summary wording given by the registrant is rendered as is on the screen the LLM reads — a trust boundary with zero defense.**★
   The `help` blobs of the 17 the shell registers are **preserved as opaque strings** (`crates/engram-dashboard-command/src/roster.rs:12` — no parsing · validation · branching, ADR-0156) and carried into the list (`control/catalog.rs:121`). **There is no length cap, no escaping, no source marking, no trusted-owner distinction.** Today there is no real harm only because the owner is our one shell, and **the moment a third-party owner attaches it becomes a prompt-injection surface.** (The adversarial review raised that MCP defines server metadata as an 「신뢰할 수 없는 힌트」 — its source is ★uncross-checked★: https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/)
2. ★**「존재한다」 and 「지금 부를 수 있다」 are not separated.**★
   There is a `callable` field (§1), but **in headless mode the shell's 17 drop out of the list entirely** — because `merge` does not create names that are not in the registry (`control/catalog.rs:61‥82`). And the daemon **has no material to tell the two apart**: the registration of a disconnected owner vanishes from the registry, so 「방금 끊긴 이름」 and 「한 번도 없던 이름」 **get the same answer** (`roster.rs:19‥21` — a loss ADR-0150 accepted). That is, the agent **cannot distinguish absence from temporary unavailability**, and may wrongly conclude 「없으니 다른 길을 찾자」. ★**Open question: do we give that distinction, and if so in which field.**★ (That file itself notes the direction of the answer — not reviving the trace but **carrying the connection list in the response**.)

---

## 7. ★Reviewed but outside this round — do not rediscover★

Findings raised by the adversarial review that are **real and valuable but that we decided not to address this round.** **Kept so the next session does not find them again from scratch.**

- **Three redesigns of the machine surface** — ① a stable JSON contract · schema version · locale · deterministic sort · structured errors ② snapshot version · change notification · cache invalidation (precedent: ACP pushing a full snapshot again when availability changes) ③ **query · capability filters** — `az find` searches in task language and `kubectl api-resources` filters by group · category · scope · verb and emits JSON/YAML/name output (https://learn.microsoft.com/en-us/cli/azure/reference-index#az-find · https://kubernetes.io/docs/reference/kubectl/generated/kubectl_api-resources/). ★**Reason = these are a redesign of the machine surface and form one lump with option E. The user fixed this round as small.**★
- **The absence of an agent evaluation set** — no option has ever been pitted on representative prompts. ★**Reason = building an evaluation set is a separate project in itself.**★ Registered as a known gap (§5 points to the same thing).

---

## 8. What needs a user decision

1. ★**Summary language**★ — the catalog summary is Korean, the help file is English. Automatic generation mixes them on one screen. (a) make the summary English (b) make the help Korean (c) leave it mixed. ★The draft wrote this mixing as a 「막는 비용」 but that was too much — **we have never measured whether a model cannot read the mix**, and there is also the path of adding a locale field. This is a **metadata policy decision**, not an argument against automatic rendering.★
2. **How to lay mail onto the root** — keep the hand-written fragment (current) vs. a refactor that puts mail into the catalog.
3. **Showing the 17 shell-registered** — do we add a docker-style 「출처」 mark (`owner` must first be carried on the wire), or take them out of the root as gh/kubectl do and point to a separate list.

---

## 9. ★Decided in this session — do not reopen★

1. ★**Attach the short prefix `eg_` to MCP tool names**★ — `send_message` → `eg_send` · `messages` → `eg_messages` (landing place = `crates/engram-dashboard-daemon/src/control/mcp_server.rs:330` · `:335`). **Reason = native tools named `send_message` · `spawn_agent` · `list_agents` really exist in codex** (confirmed by asking codex directly for its own tool list). Our tools would sit **in the same list under the same name**. Claude has no such native tools, so it simply has not collided so far.
2. ★**Do not attach a prefix to CLI · catalog names**★ — the `engram` executable itself is already the prefix, and the catalog names are **a contract shared by the frontend registry · golden tests · docs**.
3. ★**The large overhaul that merges the two surfaces into one (option E) is deferred — not rejected.**★ The starting point when reopening = the E row of §4 and the three redesigns of §7.

---

## 10. Gaps and limits

- ★**Measurement on our screen is 0**★ — the figures of §3 were all measured under other people's conditions, and we have never driven an agent against our list of 22.
- We could not find documentation of **how our peers (orca · paseo · t3code · superset · qm · sshx) show the command list to agents** — all abstained.
- We did not read the MCP core spec original directly (relying on secondary summaries). The MCP quote in §6-1 is also ★uncross-checked★.
- The two issues the draft wrote as 「직접 확인했다」 (kazi#1792 · Soup#822) and the OpenAI lazy-loading item **had no URL recorded, so this document cannot reproduce them** — the ★unchecked★ marks in §2-5 · §2-4 are that place.
- The absence of precedent for 3 or more layers is **evidence of absence**, not a proof that such a design is bad.
