# Agent persistence/memory storage — preliminary survey (beyond relational DBs)

- **Status:** survey only done · **no decision** · start deferred (→ tracking T-14). Input for when the "data storage/memory" step starts later.
- **Method:** `/research` medium (design-survey §7) — 5 collectors, one per candidate (Sonnet) + main grounding + 1 cross-family adversarial review (codex/GPT, effort high). 2026-07-19.
- **Confidence legend:** certain / likely / uncertain (derived from grounding + review, not self-reported). All vendor benchmarks are unverified.
- **Trigger:** spun off when, during a SQLite concept conversation, the user asked "is there a better way to store agent data than a relational DB". A pure survey memo (inline allowed, without the review pipeline).

---

## 0. Premise correction — engram does not yet have a data storage system (greenfield)

A fact that surfaced during the survey: **engram currently has no general-purpose data storage system.**
- `agents.json` (ProfileRegistry) = just a save file of thin profile pointers (session-id · epoch · cwd), not a general store.
- The daemon replay ring = memory-resident (volatile, 2MB/4096 cap).
- The SQLite mailbox (ADR-0086) = **only a paper decision, not implemented** — the current step is "minimal transport" (online push).

So this topic is not "extend what exists" but **"what do we build from a blank page"**.

## 1. "Data vs data control" → decomposed into 3 layers (the key framing)

The storage discussion has not two layers but **three**. The answer to "how far has the data come" differs per layer:

| Layer | Question | Status | Risk · character |
|---|---|---|---|
| ① **Storage engine** | *Where* do we put it | **a near-conclusion exists** | low risk · swappable behind a seam |
| ② **Data model/schema** | *What, with which relations* (the web) | **blank** | high risk · expensive if drawn wrong → be careful now |
| ③ **Data control/memory management** | *How* to reclaim · tidy · forget | **frontier** | hot research area · tied directly to LLM control |

The start order is **② → ③** (the engine ① is swapped later). Under engram's "low risk + long term = lay down now" principle, the seams · enums · boundaries of ② come first.

## 2. Frontier map (5 branches)

| Branch | What | Representative | engram fit |
|---|---|---|---|
| vector/embedding | semantic-similarity retrieval | sqlite-vec, LanceDB, FAISS | fits if embedded (but FTS5 first) |
| knowledge graph/temporal KG | relations · multi-hop · evolving facts | Graphiti/Zep, Kuzu (dead), SurrealDB | pattern useful, library premature |
| dedicated memory framework | *management logic* on top of storage | MemGPT/Letta, Generative Agents, Mem0 | **borrow the patterns only** (zero Rust-native) |
| event sourcing/durable | append-only · replay · audit | event-sourcing pattern, LangGraph, Restate | shares some elements (beware over-reading) |
| hybrid embedded | relational + vector (+ graph) in one local store | sqlite-vec, LanceDB, DuckDB, SurrealDB | practical answer candidate |

**2025~2026 trend (likely):** pure vector alone is receding → **BM25 + vector hybrid search** is standardizing · a triple vector + graph + KV backend (Mem0 · Zep) is mainstream · **"durable agents"** (resumable · auditable) are rising fast · memory moves from built into the framework → separated as a **shared infrastructure layer (MCP standardization)**.

## 3. What the adversarial review corrected (collector exaggerations → refuted)

- **[HIGH] LanceDB "Rust SDK 1.0" is wrong** — 1.0 is the *Lance file format SDK*; the `lancedb` **Rust crate is still 0.x**. "Stable because 1.0" is an exaggeration. (likely)
- **[HIGH] "engram has implemented 80% of event sourcing" is an unfounded figure** — the replay ring · epoch · mailbox are only *look-alike pieces*. Real event sourcing = an authoritative event stream + projection + schema evolution + ordering · concurrency + side effects. → lowered to "shares some elements".
- **[HIGH] "claude has individual memory via --resume, so per-agent is unnecessary" is also shaky** — a resumed session has context **compaction + a default 30-day expiry** (not lossless or permanent). Claude Code already has **repo-scoped auto-memory** (shared across sessions). The per-agent need has not disappeared.
- **[MED] "All frameworks are Python-only" is inaccurate** — Mem0 has Node too, LangGraph has an official TS. But **"cannot be embedded directly in Rust" is valid** (the core of engram).
- **[MED] Vector "no time order/non-deterministic" is an exaggeration** — datetime · metadata filters work, and exact search is deterministic.

## 4. Candidates the survey missed (must be evaluated next)

