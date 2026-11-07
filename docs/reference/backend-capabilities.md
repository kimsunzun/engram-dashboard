# Backend capability comparison table

This gathers in one place **what the agent CLIs we attach actually give us**. The subject is not code but **outside facts** — our structure is in `structure/agent-backend.md`; this document records the **counterpart's contract** that structure relies on.

**How to use it — before attaching a feature, fill in that row of this table first.** If it is empty, research first, write the result here, and only then go to code. If you look things up as needed and throw them away, the next session digs up the same thing again (it actually happened: `clientUserMessageId` was not found, so ADR-0193 was rejected on 「합칠 키가 없다」, yet that field existed even then).

## Rules of this table

- ★**Leave a cell you do not know empty and write `미확인` (unverified; the cells of this translation say "unverified") — do not fill it by guessing.**★ An empty cell is the list of the next research targets.
- ★**Write the version with it.**★ What is written here is **a fact observed on that version** and it goes stale when the vendor changes it. A row with no version cannot be trusted. ★**A derived version gets `추정` (estimated), to tell it apart from a measured value**★ — what lands here is a version pulled from the records before and after because that round's record has no version string. When it is measured again, drop `추정` and write the measured value.
- **Attach an evidence grade:** `소스` (source) = read the vendor source/schema directly · `실측` (measured) = we actually ran it · `문서` (docs) = vendor documentation · `관측` (observed) = the value merely looked like that.
  ★**`문서` (docs) is the weakest grade**★ — codex's public documentation uses `"thr_123"` as the thread id example, but **the real server rejects that value** (measured). When documentation and source disagree, trust the source.
- Whether we implement it on our side is not written here — that is for the code and the TRD. This table writes **only what the counterpart gives**.

**Last updated:** 2026-09-21 · **baseline versions** = **claude** 2.1.170 · **codex** 0.154.0 — ★**the range this baseline covers is §1 and §2**★ (restored what that round wrote down · 2026-09-22 — **it is a record that was written down, not an inference drawn from the absence of a notation.** So `추정` is not attached). The lock and `originator` rows of §2.5 were measured on 0.155.x, so a version is attached at that place. ★**This attribution applies to those two sections as of then**★ — a row added later must attach its own version itself.

★**Do not keep counting the section list here**★ — if we add 「어느 절이 어느 버전인가」 here every time sections grow, it falls behind even when only one row is re-measured (for the same reason we do not count the gates and instead point to the crate header). **For a row with a version attached, that notation is canonical.** ★**Do not read a row without a version as 「그러니 기준 버전에서 쟀다」 — the absence of a notation is not evidence**★. Unless it is a section the header above **attributed by pinning down the range** (§1·§2), the version is unknown, and by the rule above it cannot be trusted — to fill it, you have to trace back that row's source record. ★**The line is drawn here — a statement that says 「이 절을 이 버전에서 쟀다」 is a record, and a general inference drawn from the absence of a notation is not a record**★. The old sentence that had been written as the latter (「버전을 안 단 줄은 기준 버전에서 잰 것이다」) was discarded (2026-09-22 · that sentence made us assert that the three version-less rows of §2.5 codex were **measured on 0.154.0**, but that round was 0.155.x).

★**The claude-side baseline version has already diverged from the installed one**★ — `docs/process/step-log.md:2310` writes the installed one as **2.1.269** and in that same place nails down 「우리 근거 버전은 2.1.170, 드리프트」 (`claude --version` gives the same value on 2026-09-22, when this document is being fixed). So **do not read the claude column as a fact about the currently installed one** — until it is measured again, the baseline version is only the name of 「그때 그 관측」.

---

## 1. Session identity and restore

