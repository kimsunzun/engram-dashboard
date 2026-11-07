# TRD — Boundary refactoring 2-1: cut the shell → agent dependency (S21)

> Status: **4th edition (2026-10-06) — `/review trd` round 2 (codex PASS · doc-aware FIX) incorporated.** Not a single line of code has changed yet.
> **Decision source (결정 출처):** user delegation (2026-10-06 「알아서 해 … 단계 착수」) → main decision. D1~D7 · the handling of O1~O5 in §8 · the review fixes of the 3rd edition (including the 「감수한 대가」 of F1) come from here. Beyond that, 「LLM 통지 없음」 · 「warn 로그 유지」 of D4 are the user's decisions in ADR-0270 decision 4 (2026-10-02).
> **Revision (개정) (2026-10-06, 4th edition — review round 2):** ① U3 gets slot-contract comments at two places (`connection.rs:61-64` · `:183`) and an anchor `:971` (N1 · §2-4 · §3-1) ② order inside U3 — `:971` is flipped last (N2 · §3-4) ③ U1 test ㉢ gets a word outside the table (N3 · §3-2) ④ the `layout_commands` binary **calls** the daemon (`:3033`) — the fact is corrected and the 「전」 measurement moves before U2 ① (N4 · §2-5 · §3-3 · §9) ⑤ the `send_command` text-invariance test becomes a byte comparison (N5 · §3-4) ⑥ the table doc gets 「`refusal` 을 `Some` 으로 바꾸기 전에 ADR-0279 재론」 (leftover of F1 · §2-1) ⑦ the structs and constructors that gain a new field (minor finding · §2-1 · §2-4 · §3-1).
> **Revision (개정) (2026-10-06, 3rd edition — review round 1):** ① D4's decision on daemon refusal changes from splitting string codes to **the source type at the reply boundary** — an unknown or new daemon code also raises the box (codex C1 · §2-4 · §3-4 · §7 3) ② D1's door does not distinguish callers — the human path also seeing the 「LLM 제어 표면」 policy is recorded as 「감수한 대가」 with a reconsideration trigger · correcting the agent comments to match the facts goes into U1 (F1 · §2-1 · §7 1) ③ a dispatch-level test for the refusal branch — a policy seam in `ConnectionCore` (F2 · §2-1 · §3-2) ④ the spelling reason is withdrawn from rejection (a) (F3 · §1-5 · §2-1) ⑤ §7 1 adds the 「정책을 보는 문은 데몬 `agent.new` 하나」 of ADR-0270 「영향」 (F4) ⑥ unit file list · documentation follow-up addendum (F5 · F6 · §2-5 · §3-1 · §6) ⑦ four minor findings (capitalization of the message · causality in memo §11 · the U1 comment is true at every commit · the local message list is withdrawn under C1).
> **Revision (개정) (2026-10-06, 2nd edition):** ① removed 「슬롯 즉석 에이전트가 영속 프로필이 된다」 from the D1 rejection reasons (end of §2-1 「바로잡은 사실」 · §8 O1) ② corrected: `SpawnByCwd` does reach the registry cap (§1-5 · §8 O2) ③ widened the D4 trigger to every refusal the daemon issues (§8 O3) ④ handed O5 off to a single line in memo §11.
> ★**Precondition for starting — D1 is a partial amendment of ADR-0270 decision 3.1 → record the new ADR (ADR-0279 · §7) before starting U1**★ (CLAUDE.md 「설계 결정 기록」 — major decisions are recorded the moment they are made).
> **Scope = all of work order 2-1** (`docs/refactoring/architecture-discussion-2026-09-26.md` §3 decision candidate 5 · §10 2-1) — set up the backend-policy wall for slot spawn in the daemon, remove the shell's policy check and the daemon half of the reserved list so that the shell no longer depends on agent operations, and report registration refusals with an OS message box.
> **Placement rationale:** `docs/README.md` 「새 내용을 어디에 넣나」's 「새 기능 **설계 착수** → `process/SN-name/`」. The siblings of the same stage (the 1-1 and 1-3 TRDs) are in this folder.
> **Notation:** 「실측」 (measured) = measured at base commit `cf693f4` (master — the code is the same as HEAD `a65ffa7` of branch `v0.3.3/refactor/crate-boundaries`; the only difference is two `docs/decisions/` files), together with its command · 「결정(메인 2026-10-06)」 = from the 「결정 출처」 above · 「ADR」 = the body of a confirmed ADR · 「제안」 (proposal) = a proposal of this TRD (every new name is a draft).
> Anchors: **ADR-0270** (the charter of this stage — ADR-0279 partially amends decision 3.1) · ADR-0219 (the policy table and the doors that read it · the decision 5 door pinned to claude) · ADR-0029 (shell = daemon client) · ADR-0172 (on-the-fly creation does not use 「마지막 실패」) · ADR-0155 (registration bounce contract) · ADR-0012 (seam · standalone harness) · ADR-0006 (external calls outside locks) · ADR-0175 (agent in the shell dev graph) · step-log S21.

---

## 0. Conclusion (first)

```
① 정책 벽 = 데몬. 데몬의 SpawnByCwd 처리부가 스폰(`spawn_agent`) 전에 agent 의 llm_creation_refusal 을 부른다(D1).
     슬롯 스폰 경로 · 왕복 수 · 배치 순서는 그대로다. 셸 agent.spawnInto 와 프론트의 SpawnByCwd 두 경로가 함께 덮인다.
     그 문은 호출자를 가리지 않는다 — 사람 경로도 같은 표를 본다(감수한 대가 · 오늘 영향 0).
② 셸은 낱말 오탈자 그물(parse_backend)만 남기고, 예약 목록은 셸 자기 표 절반만 남기고, agent 의존을 지운다(D2 · D3).
     셸 시험은 agent 이름을 부르지 않게 고쳐 쓰고, 정책 주인의 단언은 데몬 · agent 시험으로 옮긴다(D5).
③ 게이트 — 셸 운영 그래프(normal · build)에 agent 0줄 + 짝(데몬 ≥ 1줄). dev 그래프는 일부러 뺀다(D6).
④ 데몬이 등록 · 차분에 Error 로 답하면 OS 메시지 박스 — 코드가 무엇이든(모르는 코드 포함), 셸 로컬 실패는 빼고.
     가름은 문자열이 아니라 답 경계에서 남기는 출처 타입이다. tauri-plugin-dialog Rust API · 새 좁은 포트 ·
     같은 (자리, 문구)는 셸 프로세스당 한 번(D4).
⑤ 단위 셋 U1 → U2 → U3. 어느 커밋에서 멈춰도 빌드 · 회귀가 초록이다. U2 · U3 은 /qa full(GUI).
```

**Decision source (결정 출처):** a main decision made under user delegation (2026-10-06 「알아서 해 … 단계 착수」) (front matter).

**Not done now:** moving slot spawn to `agent.new` (rejected by D1) · removing agent from the shell dev graph (shell → (dev) daemon → agent — work order 2-4) · retry after a registration refusal · adding a caller axis to the spawn packet (when the reconsideration trigger of 「감수한 대가」 in §2-1 fires) · policy on the frontend `CreateProfile` path (out of scope — memo §11 · §8 O5).

---

## 1. Measured current state (`cf693f4`)

### 1-1. Agent symbols the shell uses — four (two operational · two test)

| Place | Symbol | Use | Disposition |
|---|---|---|---|
| `src-tauri/src/layout/apply.rs:36` · `:558-572` | `commands::llm_creation_refusal` | `gate_backend` = `parse_backend`(`:553`) + policy | D2 — the policy part · delete the import, keep the net |
| `src-tauri/src/view_commands.rs:196-210` | `commands::COMMAND_SPECS` | `reserved_names` = agent half ∪ shell table half | D3 — delete the agent half |
| `src-tauri/tests/layout_apply.rs:37` | `llm_creation_refusal` · `AgentNewArgs` · `LLM_BACKEND_POLICY` | `every_creation_door_reads_one_backend_policy`(`:1408`) | D5 |
| `src-tauri/tests/layout_commands.rs:2686` | `COMMAND_SPECS` | `the_registration_packet_never_carries_a_name_the_daemon_answers_itself`(`:2685`) | D5 |

`normalize_cwd` is already base `path::normalize_spelling` (1-1 — `layout/commands.rs:38` · `:1127`). Any other use is 0 (`rg engram_dashboard_agent src-tauri` → the four above + the `apply.rs:559` doc link). The manifest = `src-tauri/Cargo.toml:71` (comment `:64-70`) · the shell entry `dependencies` in `Cargo.lock` also has this name → deleting the dependency changes the lock too (CI `--locked`).

### 1-2. Slot spawn path