- **[HIGH] Claude Code's own filesystem auto-memory / RAG-over-files** — the most important miss. The engram agent is the claude CLI, so it already brings local · text · repo-scoped memory. **Before building a new store, first count what claude already brings.**
- **[MED] SurrealDB** — Rust · embedded · multimodal (graph + vector + FTS in one engine). Put it on the evaluation list.
- **[MED] Local MCP memory server** — the official reference has a persistent knowledge-graph memory server. **engram already uses MCP for the control channel** → this connects directly to a cross-agent memory seam.
- **[MED] Sleep-time compute** — background consolidation when idle (memory-at-rest, Letta).
- **[LOW] Prompt/KV caching** — not persistence; classified as volatile working-memory optimization (Claude cache TTL 5 min/1 hour).

## 5. engram decision (constraints: local single host · Rust embedded · dozens of agents · LLM control)

**Fits (now/soon):**
- **FTS5 (SQLite's built-in BM25) before vectors** — lexically strong for identifiers and code, already built in. Vectors only after measuring FTS5 recall/latency.
- **sqlite-vec** — when semantic search is needed, one line of SQLite extension, static-linked in Rust, zero added processes. But **pre-v1 (outside the bindings' SemVer)** → hiding it behind a seam is a precondition.
- **Borrow patterns (not libraries):** Graphiti *bi-temporal edge* (validity time + observation time), Generative Agents *importance + decay + reflection*, Mem0 *consolidation (ADD/UPDATE/DELETE)*. All are *ideas* that can be implemented directly in Rust.

**Excess/premature:**
- Exclude every server type (pgvector · Weaviate · Milvus · Temporal · Restate server · Neo4j/Memgraph) — violates "single local daemon".
- **Kuzu** — technically fit as an embedded graph, but **Apple acquisition · 2025-10 GitHub archive (dead)**. A fork (LadybugDB) is reviving but adoption is on hold. (certain — multiple sources)
- **GraphRAG** — it is document RAG, not real-time agent memory, and the indexing cost is excessive.
- **Introducing a graph DB itself is still premature** — for the current web, SQLite recursive CTEs are enough.

**Mid-term candidates:** LanceDB (waiting for crate 0.x to mature) · SurrealDB (multimodal Rust embedded) · MCP memory server (directly tied to engram's MCP infrastructure).

## 6. Questions to open at the start (unresolved)

1. **② schema:** which entities (agent · session · message · task · tree node), with which relations (create · send/receive · parent-child), with what as the source of truth? (coarse design → PRD/TRD/ADR)
2. **The boundary with claude's existing memory:** the split between what claude auto-memory/resume already covers vs what engram will newly hold (cross-agent/orchestration memory).
3. **Drive ③ control as an LLM tool?** How do we tie MemGPT-style self-editing memory to §5 (LLM-first control).
4. **Engine starting point:** SQLite as the source of truth + FTS5 → (if needed) sqlite-vec, all behind a storage trait seam. When is a vector justified (a measurement trigger).
5. **Retention policy:** unbounded accumulation vs delivered/read pruning vs archive (consistent with ADR-0083's "keep the corpse" leaning).

## 7. Issues/limits

- All vendor benchmarks are unverified (Zep DMR/LongMemEval, FalkorDB "374x", the Mem0↔Zep LOCOMO dispute is ongoing) — cannot be cited without independent verification.
- Crate versions · licenses (sqlite-vec pre-v1, lancedb 0.x) are time-sensitive — recheck at the start.
- A medium single pass — the omission probe is one reviewer's worth. At the start, a re-survey at deep (independent parallel collection + including SurrealDB/MCP memory) is recommended.

## Sources (core)
- Zep: Temporal Knowledge Graph for Agent Memory — arXiv:2501.13956
- MemGPT / Letta — github.com/letta-ai/letta · Generative Agents(Park 2023)
- Mem0 — arXiv:2504.19413 · A-MEM arXiv:2502.12110 · MemoryOS(EMNLP 2025)
- sqlite-vec — github.com/asg017/sqlite-vec · alexgarcia.xyz/sqlite-vec/versioning.html(pre-v1)
- LanceDB — docs.rs/crate/lancedb(0.x) · lancedb.com/blog/announcing-lance-sdk (format 1.0)
- Kuzu archive/Apple acquisition — github.com/kuzudb/kuzu · theregister.com(2025-10) · LadybugDB
- SurrealDB — github.com/surrealdb/surrealdb
- MCP reference servers(persistent memory) — github.com/modelcontextprotocol/servers
- ESAA event sourcing — arXiv:2602.23193 · Claude memory/sessions docs (auto-memory · 30d expiry)
- event sourcing/durable — Azure Event Sourcing pattern · LangGraph persistence · Restate/DBOS

*Feedback: none.*