| | claude | codex (app-server) |
|---|---|---|
| **Who issues the id** | **We do** — we draw it and force it with `--session-id <uuid>` (source) | **codex** — received as `thread.id` in the **response** to `thread/start` (source · measured) |
| **id format** (id 형식) | the UUID we drew | **UUID** — the type is a `uuid::Uuid` newtype and deserialization calls `Uuid::parse_str`. Generation is `Uuid::now_v7()`. **A non-UUID is rejected by the server** (source · measured). ★But in the JSON schema it comes out as a plain `string` — not because of the generator but because **that type implements the schema directly as an unconstrained String**★. And the vendor comment limits it to 「**codex 가 발급한** id 는 UUIDv7」 — it does not guarantee future hosted · import paths |
| **How to attach to an existing one** | spawn **argument** `--resume <uuid>` (source) | **after** spawning, the first request after the handshake is `thread/resume { threadId }` instead of `thread/start` (source) |
| **If given an unknown id** | **Fails honestly** — `No conversation found …` + exit code ≠ 0. It does not quietly open a new conversation (measured = `docs/process/S9-session-restore/spike-results.md` · ADR-0082 · the regression net reproduces that wording and the exit code). ★**The exact opposite of terminal-mode codex**★ — that one reinterprets an unknown value as a session name and opens a new conversation with exit code `0` (§2.5 codex) | **Error** `-32600` `no rollout found for thread id` — it does not quietly open a new conversation (measured). ★**Do not use `-32600` as a 「모르는 스레드」 signal**★ — other invalid requests, such as a config loading failure, have the same code. You have to look at the message to tell them apart. ★**Terminal mode's `codex resume <값>` is the exact opposite of this — see §2.5 codex**★ |
| **If the cwd differs** | unverified | **It resumes.** Identity is per `CODEX_HOME` (`~/.codex`), not per workspace. The `cwd` in the response is **the original cwd recorded on the thread** (measured) |
| **Time expiry** | unverified | **None** — a thread from 3 months and 13 minor versions earlier resumed as is (measured). The often-cited 30 minutes is not an expiry but a **memory-unload grace period** (docs) |
| **Listing sessions** | unverified (scanning the `~/.claude/projects/**/*.jsonl` files is the common workaround) | **There is — `thread/list`.** Cursor pagination + filters such as `cwd` · `archived` · `searchTerm` (source · measured) |
| **Recovery when the id is lost** | unverified | **Only partially.** Adoption through `thread/list` → `thread/resume` works, and codex itself uses that pair for daemon recovery (source). ★**But the crash window cannot be filled by this**★ — see below. ★**If the process is running, you can ask the OS and find out**★ — the holder of the writer lock is that process (§2.5 codex) |
| **Disk location** | `~/.claude/projects/<경로>/<uuid>.jsonl` (observed) | `$CODEX_HOME/sessions/<YYYY>/<MM>/<DD>/rollout-<타임스탬프>-<uuid>.jsonl`. The first line, `session_meta`, is authoritative (measured) |
| **Recovering the id from the file name** | ★**There are broken cases**★ — recent claude's transcript file name UUID is **different** from the session_id the hook reports (pinned in an orca comment) | Mostly works, but **`thread/revert` keeps the thread id and creates a new file with a different rollout id** — the authority is `session_meta.id`, not the file name (source) |

**So the risk only codex has (codex 에만 있는 위험):** the id is only in hand once it **arrives in a response**, so if the process dies after receiving it and before writing it to disk, that thread is lost. ★The id side was narrowed to persisting before the first submit (2026-09-24 · ADR-0226), but with codex it was reopened by one step★ — on the codex app-server path the id is persisted **when the counterpart echoes the first user message** (user decision 2026-09-26 · ADR-0233 — amends ADR-0226 · code = `witness_first_turn` of the channel). So **if it dies after sending the first turn and before that echo arrives, no id remains.** For the codex terminal, persisting before the first turn does not hold in the first place (ADR-0226 decision 11). There is still no durability for the message content. claude has no such window, because we draw the id.

★**End-to-end measurement (2026-09-15) — the source reading of the 「id 형식」 row above was confirmed on our path**★. We launched a real app-server through the GUI and ran only the handshake (turn 0 · model usage 0 — `thread/start` comes before a turn):

- The id received = `01a0a08f-9247-7612-9b1f-dadc61693dee` — version nibble `7`, i.e. **UUIDv7 really does arrive, as the source reading said.** It is the 36-character canonical form with no decoration such as `urn:` or braces.
- That value sat in `backend_session_id`, went down into `agents.json` and **remained after kill.** ★That is the fact at the time — after D1 (2026-09-24 · ADR-0226) a 0-turn session is not persisted★: the received id stays in the first-submit latch and is persisted afterwards — for that point and the window that remains now, see §1 (「codex 에만 있는 위험」 above).
- The control group = **a Terminal-mode profile stays `null`**. Without the app-server there is no id — the two modes really do diverge.
- ★**So it was also confirmed that there is no reason to widen the storage field `Option<Uuid>` to a string**★ (it was once a worry).
- ★**But inside our registry the UUID version differs per backend**★ — for claude we draw v4 and for codex we receive v7. Every test in the repository mints with `Uuid::new_v4()`, so **no test meets that difference.** Do not add version validation — `Uuid::parse_str` does not look at the version, and so it accepts both.
- **What this measurement does not cover:** we did not look at the wire bytes themselves (there is no place that prints the JSON-RPC response body — what we saw is two things: our parsing result and the same value codex printed to its own stderr). And it was reconfirmed here that `thread/start` **does not create a rollout file** — the number of files in `~/.codex/sessions/` stayed the same throughout the measurement (one more ground for the crash-window item below).