- **Shell** `apply::spawn_into` (`:574-644`): 0) word net + policy (`:586-589`) · tab/slot guard (`:590-596`) → 1) `spawner.spawn_by_cwd` (`:598` · outside the lock) → 2) placement (`:600-` · one critical section). ★If 1) comes back as `?` it never reaches 2) — the layout is invariant★ (test `spawn_into_propagates_spawn_failure_untouched` — `tests/layout_apply.rs:1603`).
- **The shell door does not distinguish callers even today** — both the Tauri command `spawn_into` (`src-tauri/src/commands/layout.rs:492-519`) and the bus verb `agent.spawnInto` (`layout/commands.rs:1100-1153`) pass through the same `apply::spawn_into` → `gate_backend`. So the frontend `agent.spawnInto` (`src/commands/tabCommands.ts:238`) already consults the policy table today even when a human calls it.
- **Operational port** `DaemonSpawner` (`commands/layout.rs:84-113`) → `AgentCommand::SpawnByCwd`. ★The `AgentEvent::Error` arm of that `match` is never reached★ — `send_command` has already turned `Error` into `Err(message)` and returned it (`daemon_client/protocol_state.rs:29-37` · `connection.rs:971`). So the daemon's message goes to the caller as is, with no prefix — the bus `agent.spawnInto` carries that message as `CONFLICT` (`layout/commands.rs:1152` · `:1159-1161`).
- **Daemon** `SpawnByCwd` (`crates/engram-dashboard-daemon/src/connection_core.rs:1455-1500`): `spawn_command_by_cwd` (`:654-661`) → if absent `MISSING_BACKEND` (`:628` · `:1466-1469`) → `CoreProfile::new(.., auto_restore=false)` (`:1470-1476`) → `manager.spawn_agent(.., Fresh)` (`:1483`). ★It does not consult the policy★. Because it is on-the-fly creation, it does not use 「마지막 실패」 (`:1477-1482` · ADR-0172).
- **Places that send `SpawnByCwd`** — the shell `DaemonSpawner` (the two shell paths above) · `slot.createAgentHere` (`src/commands/slotContentCommands.ts:80`), which calls frontend `protocolClient.spawnAgent` (`src/api/protocolClient.ts:992-994`), and frontend `agent.spawn` (`src/commands/agentCommands.ts:104`) — the latter two are pinned to `'claude'` and do not pass through the shell policy check. Frontend `agent.spawn` is one of the two doors of ADR-0219 decision 5 that are 「일부러 claude 에 못박은」 (the other = `create_and_start` of the agent bus `agent.spawn` — `agent/src/commands.rs:967-1007` · does not consult the policy).
- **Places that consult the policy (today)** — agent `verb_new` (`crates/engram-dashboard-agent/src/commands.rs:1078`) · shell `gate_backend`. The table `LLM_BACKEND_POLICY` (`:645`) opens both claude and codex, so there are **zero refusals today**.

### 1-3. Reserved names · registration refusal

- `reserved_names()` (`view_commands.rs:204`) → bridge creation (`:370-371`) → `report` (`:436-481`) pulls it out and puts it into `refused`, and that is only one log line (`commands/view_bus.rs:53`).
- Registration packet = shell table declaration + webview declaration, **one packet** (`daemon_client/inbound.rs:183-187`). The registry folds the same names within one packet without an error (`crates/engram-dashboard-command/src/roster.rs:130` · test `:624`).
- Daemon refusal `refuse_names_i_answer` (`connection_core.rs:1160-1190`) = **only names the daemon answers** · the whole packet · `CONFLICT`. Both registration (`:1806`) and diff (`:1839`) pass through it, and the daemon test `registering_a_name_this_daemon_answers_is_refused_as_a_conflict` (`:4159`) measures both branches. The registry also has refusals of its own — someone else's name = `CONFLICT` (`roster.rs:248`) · size · cap = `INVALID_ARGUMENT` (`:283-320` · cap values `:82-100`). The refusal is carried by `reply_roster` (`connection_core.rs:1234-1251`) as `Error{message: "{CODE}: {message}"}` (`command/src/error.rs:307-310`), and the wire `Error` has no code field (`protocol/src/messages.rs:698-701`).
- **Two places in the shell that handle it** — registration `register_own_commands` (`daemon_client/connection.rs:1924-1976` · outcome task `:1961-1976` · warn `:1969`) · diff `push_delta` (`commands/view_bus.rs:63-100` · warn `:96`).
- ★**The provenance disappears at the reply boundary**★ — the connection loop folds the daemon's `Error` reply into `Err(message)` with `reply_outcome` (`protocol_state.rs:29-37`) and puts it into the waiting slot (`connection.rs:971`), but the `Err` of that same slot also receives shell-local failures — disconnect `SENT_OUTCOME_UNKNOWN` (`:677`) · not sent `UNSENT_ON_DISCONNECT` (`:702` · `:1535`) · send · serialization failure (`:1361` · `:1377`) · no request_id (`:1343`) · duplicate number `PENDING_SUPERSEDED` (`:1799`). `send_command` (`daemon_client/mod.rs:804-831`) also produces not-connected · channel closed · reply lost as the same `Err(String)`. So the two places today cannot tell 「데몬이 거절했다」 and 「셸이 못 보냈다 · 못 받았다」 apart by type, and the 「거절」 warn at the registration place is also printed on a disconnect (the `Err(_)` arm `:1974` is in practice never reached — §8 O4). → D4 keeps the provenance at that boundary (§2-4).
- The outer port of the connection task is only `DaemonEvents` (`daemon_client/events.rs:75` — 「그보다 넓히지 말 것」 `:68` · header `:3-8` 「`AppHandle` 을 쥐던 유일한 사유는 emit … 그래서 emit 만 포트로 끊는다」). The connection task does not hold `AppHandle`.

### 1-4. Dependency graph (measured `cargo tree`)

- The shell's direct workspace dependencies (normal · build) = agent · base · command · discovery · net · protocol. After 2-1 only agent drops out.
- `cargo tree --locked -p engram-dashboard -e normal,build --target all --all-features --prefix none | rg "^engram-dashboard-agent "` → **1 line today** (one direct edge — no other normal path). The same shape `-p engram-dashboard-daemon` → 1 line.
- dev graph — shell → (dev) daemon (`src-tauri/Cargo.toml:108`) → agent. It remains after 2-1 too (ADR-0270 「영향」 · work order 2-4).
- `cargo tree -i engram-dashboard-agent` **dies with rc=101** if the target is not in the graph (measured `-p engram-dashboard-net -e normal,build`).
- Seen in the form in which the shell uses net (default feature), the workspace dependencies are 0; with `server` turned on they are {platform, protocol} (measured `--depth 1` both combinations · `net/Cargo.toml:31-41`). → The 「forward 폐포는 {net, agent, protocol}」 in `src-tauri/Cargo.toml:82` is stale.

### 1-5. Facts that differ from the records

| Record | Actual (measured `cf693f4`) |
|---|---|
| Memo §3 · ADR-0270 「맥락」 — the shell uses 「세 가지」 from agent | Two (operational). `normalize_cwd` was moved to base by 1-1 — the shell `Cargo.toml:64` comment is already 「둘」 |
| ADR-0270 decision 3.1 · memo §3 · §10 — 「슬롯 스폰을 데몬 `agent.new` 명령 경로로」 | `agent.new` only registers (a sleeping state — `commands.rs:194`). To start it, `agent.spawn {target}` is needed once more. → D1 |
| Investigation input — 「`agent.new` 의 `backend` 는 PascalCase 라 셸이 lowercase wire 낱말과 갈린다(손 JSON 필요)」 | **The spelling does not diverge** — before the command table calls it, it moves the case of the enum field to the declared spelling (`command/src/table.rs:130-131` → `coerce::enum_words_to_declared_spelling` · `coerce.rs:101-109`). The only reason the shell has to build the argument JSON by hand is one — it cannot use agent's argument type (`AgentNewArgs`) (the purpose of 2-1) |
| ADR-0270 「맥락」 · memo §3 — the refusal-handling place = only `connection.rs` | Two (diff `view_bus.rs:96`) |
| Memo §3 line numbers `view_commands.rs:204` · `apply.rs:597` · `:585-620` · `layout/commands.rs:35` · agent `commands.rs:1077` | `:205` · `:598` · `:574-644` · `:38` (base import) · `:1078` |
| Memo §11 second line — shell `Cargo.toml` comment 「`COMMAND_SPECS` 하나만 남았다(실제 셋)」 | Actually two, and the comment (`:64`) has already been fixed. 2-1 deletes the dependency along with it |
| agent `commands.rs:600-614` — ③ frontend `humanOnly` among the 「LLM 제어 표면이 셋」 that ask the policy | Withdrawn on 2026-09-22 (`src/commands/agentCommands.ts:41`). The doors that consult the policy are now two |
| The table in the investigation input — `SpawnByCwd` 「닿지 않는다」 the registry cap | **It does reach it** (confirmed by main 2026-10-06 · §8 O2) — the on-the-fly new-registration branch looks at `check_roster_capacity` (`agent/src/manager.rs:1148-1151`). But it comes out as a string error, not as a typed code like `CONFLICT` |

---

## 2. Decisions (main 2026-10-06) · design (결정 (메인 2026-10-06) · 설계)

| | Decision | One-line reason |
|---|---|---|
| **D1** | The daemon `SpawnByCwd` handler consults the policy before spawning | One wall = the daemon (the meaning of ADR-0270) · slot behavior and round-trip count unchanged · also covers the frontend `SpawnByCwd` path |
| **D2** | The shell keeps only the word net (`parse_backend`) | The net does not need agent (it asks the protocol enum) |
| **D3** | Delete only the agent half of the reserved list and keep the shell table half | The daemon cannot see a name that is the same in the shell and the webview (code in §2-3) |
| **D4** | When the daemon answers registration or diff with `Error` (any code · excluding shell-local failures), an OS message box at two places — the source by the type at the reply boundary · tauri-plugin-dialog Rust API · a new narrow port | ADR-0270 decision 4 · a refusal of any code means the shell commands do not reach the daemon · do not let OS branching into the shell |
| **D5** | Rewrite the shell tests so they no longer call agent, and move the policy owner's assertions to the daemon · agent | Do not put an agent dev dependency in the shell |
| **D6** | Gate 「셸은 agent 를 의존하지 않는다」 (normal · build 0 lines + its pair) | A mechanical wall for the ADR-0270 invariant · avoids the `-i` trap |
| **D7** | In passing — two stale comments · ADR-0219 by a link from the new ADR | — |

### 2-1. D1 — the policy wall is the daemon `SpawnByCwd` handler