★**And that window cannot be filled with `thread/list` — an item the adversarial verification overturned**★: codex 0.154.0's own tests assert that **a new thread has no rollout until the first user message arrives**. That is, if it dies right after `thread/start`, the id was issued but there is nothing on disk, so `thread/list` does not catch it and `thread/resume` cannot find it. **Adoption is a means of recovering 「이미 대화가 오간 스레드」, not the solution to the crash window.** A candidate solution is **to write it down on our side first, before sending** (record 「이 프로필이 스레드를 시작하려 한다」 even before receiving the id) — not designed yet.

**Two added limits:** `thread/list` gives its results with archived ones and some source kinds **left out** under the default filter — it is not a complete registry. And `archive` blocks resume until it is undone, while `thread/delete` is a permanent delete.

## 2. Input and echo

| | claude | codex (app-server) |
|---|---|---|
| **Does it echo the input we sent** | By default it does not. You have to pass **`--replay-user-messages`** for it to echo (source) ★both `--input-format` and `--output-format` must be `stream-json`★ | **It does** — the `userMessage` of `item/*` (source · measured) |
| **A field to carry our id** | There is — the uuid we put on the stdin user line is echoed back as is (source) | ★**There is — `clientUserMessageId` of `turn/start`**★. The echoed item comes with that value as `clientId`. **It is echoed on the `turn/start` path too** — the 0.154.0 integration test asserts it (source). **From codex 0.140.0**; 0.135.0 does not have it ★the public documentation does not have this field, so reading only the documentation gives 「없다」★ |
| **If we send twice with the same id** | unverified | ★**It does not deduplicate**★ — sending twice with the same `clientUserMessageId` **may reach the model twice, yet on screen they are merged and look like one line**. This id is **a correlation key, not an idempotency key** (source · vendor issue). Do not lay this down as a premise when building the resend · reconnect paths |
| **Is the echoed content the original verbatim** | unverified | ★**No — it normalizes**★. So codex's own TUI, when the match succeeds, discards the server projection and renders **the local copy** (source) |
| **What the vendor's own client does** | — | If no turn is running, it renders locally **before sending** and ignores the echo by content equality. If a turn is running (steer), it **does not** render locally, waits, and renders the local copy when the echo arrives. The matching key is `clientId` first, and without it the fallback 「본문 + 이미지 개수」 (source) |
| **Old-version fallback** | — | A content-comparison key. ★Two sends that differ only in audio · mention **become the same key**★ (vendor TODO comment) |

## 2.5. Launch arguments and modes (measurements we already knew — the scattered ones are gathered here)

### claude

| Fact | Basis |
|---|---|
| JSON mode is **`-p` only** — used together with `--input-format stream-json` · `--output-format stream-json` | source · `--help` writes "only works with --print" |
| ★**It dies instantly without `--verbose`**★ — `--help` has no such wording, but the runtime kills it with "When using --print, --output-format=stream-json requires --verbose". The symptom is **the agent vanishing right after spawn** | measured 2026-07-02 (claude 2.1.170) |
| Headless stream-json also supports `--resume <sid>` — it coexists with the `-p` family and resumes a past conversation losslessly with no "session already in use" | measured 2026-07-13 (claude 2.1.170) |
| ★**A `thinking` block appears only when the env `MAX_THINKING_TOKENS` is set**★ — the CLI default is off | measured 2026-07-06 (ADR-0049) |
| The working root is set by `--cd`. The env actually uses the state directory we pass | measured (the contract test bench measures it with a real process) |
| The session id **changes midway** — it forks when the user does `/clear`. That is why file-polling watching is needed | source · observed |
| Supports a control channel | source |

### codex