- **Chosen:** when the daemon receives `SpawnByCwd`, it calls the policy (operational = agent `llm_creation_refusal`) before the spawn (`spawn_agent`), and if it refuses, answers with `Error` — an error path the shell already surfaces as is (§1-2).
- **Reason:** it keeps the meaning of ADR-0270 (the policy wall is the daemon) while slot behavior · round-trip count · placement order stay the same. It also covers the two paths of `SpawnByCwd` that the frontend sends directly (`slotContentCommands.ts:80` · `agentCommands.ts:104`). The shell does not become, for the first time, a producer that issues a bus call (`AgentCommand::Command`) (zero such producers in the shell Rust — investigation · `messages.rs:645` comment).
- **Rejected:**
  - (a) slot spawn as `agent.new` + `agent.spawn {target}` — `agent.new` only registers (a sleeping state), so there are two round trips; if the second fails a sleeping orphan remains; the shell cannot use agent's argument type (`AgentNewArgs`), so it would have to build the argument JSON by hand; and the frontend's `SpawnByCwd` slot path would still bypass the policy.
  - (b) a new daemon verb that creates and starts in one go — gains nothing over D1 and only grows the surface.
- **ADR:** partial amendment of ADR-0270 decision 3.1 → ADR-0279 §7.
- ★**Accepted cost (감수한 대가) — this door does not distinguish callers**★ (main decision 2026-10-06 · review F1): the policy name is 「LLM 제어 표면」, but the daemon `SpawnByCwd` does not know who sent it. So after 2-1 **the human path also consults the same table** — what is newly reached is the two doors pinned to claude (`slot.createAgentHere` · frontend `agent.spawn` — the latter is the 「일부러 claude 에 못박은」 door of ADR-0219 decision 5), and the frontend `agent.spawnInto` is **not a new exposure** (the shell door already consults the table without distinguishing callers — §1-2). As a result two doors with the same name diverge — the frontend `agent.spawn` consults the table while the agent bus `agent.spawn` (`create_and_start`) does not. **Zero impact today** — the table closes nothing and claude is open. ★**Trigger for reconsideration: the day any backend is closed to the LLM or its declaration is missing (`NO_POLICY_DECLARED`) — at that point add a caller axis to the spawn packet.**★
- ★**Corrected fact (바로잡은 사실) — 「영속 프로필이 되느냐」 is not the axis that separates (a) from D1. Do not bring it up again as a rejection reason**★ (main decision 2026-10-06 · §8 O1): today `SpawnByCwd` also puts an on-the-fly profile into the registry and writes it to disk — `spawn_agent` → `register_for_spawn` → `upsert_preserving_hierarchy` (`agent/src/manager.rs:1161`) → `store.save` (`agent/src/profile.rs:387-395`) · protocol doc `messages.rs:113-115` 「spawn 경로가 그 프로필을 registry 에 등록·persist 한다」. Both paths are `auto_restore = false` (`connection_core.rs:1470-1476` · agent `commands.rs:1114`). The spelling is not a reason either (§1-5).

**Design (proposal):**

- **Place** = right after the `MISSING_BACKEND` decision (`connection_core.rs:1466-1469`) · before `CoreProfile::new` (`:1470`). ★The boundary is `manager.spawn_agent` (`:1483`) — it has to come before that★ — an on-the-fly profile is put into the registry and written to disk inside it (the chain of 「바로잡은 사실」 above), so if the check is placed after it, a refused on-the-fly profile remains. `CoreProfile::new` only builds in-memory values — putting the check ahead of it is a matter of placement.
- **Policy seam = one field of `ConnectionCore`** (draft `llm_policy: fn(&str) -> Option<&'static str>`) — `ConnectionCore::new` (`:1096-1107`) fills it with `llm_creation_refusal`. Tests plug in a fake that closes, through the `#[cfg(test)]` builder (draft `with_llm_policy`) — the operational constructor signature stays the same, so the callers (`agent_conn.rs:587` · test assembly `connection_core.rs:2751`) are not touched. The struct literal is in only one place (`:1108-1119`) inside `new`. ★The struct doc (`:1067-1071`) warns against 「이 struct 는 연결마다 새로 만들어지고 필드는 전 연결이 공유하는 핸들의 clone 이다 — 공유 핸들이 아닌 값을 넣으면 연결마다 별개인 상태가 생긴다」★ — the new field is a stateless pure function pointer, so there is no state that would differ per connection. Write that fact in the field doc and add one line of exception to the warning in the struct doc (U1). Because the table closes nothing today, real input cannot reach the refusal branch, so the seam is needed (ADR-0012). The decision · message is carried by one pure helper (draft `by_cwd_refusal(kind, policy) -> Option<String>`). `spawn_command_by_cwd` and its test (`:4934`) are not touched.
- **Words** = obtained by wire enum serialization (serde — `#[serde(rename_all = "lowercase")]`, `protocol/src/domain.rs:209-215`). They are not written by hand (the same technique as the shell test `wire_word` — `tests/layout_apply.rs:1369`). Table comparison ignores case (`commands.rs:675-680`).
- **Message** = exactly the frame of today's shell message — 「`backend '{word}' 는 아는 낱말이지만 이 표면으로는 지금 만들지 않는다 — {reason} 스폰 안 함.`」 (`apply.rs:568-570`). ★The only difference is the case of `{word}`★ — the shell trims the typed spelling (`trim`) and carried it as is (`Codex` also passes — protocol deserialization ignores case, `domain.rs:235-245`), whereas the daemon serializes the enum and always carries lowercase (`codex`). It still stays distinguishable from the typo message (「를 모른다」).
- **Does not use 「마지막 실패」** — neither activation nor on-the-fly creation has happened (ADR-0172 decisions 1 · 4 — there is no failure to record for something that was not done).
- **Wire · catalog** — `PROTOCOL_VERSION` · shell `CATALOG_VERSION` unchanged (neither the packet shape nor today's behavior changes).
- **Anchors** `// ADR-0270` · `// ADR-0279`.
- **Correct the agent comments to match the facts (agent 주석을 사실대로) (U1 · true at every commit):** ① In the policy section header (`commands.rs:593` 「LLM 제어 표면의 백엔드 생성 정책」 · `:600-614` 「LLM 제어 표면이 셋」), the table doc (`:629` 「LLM 제어 표면이 어느 백엔드를 만들 수 있나」) and the decision function doc (`:668-673` 「사람이 쓰는 문에서는 그대로 만들어진다 · 막는 것은 사람이 아닌 호출자뿐」), write that **the daemon `SpawnByCwd` door does not distinguish callers and applies this decision** (the human path is blocked too — 「감수한 대가」 above). ★In the table doc (`:629`) add one more line — 「어느 줄의 `refusal` 을 `Some` 으로 바꾸거나 선언 없는 낱말을 들이기 전에 ADR-0279 를 다시 연다 — 데몬 `SpawnByCwd` 문이 사람 경로도 막는다」★ (put the reconsideration trigger at the place where that edit happens). ② In the U1 commit, the 「문」 list **adds the daemon `SpawnByCwd` door and keeps the shell `gate_backend` door as 「2-1 U2 가 걷는다 — 그때까지 데몬 문보다 먼저 같은 표를 본다」** — because at the time of U1 the shell door is still alive. The U2 commit deletes that line. ③ (the withdrawn `humanOnly`) is corrected to match the facts in U1. `:1073-1077` 「형제 문의 자리」 follow the same rule.

### 2-2. D2 — the shell keeps only the word net

- Delete `gate_backend` (`apply.rs:558-572`) · step 0) (`:586-589`) calls `parse_backend` directly · delete the import (`:36`). `parse_backend` asks only the protocol enum (agent is not needed).
- **The refusal message makes one trip to the daemon** — the refusal comes back at `?` (`:598`) of step 1) and never reaches 2) placement → the layout is invariant (§1-2). The tab/slot guard (`:590-596`) is still before the spawn.
- Fix the 「두 축」 description in the header comment (`:531-556`) to 「셸 = 오탈자 그물 · 정책 = 데몬(ADR-0279)」. 「두 거절을 한 문구로 뭉치지 말 것」 stays alive as is (daemon message ≠ net message).
- Bus error code unchanged — both today's policy refusal and the daemon refusal after 2-1 go `not_applied` → `CONFLICT` (§1-2).

### 2-3. D3 — reserved list: keep the shell table half (decided from the code)

- Delete only the agent half (`view_commands.rs:205`).
- **The shell table half still has work to do (code)** — ① the shell table declaration and the webview declaration go as one packet (`inbound.rs:183-187`) ② the registry folds the same names within one packet without an error (`roster.rs:130` · test `:624`) ③ the daemon refusal only looks at names the daemon answers (`connection_core.rs:1160-1190`). So if someone attaches `help` to `tab.create` · `slot.close` of the webview registry, nothing except that half blocks it, and no refusal box appears either. The same conclusion as ADR-0270 decision 2 · the second item of its 「거부한 대안」.
- `reserved_names` stays, and fix the doc · module header (`view_commands.rs:8-15` · `:196-203`) — 「데몬 이름의 벽 = 데몬 거절 + 거절 박스(U3)」. The 「데몬 어휘를 전부 덮지는 못한다(`mail.*`)」 sentence loses its meaning (the shell no longer sees daemon names).
- The names filtered out today are 0 (code reading — the only webview commands carrying `help` are `tab.next` · `slot.empty` · ADR-0270 「맥락」) → app behavior invariant. Confirm with the GUI (§5-1).

### 2-4. D4 — the refusal as an OS message box

- **Chosen:** **both places** — registration (`connection.rs:1969`) and diff (`view_bus.rs:96`) · the **Rust API** of tauri-plugin-dialog · a **new narrow port** that does not widen `DaemonEvents`, plus a test double · today's warn log stays every time (the label too — §8 O4) · no LLM notification (user decision — ADR-0270 decision 4).
- **Trigger = every case where the daemon answered this registration or diff with `Error` (main decision 2026-10-06 · §8 O3 · review C1)** — that reply is 「적용되지 않았다」 and the result is the same (for registration the shell has zero commands, for diff that diff does not reach the daemon). Today's real cases are `CONFLICT` (daemon name · someone else's name) and `INVALID_ARGUMENT` (name 128 B · description 4096 B · 512 per owner · 4096 total — §1-3), and ★**an unknown or new code raises the box just the same**★ — adding a code is a permitted extension (`ErrorCode` doc 「코드 추가는 additive」 of `command/src/error.rs`). **Only shell-local failures are excluded** (the list in §1-3).

**Keep the source as a type at the reply boundary (review C1 — splitting string codes is withdrawn):**

- **Three places change; other callers are unchanged:**
  1. `connection.rs:971` — put the daemon's paired reply into the slot as is, without folding (`Ok(ev)` — `Error` too as `Ok(AgentEvent::Error{..})`). Then the `Err` of the waiting slot (`CommandReply` = `oneshot::Sender<Result<AgentEvent, String>>` — `:65`, **the type unchanged**) holds **only shell-local failures** — the seven local send places in §1-3 (`:677` · `:702` · `:1343` · `:1361` · `:1377` · `:1535` · `:1799`) are left untouched. ★The two places with comments stating that contract are fixed together★ — write 「**데몬 답을 슬롯에 넣는 자리는 `:971` 하나이고 `Error` 도 `Ok` 로 넣는다 · 슬롯의 `Err` 는 셸 로컬 실패뿐이다 · `Error` 를 `Err` 로 접지 말 것 — 접으면 거절 박스가 조용히 죽는다**」 in the `CommandReply` doc (`:61-64` — currently 「`Err(msg)` = 데몬 Error 또는 연결 끊김」) and in the `SendCommand` doc (`:183` — 「send/끊김 실패 시 Err」). Attach the `// ADR-0279` anchor to `:971`.
  2. `protocol_state.rs:29-37` `reply_outcome(ev)` → one function that turns a slot result into a source type (draft `classify_reply(Result<AgentEvent, String>) -> Result<AgentEvent, CommandFailure>`) · `enum CommandFailure { Daemon { message: String }, Local(String) }` (draft) · `Display` = the same text as today (`Daemon` = the original daemon message · `Local` = the local message). Decision: `Ok(Error{message,..})` → `Daemon` · any other `Ok(ev)` → `Ok(ev)` · `Err(s)` → `Local(s)`. **It does not read strings** — whether there is a code prefix or not, even an unknown code, if the daemon answered it is `Daemon`.
  3. One sibling in `DaemonClient` that preserves the source (draft `send_command_with_origin(cmd) -> Result<AgentEvent, CommandFailure>` — it moves the body of `mod.rs:804-831`, and not-connected · channel closed · reply lost · no request_id (`:806`) are `Local`) · the existing `send_command` calls it and turns `CommandFailure` into `String` (`to_string`). ★The text received by the existing callers (the six of `commands/agent.rs` · `commands/layout.rs:100` · tests) is byte-for-byte the same★ — even today a daemon `Error` came as `Err(message)`, and a local failure came as that constant text.
- **Two places:** the registration outcome task (`connection.rs:1961-1976`) separates the result of `outcome_rx` with the same function — `Daemon` → warn (the current label unchanged) + box · `Local` → warn only (the same label — §8 O4) · `Ok` → info. The diff place (`view_bus.rs:94-100`) calls `send_command_with_origin` and follows the same rule.
- **What does not change:** the wire (daemon `reply_roster` · `Error` shape) · the waiting slot type · `register_pending` · `src-tauri/tests/daemon_client_pending.rs` (handles only local `Err`).
- **The cost of the chosen shape, and the alternative (고른 모양의 대가와 다른 안):** that the `Err` of the slot is 「로컬뿐」 is guarded not by the type but by placement (the place that receives daemon replies is only one, `:971`) — if someone folds `Error` into `Err` again at that place, the box silently dies. Test ㉢ (daemon `Error` → box over a real socket round trip) catches that. There is also an option of changing the slot `Err` to a new type (draft `LocalFailure`) so that the compiler guards it, but it would spread to the seven local send places · `register_pending` · two test files (`daemon_client/tests.rs:3125` · `:3639` · `tests/daemon_client_pending.rs`), so it was not chosen (minimal change — 「keep it minimal」 of review C1).

**Rust API measured (box):**