| Fact | Basis |
|---|---|
| **Terminal mode has threads too — what is missing is a channel that tells us about them.** Attaching works with the `codex resume <thread id>` subcommand (measured 0.155.0 — the same id is preserved and the record is appended). Echo is not applicable | source · measured |
| ★**You can ask the OS for the id of a running thread**★ — the codex process runs **holding open** `$CODEX_HOME/thread-writer-locks/<thread id>.lock`. The holder is `codex.exe` itself, not a wrapper, and the file is 0 bytes, so only the name is information. We measured it appearing 0.735 · 0.879 · 0.907 · 1.891 seconds after launch (plus one 21-second case that did not reproduce), which is **before the first turn** | measured 2026-09-21 (0.155.x estimated) |
| **A dead lock and a live lock can be told apart** — after killing the owning process, asking about the same file gives holder 0. codex itself also sweeps away dead locks at launch. Administrator rights are not needed | measured 2026-09-21 (0.155.x estimated) |
| **`CODEX_HOME` applies only to the process it is set on** — the lock folder above also moves along under that home. The lock folder's name · layout is **not documented** — if upstream changes it, we get not an error but silently 0 candidates | measured 2026-09-21 (0.155.x estimated) |
| ★**The record side has no PID**★ — none of the rollout JSONL · the 40 columns of `state_5.threads` · `sandbox.*.log` has pid · ppid · tty · host · command line · environment variables (exhaustive sweep). From the disk records alone, you cannot build 「이 스레드가 우리 자식의 것인가」 | measured 2026-09-21 (0.155.x estimated) |
| **`CODEX_INTERNAL_ORIGINATOR_OVERRIDE`** — an internal variable upstream does not document. The value we set stays as is in the `originator` field of the session record (3 arbitrary strings · no allowlist rejection). If it goes away, it is silently ignored, not an error | measured (0.155.1) |
| **Text passed as `developer_instructions` stays verbatim in the rollout** — but does not go to the sqlite side | measured 2026-09-21 (0.155.x estimated) |
| ★**`codex resume <값>` does not fail even when wrong**★ — given a string it cannot read as a thread id, it reinterprets it as a **session name**, **quietly creates a new session**, ends with exit code `0`, and the turn is billed. It is **the opposite in nature** to the app-server's `thread/resume` (unknown id → `-32600` error), so the two modes must not be handled by the same rule | measured 2026-09-21 (0.155.x estimated) |
| `thread/start` parameters = all optional (no `threadId`) — `approvalPolicy` · `approvalsReviewer` · `baseInstructions` · `config` · `cwd` · `developerInstructions` · `ephemeral` · `model` · `modelProvider` · `personality` · `sandbox` · `serviceName` · `serviceTier` · `sessionStartSource` · `threadSource`. ★`input` · `clientUserMessageId` are not here★ — the old edition of this row wrote the fields of `turn/start` here (our structs also keep the two apart — `ThreadStartParams` · `TurnStartParams` of `backend/codex/protocol.rs`) | source (0.156.1 schema) |
| `turn/start` parameters = required `threadId` · `input` + optional `clientUserMessageId` · `turnTrigger` · `toolOutput` · `cwd` (per-turn override) etc. | source (0.156.1 schema) |
| `thread/resume` requires only `threadId`. Many optional arguments — `cwd` · `model` · `sandbox` · `approvalPolicy` · `excludeTurns` · `initialTurnsPage` · `personality` etc. | source |
| `excludeTurns: true` = does not fill `thread.turns` and gives only metadata and the live-resume state. **Full-history hydration is deprecated**, and the schema advises using it together with `thread/turns/list` · `thread/items/list` | source |
| Priority rule: for a non-running thread, `history` > a non-empty `path` > `threadId`. If `threadId` points to a thread that **is running**, it rejoins there and `path` is used only for a match check | source |
| **A sub-agent thread is refused a standalone resume** — you must resume the parent first or peek with `thread/read` | source |
| There are 70-odd `thread/*` methods (`thread/read` · `thread/turns/list` · `thread/items/list` · `thread/search` · `thread/fork` · `thread/archive` · `thread/delete` …) | source |
| The `history` · `path` arguments are marked **UNSTABLE** (`history` is Codex Cloud only; the schema writes "DO NOT USE") | source |

★**The 「소스」 rows of the table above can be checked against local**★ — `codex app-server generate-json-schema` extracts the schema of that installed version (the `thread/start` · `turn/start` rows were checked against 0.156.1 that way — that check caught the old edition in which the two parameter lists were mixed up).

★**The `(0.155.x 추정)` (shown as "(0.155.x estimated)") in the table above is not a version that round wrote down**★ — **the ADR-0218 round's record has no version string** (the full text of `docs/decisions/0218-…` · `docs/process/step-log.md:2445` and below). The records before and after write the installed one as `0.155.1`, so we slotted it in between — the day before, ADR-0216 confirmed with `codex --version`, and **ADR-0217, which stood first the same day,** writes its own measurement as `codex-cli 0.155.1` (how that round switched from 0217 to 0218 = step-log:2447). Asking again on 2026-09-22, when this document is being fixed, still gives `codex-cli 0.155.1`. ★**Even so, 「그 라운드가 0.155.1 이었다」 is not a measurement but an inference bridging the observations on both sides**★ — when it is measured again, drop `추정` and write the measured value.