- Locked version = `tauri-plugin-dialog` 2.7.1 (`Cargo.lock:4226-4227`) · direct dependency (`src-tauri/Cargo.toml:48`) · registration (`src-tauri/src/lib.rs:44`).
- Source = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tauri-plugin-dialog-2.7.1/src/` — `DialogExt::dialog()` (`lib.rs:83-91` · `AppHandle` etc. that implement `Manager`) → `Dialog::message(text)` (`:173`) → `MessageDialogBuilder::{title, kind, buttons, show, blocking_show}` (`:283-367`) · `MessageDialogKind::{Info, Warning, Error}` (`models.rs:10-17`).
- `show(f)` does not block — inside it a window is opened with `run_on_main_thread` and the waiting is carried by a separately spawned thread (`desktop.rs:215-245`). `blocking_show` is 「main thread 에서 쓰지 말 것」 (`lib.rs:351-358`) → not used.
- **No capability needed — confidence: certain (source reading)** — `dialog:allow-message` is the permission that opens and closes the JS-side IPC command `message` (`permissions/autogenerated/commands/message.toml:6-13`), and the Rust API does not go through IPC and calls `show_message_dialog` directly. `src-tauri/capabilities/*.json` unchanged (only `dialog:allow-open`).
- **No OS branching** — the OS differences of the window are inside the plugin (rfd 0.16). It matches ADR-0270 decision 4 「Tauri 의 대화상자 기능 뒤」, and no `cfg` enters the shell (platform gates 4f · 4h list unchanged).

**Box-side design (proposal — names are drafts):**

- New file `src-tauri/src/daemon_client/refusal.rs`:
  - Port `RefusalNotice: Send + Sync { fn show(&self, site: RefusalSite, daemon_message: &str); }` · `RefusalSite = Register | Update`. Separate from `DaemonEvents` (its doc `events.rs:68` forbids widening it — add one line for the sibling port to 「emit 만 포트로 끊는다」 of that file's header `:3-8`).
  - Operational adapter `TauriRefusalBox(AppHandle)` — `app.dialog().message(..).title(..).kind(MessageDialogKind::Warning).show(|_| {})`. Failures are swallowed (the same contract as `DaemonEvents`).
  - Test double `RecordingRefusals` (`#[cfg(test)]`) — records (place, message).
  - Filter `RefusalAlerts { port, shown: Mutex<HashSet<(RefusalSite, String)>> }` · `surface(site, daemon_message: &str)` — **called only when `CommandFailure::Daemon`** (the decision was already made by the source type above — no string is read here):
    1. **Repeat suppression** — if the same (place, daemon message) has already been shown in this process, it is not shown. The decision is inside the lock · the port call is after the lock is released (ADR-0006).
    2. One info log line (shown / held back) — the GUI measurement also counts the box decisions from the log (§5-2).
  - Ownership — `DaemonClient` (`daemon_client/mod.rs`) gets one `Arc<RefusalAlerts>` field and hands it to the connection task (the same path as `events`). ★There are four struct literals of it and all four fill the field★ — the same places that fill `events`: `new_with_events` (`:239` · literal `:246` · test) · `new_with_router` (`:275` · `:281` · test) · `new_with_router_and_timeout` (`:300` · `:307` · test) · `new_real_with_owned_runtime` (`:326` · `:337` · operational). Operational uses `TauriRefusalBox(app.clone())`, and the three tests use a double (or a do-nothing double — the same shape as `NoDaemonEvents`). The daemon-side `ConnectionCore` gets no new field in U3 (only U1's policy field). The diff place uses the same filter through an accessor (draft `client.refusal_alerts()`).
- **Box text (draft)** — follows the shell's convention for user-facing strings (hard-coded Korean — `tray/core.rs:35-41`). Title 「Engram — 명령 등록 거절」. Registration = 「데몬이 이 셸의 명령 등록을 통째로 거절했습니다. 다음 재연결까지 LLM 이 창 · 탭 · 슬롯 명령을 쓸 수 없습니다.」 · diff = 「데몬이 화면 명령 명단 갱신을 거절했습니다. 바뀐 화면 명령은 데몬에 오르지 않았습니다(셸 명령은 그대로입니다).」; both append the original daemon message (including the code prefix — overlapping names · exceeded sizes · cap values are carried there) and 「원인이 된 명령 선언을 고친 뒤 다시 빌드하세요.」. No per-code messages are provided — the daemon message already states the cause.
- **Repeat suppression — key = (place, daemon message) · lifetime = the shell process (a judgment of this TRD):**
  - ① A refusal is deterministic for the same build and the same names — on every reconnect the same packet is refused with the same message, so showing it again gives no new information, and when reconnects are frequent, as after a daemon restart or waking from sleep, windows pile up. 「연결마다 한 번」 cannot prevent exactly this repetition.
  - ② The daemon message carries the cause — all the overlapping names (`connection_core.rs:1162-1187`) · the exceeded size · the cap values (`roster.rs:283-320`). So a different incident is a different key and appears anew. A cap message carries the count remaining in the registry, so even the same incident can appear again if the count changes — we accept that because the state has changed.
  - ③ The reason the place is in the key — the two refusals weigh differently (registration refusal = zero commands even for shell names · diff refusal = only the screen part of the diff is missing). And the two places can be measured separately by the GUI (§5-2).
  - Cost — from the second time on for the same incident, only the warn log remains (it was already shown once, and the cause log is printed every time).

### 2-5. D5 — test relocation

| Today (shell) | What it measures | Where it moves | Unit |
|---|---|---|---|
| `every_creation_door…` ① | whether the whole wire vocabulary is declared in the policy table | the daemon `connection_core.rs` test (a place that sees both crates) — prevent blanks with `wire_slot`-style exhaustive `match` | U1 |
| ② | `agent.new` advertised vocabulary == the set the table opens | agent `commands.rs` test (both are inside agent) — together with the `advertised_new_backends` helper | U2 |
| ③ open arm | an open backend reaches the port | already in the shell — `spawn_into_forwards_a_known_backend` (`layout_apply.rs:1315`) | unchanged |
| ③ closed arm | a closed backend is refused before the spawn · carries the reason · is distinguishable from the typo message | daemon — a dispatch-level test that plugs a closing fake policy into `ConnectionCore` (§3-2 ㉡) | U1 |
| `layout_commands.rs` `the_registration_packet_never_carries_a_name_the_daemon_answers_itself` | the shell removes daemon names from the packet | the daemon-side assertion already exists (`connection_core.rs:4159` — registration · diff). The shell test is rewritten as the shell's part | U2 |
| `layout_commands.rs:629-634` `no_two_declarations_claim_the_same_name` | overlap of declared names in 「이 바이너리에 링크된 **전 crate**」 (the doc gives overlap with core `agent.*` as the reason) | kept · rewrite the doc scope (below) | U2 |

- **What the shell guards (U2):**
  - New test (draft `spawn_into_passes_a_daemon_refusal_through_without_placing`) — a fake `Spawner` answers `Err` with the refusal message · word `Some("codex")` → `calls() == 1` · message as is · tab count unchanged · resync 0. The existing `spawn_into_propagates_spawn_failure_untouched` (`:1603`) is word `None`, so it does not measure 「낱말이 데몬까지 갔다가 거절되어 돌아온다」.
  - `layout_commands` rewrite (draft `the_registration_packet_carries_shell_names_once_and_leaves_daemon_names_to_the_daemon`) — the webview reports the whole set of shell table names (`COMMAND_SPECS` — the shell's own table, which that file already imports · `:59`) · `tab.next` · one name string of the shape the daemon answers (`"agent.spawn"`) → `refused` = the shell table names · `tab.next` and `agent.spawn` are carried in the packet (the wall = the daemon — ADR-0270 decision 2) · the shell table names come from the table only once. The test bridge uses the real `reserved_names()` (`:2628`).
  - **The doc of `no_two_declarations_claim_the_same_name` (`:629-630`)** — that test is a linker collection (`duplicate_command_names` of `command` — `spec.rs:96`), so it sees **only the declarations of crates linked into this binary**. ★This file calls the daemon★ — `the_webview_deadline_fits_inside_the_daemon_seat` (`:3031-3033`) reads `engram_dashboard_daemon::command_delivery::CommandDeliveries::DEFAULT_DEADLINE` (the 「데몬을 부르지 않는다 · rg 0」 of the 3rd edition was an error from reading truncated search output). So the daemon is still linked after 2-1, and whether the agent declarations remain visible in this binary **depends on whether that incidental reference pulls in agent's declaration objects — uncertain** (the shell lib no longer calls agent). Rewrite the doc as 「셸 표와 이 바이너리에 링크된 crate 사이의 겹침만 잰다 · agent 선언이 보이는 것은 데몬 dev 경로의 우연이다 · 셸 · 데몬 이름 겹침의 벽은 데몬 등록 거절이다(ADR-0270 결정 2) + 거절 박스(U3)」. **Measurement (one-off, not committed) — the 「전」 measurement before U2 step ① (or at the start of U1), the 「후」 measurement after U2 step ⑤**: measure with the same binary whether `spec_of("agent.new")` is `Some`. Write the result in the U2 commit body.
- ★**Do not put an agent dev dependency in the shell**★ — shell tests do not call agent by name. The daemon dev dependency pulls agent into compilation (§1-4), but they do not use it.
- **Build at the destination first, then delete** — U1 adds the daemon part and U2 adds the agent part first, and only then are the shell tests deleted (regression-net gap 0).

### 2-6. D6 — gate 「셸은 agent 를 의존하지 않는다」

```bash
# 셸 운영 그래프(normal · build)에 agent 가 없다 → 0줄
cargo tree --locked -p engram-dashboard -e normal,build --target all --all-features --prefix none | rg "^engram-dashboard-agent "
# 짝 — 눈먼 게이트 방지 → 1줄 이상
cargo tree --locked -p engram-dashboard-daemon -e normal,build --target all --all-features --prefix none | rg "^engram-dashboard-agent "
```

- **Decision** — the first line is 0 matches (rg exit code 1) · the second line is 1 or more lines · FAIL if the `cargo tree` exit code is not 0 (the same shape as the discovery async-import gate `ci.yml:1163-1180`).
- **Does not use `-i`** — if the target is not in the graph it dies with rc=101 (§1-4), and 「없다」 (absence) would be read as FAIL.
- **Deliberately leaves out dev** — shell → (dev) daemon → agent remains until 2-4.
- **What the pair blocks** — the first line passing blind after an agent rename · a prefix change · a pattern typo.
- **The `^…␣` anchor (trailing space)** — without it a different name with the same prefix gets caught (the same reason as the comment on the discovery gate).
- It is the **resolved graph**, not a manifest grep — it also catches transitive paths where another dependency pulls in agent. It is the mechanical wall for the 「영향」 invariant (「`src-tauri` 의 매니페스트에 `engram-dashboard-agent` 가 없다(운영 의존)」) of ADR-0270.
- **Where to register** — `.github/workflows/ci.yml` `gates` job (proposal = after `platform gate 3` (`:761`)) · a new entry in `.claude/skill-bindings/qa.md` standard · a new line in CLAUDE.md 「빌드·검증 명령」 + one phrase in the src-tauri entry of 「백엔드 모듈 맵」 · `docs/testing-strategy.md` §1 src-tauri section.

### 2-7. D7 — in passing

- `src-tauri/Cargo.toml:64-71` — delete the comment and the dependency line (「뒤 단계에서 닫힌다」 is closed by 2-1).
- `src-tauri/Cargo.toml:79-82` net comment 「forward 폐포는 {net, agent, protocol} … 셋 다 이미 이 셸의 의존이다」 → 「기본 feature(이 셸이 쓰는 조합)로는 워크스페이스 의존 0 · `server` 를 켜면 {platform, protocol}」 (§1-4 measured).
- The 「문」 · 「재는 자리 셋」 descriptions of ADR-0219 are not edited in the body — ADR-0279 carries them by a link (`/adr link` · §7).

---

## 3. Work units

**Principle (user 2026-10-02 — 「망가지지 않는 단위로」 · 「임시 땜빵 금지」):** every unit is cut with the workspace building and the regression green. Build the destination first, then delete the old place. Between U1 and U2 both the shell door and the daemon door stand on the slot path, but that is not a patch-up but an order (the daemon door is the permanent wall and the shell door is removed by U2). The comments and docs in between are also written to be true at every commit (§2-1 「agent 주석」).

### 3-1. Unit table

| Unit | Scope | Files touched | Review · QA |
|---|---|---|---|
| **U1** | D1 + daemon tests (D5 ① · ③ closed arm) | daemon `src/connection_core.rs` (policy seam field and its doc · one exception line in the struct doc `:1067-1071` · `new` literal `:1108-1119` · handler · helper · tests) · agent `src/commands.rs` (comments only — `:593` · `:600-614` · `:629` · `:668-673` · `:1073-1077`) | `/implement standard` · `/review code full` · `/qa standard` |
| **U2** | D2 · D3 · D5 (② · shell rewrite · duplicate-name test doc) · D6 · D7 · delete the agent dependency | agent `src/commands.rs` (test · delete the shell door line of `:600-614`) · shell `src/layout/apply.rs` · `src/view_commands.rs` · `tests/layout_apply.rs` · `tests/layout_commands.rs` (including `:629-634`) · `Cargo.toml` · root `Cargo.lock` · frontend `src/commands/agentCommands.ts:46-51` (comments — 「형제 문 둘」 · 「재는 자리 = `layout_apply.rs`」) · `docs/reference/structure/agent-backend.md` (`:98` · `:107` · `:209` · `:217`) · `docs/reference/structure/agent-backend.html` (`:605` · `:616` — the pair of `.md`) · `ci.yml` · `qa.md` · `CLAUDE.md` · `docs/testing-strategy.md` | `/implement standard` · `/review code full` (+ doc lens — a load-bearing document) · `/qa full` (GUI) |
| **U3** | D4 + GUI measurement | Shell `src/daemon_client/{protocol_state.rs(`reply_outcome` → source classification · its test `:433-450`), connection.rs(`:971` + `// ADR-0279` · slot-contract comments `:61-64` · `:183` · outcome task `:1961-1976`), mod.rs(`send_command` `:804-831` · sibling · `DaemonClient` one field + four literals `:246` · `:281` · `:307` · `:337`), refusal.rs(new), events.rs(header `:3-8`), tests.rs}` · `src/commands/view_bus.rs` | `/implement standard` · `/review code full` · `/qa full` (GUI) |

**Order U1 → U2 → U3.** U1 must come before U2 — in reverse, the moment the shell check is deleted a policy hole opens in slot spawn (the third item of ADR-0270 「거부한 대안」). U3's code is unrelated to U2, but the trigger of the GUI measurement (§5-2) relies on D3 (the shell does not filter daemon names), so it comes after U2. The only file overlap is agent `commands.rs` (U1 comments · U2 tests and the shell-door line) — even so, **one coder at a time, in series**, is recommended (U3 may touch U2's Cargo.lock · CLAUDE.md again).

### 3-2. U1 — the daemon wall

- **Acceptance criteria:** ① a `SpawnByCwd` for a backend that the policy closes answers `Error` (the message of §2-1) **without any of** profile creation · registry registration · disk save · spawn ② an open backend is the same as today (`ws_e2e.rs` `case40_ws_spawn_by_cwd` green) ③ `MISSING_BACKEND` comes before the policy (a blank field = that message as is) ④ the shell is not touched yet ⑤ the agent comments state the facts as of this commit (the shell door is still alive · the daemon door does not distinguish callers — §2-1).
- **Tests (proposal · the test module of the same file):**
  - ㉠ `every_wire_backend_declares_an_llm_policy` — protocol wire vocabulary × agent table (shell ① moved over · `wire_slot`-style exhaustive `match`).
  - ㉡ **dispatch-level refusal test** (review F2 — model = `spawn_by_cwd_without_a_backend_is_refused`, `connection_core.rs:4822-4847`) — plug a closing fake policy into `test_core()` (`:2628`) and dispatch `SpawnByCwd{backend: Some(Codex)}` → the reply is **exactly one** `Error{request_id: Some(req)}` · `core.manager.agent_snapshots()` (`manager.rs:892`) · `list_agents()` are both empty · the message contains the reason, does not contain 「를 모른다」, and differs from `MISSING_BACKEND`.
  - ㉢ the operational default = the real policy — the policy of a core built with `ConnectionCore::new` gives the same answer as `llm_creation_refusal` for every wire word **and for one word outside the table (`"no-such-backend"`)**. All wire words are `None` today, so they cannot catch a fake that 「늘 연다」 (always opens) — it is caught only when the answers for the word outside the table must also match as `Some(NO_POLICY_DECLARED …)` (fail-closed — `commands.rs:666` · `:675-680`). The constant is private, so it is not called by name and is compared with the value of `llm_creation_refusal("no-such-backend")`.
  - ㉣ (optional) source order of the call site — not added — the boundary is `spawn_agent` and ㉡ measures it. The order relative to `CoreProfile::new` is only a matter of placement.
- **Gate:** `cargo fmt --check` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · `cargo test -p engram-dashboard-agent -- --test-threads=4` (doc link check even if only comments change) · CI.
- **Why it is not critical:** it only adds one decision **before** the spawn and passes through none of the invariants — kill causality · finalize · lock order · replay. The runtime behavior is the same, since the policy closes nothing today (the open branch = the current code as is). So the GUI is not needed either — the real spawn of the open branch is measured by CI `case40`.
- **Risks:** if the decision is placed after `manager.spawn_agent`, a refused on-the-fly profile remains in the registry · on disk (㉡ catches it — moving it between `CoreProfile::new` and `spawn_agent` moves only in-memory values, so it does no harm and ㉡ does not catch it either) · if the operational wiring of the seam is missing, the wall opens (㉢) · if the message is changed, the text the bus callers receive changes (apart from the case of the word, it is the current shell message as is) · the human path also consults the table (accepted cost · zero impact today — §2-1).

### 3-3. U2 — cutting the shell

- **Order inside the unit (the build stands wherever it stops):** ⓪ the 「전」 measurement of the duplicate-name test (§2-5 — it may also be done at the start of U1) ① first set up, on the agent side, the ② test of the §2-5 table together with the helper `advertised_new_backends` (the policy owner's side) ② shell `apply.rs` — `gate_backend` · delete the import · step 0) = `parse_backend` · header comment ③ shell `view_commands.rs` — delete the agent half · header · doc ④ shell tests — the `every_creation_door…` of `layout_apply.rs` and the helpers that only it uses (`WIRE_BACKENDS` · `wire_slot` · `wire_word` · `advertised_new_backends` — `:1349-1396` together with the section header comment) · delete the import (`:37`) · fix the mention of that test in the `:1308-1313` doc · the new refusal pass-through test · the `layout_commands.rs` rewrite and the `:629-630` doc ⑤ delete the `src-tauri/Cargo.toml` dependency (including the two D7 comments) · regenerate `Cargo.lock` · the 「후」 measurement of the duplicate-name test (§2-5) ⑥ the D6 gate · delete the shell-door line of the agent `commands.rs` 「문」 list · frontend · structure docs · the remaining docs. During ①~④ the operational dependency is still there, so it is merely an unused dependency and the build stands. ★④ must come before ⑤★ — an integration test sees only the dependencies of its package, so deleting the dependency while a test still calls agent breaks compilation.
- **Acceptance criteria:** ① the two D6 lines = 0 lines · 1 or more lines ② 0 `engram_dashboard_agent` in shell sources · tests (`rg engram_dashboard_agent src-tauri` → 0 lines) ③ `agent.spawnInto` launches claude · codex and places them as it does now, and refuses typos before the spawn ④ the names filtered out of the registration packet are 0 today (GUI log — §5-1) ⑤ the bus error code is unchanged (`CONFLICT`) ⑥ no text pointing to 「셸 `gate_backend` 문」 remains in code or docs (`rg gate_backend` → only the date-stamped snapshot `architecture-map-notes.md` and old records).
- **Tests:** the U2 part of the §2-5 table.
- **Gate:** `cargo build` · `cargo fmt --check` · `cargo test -p engram-dashboard --test layout_apply` · `--test layout_commands` · `--test lib_unit` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · `npm test` · `npx tsc --noEmit` (even comment-only frontend changes pass through that file) · the two D6 lines · generated-output sync (the bindings must not change) · CI (`--locked` sees the new `Cargo.lock`).
- **QA = full (GUI)** — ADR-0270 「영향」 set 「슬롯 스폰 경로 교체 뒤 레이아웃 불변 · 셸 명령 등록(로그의 `refused` 로 「오늘 걸러 내는 이름 0」 measured)」 as a completion condition (§5-1).
- **Risks:** deleting the shell tests first leaves a regression-net gap (① · U1 comes first) · if `Cargo.lock` is left out, CI `--locked` turns red · the gate copies in CLAUDE.md · qa · testing-strategy drift apart from each other (fix them together in one unit) · the `agent-backend.md` and `.html` pair splits (same commit) · if the daemon message is read as a typo message, a caller tries to fix a typo that does not exist (U1 ㉡ measures this).

### 3-4. U3 — the refusal box

- **Acceptance criteria:** ① when the daemon answers registration or diff with `Error`, one box per place regardless of the code (`CONFLICT` · `INVALID_ARGUMENT` · **unknown code**) (GUI) ② the same (place, message) once per process · warn every time (label unchanged) ③ no box for shell-local failures (disconnect · not connected · send failure etc.) — the box follows the **source type** (§2-4), not the warn arm ④ the text received by existing `send_command` callers is byte-for-byte the same ⑤ the box does not block the connection task or the command path (`show` is asynchronous · outside the lock) ⑥ no `cfg` enters the shell and the capability is unchanged.
- **Tests (proposal):**
  - ㉠ source classification (`protocol_state.rs` · rewrite the old `reply_outcome_splits_ok_and_err` `:433-450`) — `Ok(Error{"CONFLICT: …"})` · `Ok(Error{"INVALID_ARGUMENT: …"})` · ★`Ok(Error{"BRAND_NEW_CODE: …"})` (unknown code)★ · `Ok(Error{"접두 없는 글"})` → all four `Daemon` · `Err(SENT_OUTCOME_UNKNOWN)` (call the constant by name) → `Local` · `Ok(Ack)` → `Ok` · `Display` is the same as the old text.
  - ㉡ suppression — the same (place, message) twice → once · only the place differs → twice · only the message differs → twice · no deadlock even if the double calls the filter again (call outside the lock).
  - ㉢ registration place (real socket round trip — the fake server of `daemon_client/tests.rs`) — four branches: **reply with `CONFLICT`** → the double receives `(Register, 문구)` exactly once, and it does not increase for the same reply after a reconnect · **`INVALID_ARGUMENT`** (e.g. the 「a command name may be at most 128 bytes …」 shape) → once · ★**unknown code** (`"BRAND_NEW_CODE: …"`) → once★ · **cut before the reply** (`SENT_OUTCOME_UNKNOWN`) → 0 times (warn is printed — O4). This branch also catches 「`:971` 에서 누가 다시 `Error` 를 `Err` 로 접는 회귀」 (§2-4 「대가와 다른 안」).
  - ㉣ `send_command` compatibility — the `send_command` tests of the existing `daemon_client/tests.rs` must be green without being fixed. ★However, the assertion of `send_command_resolves_err_on_matching_error` (`:2528-2542`) is today `contains("거부")`, so it cannot measure acceptance criterion ④ (byte-for-byte the same) → tighten it with a byte comparison★: since the text the fake server sends is `"데몬측 거부(테스트)"` (`ReplyBehavior::ErrorEcho` · `:2469-2472`), `assert_eq!(result.err().as_deref(), Some("데몬측 거부(테스트)"))`. (`assert_eq!(result, Err(…))` does not compile — `AgentEvent` does not derive `PartialEq`, `protocol/src/messages.rs:429`.) This tightening is done first, at U3 step ① — so that every step measures that the later steps did not change that text.
  - The diff place `view_bus.rs` is a Tauri adapter (「로직이 없다」 in that file's header), so ㉠ · ㉡ measure the decision and the GUI measures the one line of wiring.
- **Order inside the unit — every intermediate commit is correct in behavior, not only in compiling (review N2):**
  1. `CommandFailure` + `classify_reply` + the tightening of tests ㉠ · ㉣ — nobody calls them. `reply_outcome` and `:971` are unchanged.
  2. Add `send_command_with_origin` · `send_command` delegates to it — today the daemon `Error` enters the slot as `Err`, so the classification result is `Local`, and the text `send_command` returns is unchanged (㉣ measures it).
  3. Turn the registration outcome task (`connection.rs:1961-1976`) and `view_bus.rs:94-100` to the classification result — at this point a daemon refusal still comes as `Err` → `Local`, so the box branch does not stand, and warn is the same as now (behavior unchanged).
  4. ★Flip `:971` last★ (`Ok(ev)` unchanged · slot contract comments `:61-64` · `:183` · anchor) — from this moment a daemon refusal is classified as `Daemon`. Since the box port does not exist yet, `Daemon` also produces only warn.
  5. Box port · filter · operational adapter · `DaemonClient` field (four literals) · `events.rs:3-8` header · tests ㉡ · ㉢ — the box stands.
  - **Why flipping `:971` first is wrong:** in that commit `send_command` still returns the slot result as is (`mod.rs:827-830` — `Ok(result) => result`), so the daemon `Error` goes to the caller as `Ok(AgentEvent::Error{..})`. `expect_ack` (`:292-305`) and `expect_ack_or_spawned` (`:309-319`) of `commands/agent.rs` warn on `Ok(other)` and pass it on **as a success**, so a command the daemon refused looks like a success to the frontend, and the registration outcome task goes to the `Ok(Ok(_))` arm (`connection.rs:1963`) and prints 「셸 명령 등록 완료」. It compiles, but it is an intermediate commit whose behavior is wrong.
- **Gate:** `cargo fmt --check` · `cargo test -p engram-dashboard --test lib_unit` · `--test daemon_client_pending` (confirm the slot type is unchanged) · CI · GUI (§5-2). Each commit of steps 1~4 must also be green for `lib_unit` (including ㉣).
- **Risks:** if `blocking_show` is used, the connection task or the main thread stops (use only `show`) · calling the port while holding the lock deadlocks on re-entry from a double · adapter (ADR-0006) · whether the box comes to the front when the window is hidden (tray) (not verified — §5-2) · if `:971` is folded again, the box silently dies (㉢ catches it).

### 3-5. Verification per unit — regression count comparison

The commands are exactly those of 1-3 TRD §5-2 (run `cargo test --workspace -- --test-threads=4` before and after and compare the result-line count · passed · failed · ignored · if the line count drops a target was lost — stop even if everything is green).

| Unit | Result lines | Passed total | Distribution shift |
|---|---|---|---|
| U1 | unchanged (added to existing files) | + three new daemon tests (four if ㉣ is included) | — |
| U2 | unchanged | − 1 (`every_creation_door…`) + 1 (shell refusal pass-through) + 1 (agent ②) · `layout_commands` is a rewrite, so ± 0 | policy assertions shell → daemon (U1) · agent |
| U3 | unchanged | + `lib_unit` number of new tests (㉠ is a rewrite, so ± 0) | — |

**Measure the baseline right before starting U1** — the last record (CLAUDE.md 「빌드·검증 명령」 = 59 result lines · 3822 passed · 30 ignored, `cac9ee0`) has gone stale with the 1-1 · 1-3 merges. This TRD was not run.

### 3-6. U1 start checklist

0. **Two preconditions.** ① Record ADR-0279 (§7 — numbering · link · stamp = `/adr`). ② A point to return to — commit this TRD locally to create a starting point (code tree = `cf693f4`).
1. **Starting numbers** — run the §3-5 command and write down the result lines · passed · failed · ignored (if the terminal dies `scripts/run-detached.ps1` — qa binding 「분리 실행」).
2. **Files to touch (exactly):** daemon `src/connection_core.rs` — one import line (next to `:24` · `llm_creation_refusal`) · one struct field (`:1072-1092`) and its doc · one exception line (a stateless pure function pointer) in the 「공유 핸들만」 warning of the struct doc (`:1067-1071`) · filling it in `new` (literal `:1108-1119` — the only one) · `#[cfg(test)]` builder · helper (next to `spawn_command_by_cwd` `:654-661` · `// ADR-0270` · `// ADR-0279`) · handler (between `:1466-1469` and `:1470`) · tests (§3-2 ㉠~㉢ · optional ㉣ — next to `spawn_by_cwd_without_a_backend_is_refused` `:4822`) · agent `src/commands.rs` comments (`:593` · `:600-614` · `:629` · `:668-673` · `:1073-1077` — the §2-1 「agent 주석」 rule). The structure docs · frontend comments are U2.
3. **Order:** struct field + `new` fill (nobody uses it · build green) → helper + tests ㉠ · ㉢ → one handler line + ㉡ (+ ㉣) → agent comments.
4. **Gate · numbers:** the §3-2 gate → the §3-5 comparison (expected = result lines unchanged · passed + three or four).
5. `/review code full` → `/qa standard` → commit after the gate is green (`S21: <타입>(daemon): …` — the step number continues the step-log · the Co-Authored-By trailer at the end).

---

## 4. Dependency graph — after 2-1

| crate | direct workspace dependencies (normal · build), now | after 2-1 |
|---|---|---|
| Shell (`engram-dashboard`) | agent · base · command · discovery · net · protocol | base · command · discovery · net · protocol |
| Shell dev | + daemon (→ agent) | unchanged (2-4) |
| daemon | includes agent | unchanged — `llm_creation_refusal` sits next to `engram_dashboard_agent::commands` (`connection_core.rs:24`), which it already uses |

Gates that do not change: the two shell `test-support` leak gates (base · platform — `-p engram-dashboard -e normal,build,features -i …` → 0 lines) still stand as they are, since base · platform remain in the shell graph (base directly · platform via discovery) (the `-i` target does not disappear).

---

## 5. GUI measurement

The procedure (launch arguments · environment variables · PID · teardown) is held by the §full of the `/qa` binding. The app is launched with the `scripts/` launcher (it is not launched directly from the shell). Logs are turned on with `RUST_LOG=info` (the decisions below look at info lines).

### 5-1. U2 — slot spawn · registration

1. **Once before starting + once after U2** — the boot log has 「셸 명령 등록 완료」 (`connection.rs:1963`) and does not have 「웹뷰 명령 일부를 등록에서 뺐다」 (`view_bus.rs:53`) → 「오늘 걸러 내는 이름 0」 measured (the unverified item of ADR-0270 「근거」).
2. **Slot spawn** — `node scripts/cdp.mjs eval "window.__engramCmd.run('agent.spawnInto', { cwd: '<scratch>/ws-2-1', backend: 'claude' })"` · the same shape `'codex'` (needs the local codex CLI) → placed in a new tab · id returned. `backend: 'codx'` → 「를 모른다 … 스폰 안 함」 · tab count unchanged (compare before and after with `list_tabs` — the direct invoke of qa recipe B).
3. The policy-refusal branch cannot be reached by the GUI today (the table closes nothing) — U1 test ㉡ carries it.

### 5-2. U3 — how to raise the refusal box on purpose (without editing the source)

1. **Diff place · `CONFLICT`** — change the webview report via CDP so that it carries a name the daemon answers:
   ```bash
   node scripts/cdp.mjs eval "window.__TAURI__.core.invoke('report_view_commands', { commands: [{ name: 'agent.rename', help: { summary: '2-1 GUI 탐침', effect: 'write' } }] })"
   ```
   The report of the main window now takes this list → after U2 the shell does not filter `agent.rename` (D3) → `UpdateCommands` → daemon `CONFLICT` → **box ① (diff)**. Log = warn 「웹뷰 명령 차분 등록 실패」 + info (shown · Update). The reason `agent.rename` was chosen = it is a daemon name that really exists in the webview registry (ADR-0270 「맥락」), so it imitates the accident of 「누가 그 이름에 `help` 를 단 날」 as is.
2. **Registration place** — reconnect in the same session:
   ```bash
   node scripts/cdp.mjs eval "(async () => { const i = window.__TAURI__.core.invoke; await i('daemon_close'); await i('daemon_connect'); return 'ok' })()"
   ```
   The full registration of the new connection carries the `agent.rename` held by the bridge → the whole packet `CONFLICT` → **box ② (registration)**. Log = warn 「셸 명령 등록 거절」 + info (shown · Register).
3. **Suppression** — do 2 once more → warn is printed again and there is no box (info · held back).
4. **`INVALID_ARGUMENT` branch — raised cheaply (code reading · not verified)** — carry a name that is not a daemon name and is longer than 128 bytes:
   ```bash
   node scripts/cdp.mjs eval "window.__TAURI__.core.invoke('report_view_commands', { commands: [{ name: 'probe.' + 'x'.repeat(200), help: { summary: '2-1 GUI 탐침', effect: 'write' } }] })"
   ```
   The shell `report` does not look at the length (`view_commands.rs:436-481` — only marker · reservation · description) → since it is not a daemon name, it passes `refuse_names_i_answer` → the registry measures the length first (`roster.rs:182-184` · registration also `:127`) → `INVALID_ARGUMENT: a command name may be at most 128 bytes …` → **box ③ (diff · new key)**. Then 2 → **box ④ (registration)**. If this branch does not stand in the GUI, only the `INVALID_ARGUMENT` branch of test ㉢ carries it (test only). The unknown-code branch is not raised by a real daemon, so it is test only (㉠ · ㉢).
5. **No box for local failures (code reading · not verified)** — `node scripts/cdp.mjs eval "(async () => { const i = window.__TAURI__.core.invoke; await i('daemon_close'); await i('report_view_commands', { commands: [{ name: 'probe.local', help: { summary: '2-1 GUI 탐침', effect: 'write' } }] }); return 'ok' })()"` → the diff ends as not connected (`Local`) → warn 「웹뷰 명령 차분 등록 실패」 only · no box · no info either. Then `daemon_connect`.
6. **Recovery** — after reloading the window (the webview reports the real list again), 2 → 「셸 명령 등록 완료」.
7. **Observing the box** — `cdp.mjs shot` captures only the webview (qa recipe A). The OS window is captured by taking the whole screen with PowerShell (`System.Drawing` `CopyFromScreen`) or by finding the top-level window by its title (UI Automation) — neither has a precedent in this repository (not verified). The box decision itself is counted by the info log of the filter (§2-4).
8. With the window hidden in the tray, do 1 once more (with a different name — e.g. `agent.interrupt`) → see whether the box comes to the front (an unverified item).

---

## 6. Document follow-ups (this TRD does not fix them — each unit does)

1. **CLAUDE.md** — the two D6 lines in 「빌드·검증 명령」 (U2) · 「agent crate 를 의존하지 않는다(ADR-0270 · 게이트 = 「빌드·검증 명령」)」 in the src-tauri entry of 「백엔드 모듈 맵」 (U2). Since it is load-bearing, attach the `/review doc` lens too.
2. **`.claude/skill-bindings/qa.md`** — a new entry in standard (D6 · decision rule · `-i` trap · reason for excluding dev) (U2).
3. **`docs/testing-strategy.md`** §1 src-tauri section — one gate line (canonical = qa) (U2).
4. **`docs/reference/structure/agent-backend.md`** (`:98` · `:107` · `:209` · `:217`) **and its pair `agent-backend.html`** (`:605` · `:616`) — the list of policy doors (shell `gate_backend` → daemon `SpawnByCwd` · does not distinguish callers) (U2 · same commit).
5. **Code comments** — agent `commands.rs:593` · `:600-614` · `:629` (+ 「`refusal` 을 `Some` 으로 바꾸기 전에 ADR-0279 재론」) · `:668-673` · `:1073-1077` (U1 — the shell-door line is deleted by U2) · daemon `connection_core.rs:1067-1071` struct doc (U1) · shell `daemon_client/connection.rs:61-64` · `:183` slot contract · `:971` anchor (U3) · shell `apply.rs:531-556` · `view_commands.rs:8-15` · `:196-203` · `tests/layout_apply.rs:1308-1313` · `tests/layout_commands.rs:629-630` · `src-tauri/Cargo.toml:64-71` · `:79-82` · frontend `src/commands/agentCommands.ts:46-51` (U2) · shell `daemon_client/events.rs:3-8` header (one line for the sibling port `refusal.rs` — U3).
6. **Memo `docs/refactoring/architecture-discussion-2026-09-26.md`** — the 「순서: 슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼 뒤」 of §3 decision candidate 5 and the 「① 슬롯 스폰을 `agent.new` 경로로 → ② 셸 검사 제거 순서 엄수」 of §10 2-1 become stale once ADR-0279 stands (orchestrator) · marking the handling of the second line of §11 (shell `Cargo.toml` comment) (after U2).
7. **Date-stamped snapshot** — `docs/reference/architecture-map-notes.md:134` · `:209` (`gate_backend`) when that map is re-extracted.
8. **step-log** — the landing entry (commit · user decision · link to this TRD) is the orchestrator's.

---

## 7. What to record in ADR-0279

> Numbering · `Amends` link · the stamp on the ADR being amended = `/adr`. Only the rejected alternatives that main gave are carried over (CLAUDE.md 「결정 날조 금지」). ★`docs/decisions/` is being written by other work at the same time, so this TRD did not touch it★.

1. **Partial amendment of ADR-0270 (D1)** — the text being amended:
   - Decision 3.1 「슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼다」 → 「데몬 `SpawnByCwd` 처리부가 스폰 전에 정책 표를 본다 · 슬롯 스폰 경로는 그대로」.
   - The 「(ADR-0219 를 고친다 — …) 정책을 보는 문은 데몬 `agent.new` 하나가 된다」 of 「영향」 → **the two doors of the daemon** — `agent.new` (`verb_new`) · the `SpawnByCwd` handler.
   - The code anchor 「슬롯 스폰의 `agent.new` 경로」 of 「영향」 and the GUI completion condition 「슬롯 스폰 경로 교체 뒤 레이아웃 불변」 are also read in this sense.
   - The order of decisions 3.2 · 3.3 (policy wall → delete the shell check → reserved list) stays, the 「셸의 정책 검사(`gate_backend`)를 지운다」 of 3.2 is the policy part, and the word net (`parse_backend`) remains (D2).
   - Rejection = §2-1 (a) · (b) · reason = §2-1. In the rationale, write in one line the 「바로잡은 사실」 at the end of §2-1 (`SpawnByCwd` also registers · saves an on-the-fly profile · the spelling is matched by the command table) — so that the same reason does not come up again.
   - ★**Accepted cost (감수한 대가)**★ — because this door does not distinguish callers, the human path (`slot.createAgentHere` · frontend `agent.spawn` — the claude-pinned door of ADR-0219 decision 5) also consults the 「LLM 제어 표면」 policy · the frontend `agent.spawn` and the agent bus `agent.spawn` diverge on that point · the frontend `agent.spawnInto` was already like that since the shell door · zero impact today. **Trigger for reconsideration = the day any backend is closed to the LLM or its declaration is missing — add a caller axis to the spawn packet.** Link it to ADR-0219 decision 5.
2. **ADR-0219 「영향」 link (D7)** — the door that consults the policy ② = shell `gate_backend` → daemon `SpawnByCwd` (does not distinguish callers). Of the three measuring places, `src-tauri/tests/layout_apply.rs::every_creation_door_reads_one_backend_policy` → the daemon · agent tests (§2-5). Of the claude-pinned doors of decision 5, the frontend `agent.spawn` now consults the policy (「감수한 대가」 in 1 above). The body is not edited.
3. **Fill in the 「구현 때 고른다」 of ADR-0270 decision 4 (D4 — not an amendment)** — tauri-plugin-dialog Rust API · a new port that does not widen `DaemonEvents` · two places · trigger = every case where the daemon answered registration or diff with `Error` (code-independent · including unknown codes) · shell-local failures excluded · ★the distinction is the source type at the reply boundary — string codes are not read (the reason = adding a code is a permitted extension)★ · repeat-suppression key (place, message) · the lifetime of the shell process and its reason (§2-4).
4. **The mechanical wall for the 「영향」 invariant of ADR-0270 (D6)** — gate shape · the reason `-i` is not used · the reason dev is left out.

---

## 8. Open items — handling record

There were seven raised by the 1st edition. O1~O5 were handled by main on 2026-10-06 (front matter 「결정 출처」), and O6 · O7 were accepted as written. The review fixes of the 3rd edition are listed in front-matter 「개정」.

- **O1 — the D1 rejection reason 「슬롯 즉석 에이전트가 영속 프로필이 된다」 → deleted.** It does not stand on the code. The corrected fact is written in only one place, at the end of §2-1, and the ADR-0279 rationale also carries it (§7 1).
- **O2 — 「`SpawnByCwd` 는 명부 상한에 닿지 않는다」 → corrected** (§1-5 — `manager.rs:1148-1151`).
- **O3 — the D4 trigger → widened.** If the daemon answers with `Error`, the box is shown regardless of the code, and only shell-local failures are excluded. In the 3rd edition the distinction was changed to the source type (§2-4 · §3-4 · §5-2 4 · 5).
- **O4 — the 「거절」 warn at the registration place is also printed on a disconnect (pre-existing) → left as is.** The label is not fixed, and the box follows the source type, not the warn arm (§2-4 · §3-4 acceptance criterion ③).
- **O5 — frontend `agentlist.createCodex` → `CreateProfile` does not go through the LLM backend policy (out of scope · zero impact today) → handed to memo §11** (that line of `docs/refactoring/architecture-discussion-2026-09-26.md` §11 — unhandled). The 「문 ③」 text of agent `commands.rs:600-614` is corrected to match the facts when U1 passes through that comment (§2-1 「agent 주석」).
- **O6 — the `AgentEvent::Error` arm of `DaemonSpawner` is dead code (certain)** — §1-2. Since the daemon message goes out without the prefix 「spawn 실패:」, the D1 message has to stand on its own (this is why §2-1 chose the frame of the current shell message). Whether to delete the arm is a separate matter. Even after U3, `send_command` returns the daemon `Error` as `Err`, so it is still a dead arm.
- **O7 — ADR-0270 decision 3 listed three units (path replacement · shell check · reserved list), but U2 of this TRD bundles the latter two into one unit.** The order (3.2 → 3.3) is kept inside U2 and each step makes the build stand — I read it as a bundling, not an amendment.

---

## 9. Unverified

- **Baseline regression count** — not run in this edition (§3-5 · right before U1).
- **The real box** — the shape of the window rfd shows on Windows · whether it comes to the front when the window is hidden (tray) (§5-2 8).
- **A means of capturing an OS window** (§5-2 7) — there is no precedent in this repository.
- **Whether the full registration of a new connection goes out as `daemon_close` → `daemon_connect`** (§5-2 2) — code reading (`register_own_commands` is called after every connection — `connection.rs:943`) and not run.
- **GUI `INVALID_ARGUMENT` · local-failure branches** (§5-2 4 · 5) — code reading. If the former does not stand in the GUI, only test ㉢ carries it.
- **Whether agent declarations are visible in the `layout_commands` binary after 2-1** (§2-5) — uncertain · unmeasured. Since that binary calls the daemon (`:3033`), the daemon is linked — whether the agent declarations come along is what splits. The 「전」 measurement is taken once before U2 ① (or at the start of U1) and the 「후」 measurement after U2 ⑤ (to measure 「후」 in this edition, the code would have to be changed, so it was not done).
- **Real behavior of the policy-refusal branch** — it cannot be measured at runtime since the table closes nothing (only by the seam test — U1 ㉡).
- **「오늘 걸러 내는 이름 0」** — code reading (§5-1 1 measures it).
- **codex real-spawn GUI** (§5-1 2) — the local codex CLI is required.