### gemini

We have never attached it. There is neither a command variant nor an item in the divergence table, so it is **unreachable**, and there is not a single observation to write in this document.

## 3. Approval and permissions

| | claude | codex (app-server) |
|---|---|---|
| **Do approval requests arrive** | We unconditionally inject `--permission-mode bypassPermissions` so they do not arrive (source · ADR-0097) | We pass `approvalPolicy` · `sandbox` to `thread/start` · `thread/resume`. **We have never run the path on which a real request arrives** — unverified |

## 3.5. Traps to step on when attaching (left as comments by peer implementations)

- ★**A codex item arrives twice**★ — the same item arrives twice as `item/started` and `item/completed`. Rendering without deduplication shows two copies (the peer implementation has a guard for it).
- ★**The `cwd` of a `thread/list` row is optional**★ — treating a row without it as 「cwd 가 같다」 matches the wrong thread. When filtering by cwd, **discard rows without it** and set the list limit generously (most rows have a different cwd, so the default limit cannot find it).
- **`thread.sessionId` and `thread.id` are observed to be the same value** — we have not confirmed a guarantee that the two are always the same. Do not use one as a stand-in for the other (observed).
- **The transcript file name UUID of claude can differ from the session id** — code that assembles a file path from the id breaks. If you need the path, carry the path separately (an actual regression in a peer implementation).
- **Replacing a file atomically is different from durability** — a rename is atomic to the reader, but without fsync a power cut leaves the old content or an empty file (peer implementation comment).

## 4. Unverified — the next research targets

- **Many cells on the claude side** — every row marked `미확인` (unverified) in the tables above. We have used claude for a long time but have never sorted out these axes.
- ★**How to fill the crash window — there is no design**★. Only this was settled: adoption is not the answer (§1). ★The id side is not fully closed either★ — for the window that remains see §1 (「codex 에만 있는 위험」). There is still no durability for the message content. Candidate = a durable record written on our side first, before sending (thread id · our message id · the content sent · the state as one unit).
- **Recovery from an ambiguous disconnect** — the span in which codex received the request but we did not get the response (or died before writing it down). Interlocked with 「멱등 아님」 above, **resending is not safe**.
- **How an archived thread** looks when we meet it — we only know that resume is blocked.
- **What happens if two open the same thread at the same time** — the ownership rule is unverified.
- Behavior **when the rollout file is damaged or missing** is unverified.
- **If `CODEX_HOME` differs after a restart**, attaching fails altogether — we have not checked whether we pin that value.
- **Is there a case where one codex process holds two or more writer locks** — unmeasured. Until this value is settled as 「없다」, the side that picks a thread by lock holder must reject that case.
- **Does `codex exec` create a writer lock** — unmeasured (our path is the TUI, so it is irrelevant for now).
- **Does a hosted/cloud backend give a non-UUID thread id** — the local app-server rejects non-UUIDs, but the vendor comment limits it to 「codex 가 발급한 id 는 UUIDv7」.
- **Rollout file cleanup (GC) · retention policy** — 「만료 없음」 is a lower bound based on one success.

## Research method notes (so the next person does it cheaply)

- ★**With a vendor CLI, first look in `--help` for a schema-export subcommand**★ — codex has `codex app-server generate-json-schema` · `generate-ts`, which extract **the spec of the very version we use**. It is more accurate than GitHub main.
- ★**For 「이 백엔드에 X API 가 있나」, grep a client clone that uses it before the API docs**★ — not just existence but the real parameters and traps come out together. Clone location = `../../../Engram_Workspace/opensource/` (outside the repo · untracked).
- Web search had a low yield in this class — it has almost never given decisive evidence beyond issue titles.

## Sources

The vendor sources are `codex-rs/protocol/src/thread_id.rs` · `codex-rs/app-server-protocol/src/protocol/v2/thread.rs` · `codex-rs/app-server/README.md` · `codex-rs/tui/src/chatwidget/` of `openai/codex`. The public documentation is `learn.chatgpt.com/docs/app-server` (★its examples differ from reality★). The peer implementations are `paseo` (real use of the codex app-server) and `orca` (echo matching · durability comments) in the clone above.
