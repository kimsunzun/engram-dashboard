# ADR-0109: Group v1 management semantics and dual-entrance contract — anyone can edit · implicit creation · @ reserved · single point of normalization · --settings file injection

- 상태: 확정 (2026-07-27, basis: 5 user decisions (2026-07-26~27) + real findings from the D adversarial review) · 부분 폐기 by ADR-0111 (Decisions 1~5, group management semantics) · 부분 폐기 by ADR-0328 (the settings fragment part of Decision 9)
- 관련: ADR-0086 (dual entrance) · ADR-0094 (grants least privilege — kept unextended) · ADR-0099 (mcp-config file lifecycle — settings rides along) · ADR-0101 (WYSIWYA name addressing) · ADR-0103 (original decision on §6 entrance and fragments — this ADR makes it concrete as the D-stage contract) · spec §4 · §6 · Amended by ADR-0111 (Decisions 1~5, group management semantics) · Amended by ADR-0328 (the settings fragment part of Decision 9)

## 맥락

While implementing the `group` (roster management) and `messages` (pending query) tools, their CLI mirrors, and the spawn-time `--settings` injection, D (entrance completion) had to settle the policy and contract forks that were still open. The review (2 adversarial reviewers + 7 blind rounds) found real contract divergences caused by the split entrance (normalization asymmetry, error vocabulary divergence).

## 결정

**Group v1 semantics (user decisions 2026-07-26~27):**
1. **Anyone can edit** — anyone creates, edits and deletes any group. No ACL and no record of the acting party (to be done all at once when records and the permission system are upgraded — step-log backlog).
2. **Silently** — no notification on roster change or deletion (v1 default-only principle).
3. **Roster changes and deletions have no effect on already-parked broadcasts** — a consequence of the snapshot-at-send-moment principle (a recall feature is not adopted).
4. **`@` = a namespace for groups only** — a group name must have `@` (otherwise `INVALID_GROUP_NAME`), a member name must not contain `@` (`INVALID_MEMBER_NAME` — blocks becoming a literal ghost member, since nested groups are unsupported), the `@all` built-in is protected (`GROUP_BUILTIN`).
5. **Implicit creation** — there is no create verb: add to a nonexistent group creates it; remove and query do not create. No downgrade to an empty add (`INVALID_GROUP_ARGS`).

**Entrance and contract (implementation decisions — reflecting real review findings):**
6. **Semantic validation and normalization = the daemon single point.** Member-name comma splitting, trimming and whitespace removal happen only in the shared handler (ingress) — the CLI forwards the raw text. The two entrances produce byte-identical JSON (pinned by a parity integration test). Batch changes are applied all at once after full validation (no partial application).
7. **`messages` contract** — an id query returns one row per recipient + `may_be_truncated` when short against `expected_rows` (the expected row count recorded at send time — the position-based "proof" was discarded because the two-stage group record disproved it). With no argument it returns the caller's three pending axes (direction tag), **reply obligation matches by id first** (blocks misattribution to same-named senders), and **the obligation follows whoever actually received the envelope** (re-bound at delivery time). Times are relative seconds (the ledger holds only a monotonic clock). No ownership restriction; the body is not returned.
8. **CLI identity = ENGRAM_TOKEN only** — `--as` cannot be spoofed (derived server-side as BoundIdentity). Exit code: a 2xx response that is `status=="error"` can never be 0 in any form (shared by send and query).
9. **`--settings` = file-path injection** (not inline JSON) — excluded because quote collapse in Windows `cmd /c` wrapping is a silent failure mode. The fragment keeps the single key `{"allowedMcpServers":[{"serverName":"engram"}]}` (its priority is high, so it must not become a junk drawer — pinned by a regression test), and rides along on the mcp-config directory, sweep and revoke lifecycle (ADR-0099). A write failure is warn + continue (unlike mcp-config, a missing fragment = degradation, not a block).
10. **grants (ADR-0094) unextended** — `messages`/`group` are not sending entrances, so they are not added to the `--allowedTools` list. There is no effect under the current spawn permission mode, and a user decision is needed when a constraint layer is introduced (flagged in a code comment).

## 거부한 대안

- **Including ACL / acting-party records in v1** — deferred by user decision (no stopgap patches — do it all at once). Future direction: keep the edit capability open but block it systemically through custom settings + hiding it from priming.
- **An explicit create verb** — only adds API surface and the fork of "re-creating an existing name". With implicit creation the question disappears.
- **Notifying group changes** — notification overuse + the cost of withdrawing it. Confirm by query.
- **Implementing member-name normalization per entrance** — the asymmetry where only the CLI split names actually created a ghost member (one `"alice,bob"` member) (a real finding). Only a single point guarantees parity structurally.
- **`--settings` inline JSON** — `cmd /c` quote collapse silently drops the settings (documented basis from the spike measurement). A file path is the same pattern as the three already-verified mechanisms.
- **Splitting a name that contains a comma instead of rejecting it** — priming teaches the comma notation, so rejecting it would pit the documentation against the implementation. The cost (a name containing a comma cannot be registered in a group) is accepted by documenting it.

## 근거

- Real review findings: normalization asymmetry (ghost member) · silent registration of a `@` member · `--delete` parity divergence · the hybrid exit-code hole · partial batch application — all passed re-decision after fixes (round 7 PASS).
- Gates: rust 1,079 (core 290 · daemon 691 · proto/discovery 98) green · parity integration test (the two entrances byte-identical) · mutation verification.

## 영향 / 불변식

- Code anchors `// ADR-0109` = `ingress.rs` (normalization and validation single point) · `groups.rs` (update_members atomicity and @ guard) · `mcp_config.rs` (settings fragment) · `backend/claude.rs` (--settings injection). Even if a third surface (UI etc.) is attached to the dual entrance, the principle that semantic validation stays at the daemon single point holds.
- Accepted leftovers: the `outbound_pending` axis is name-based (MessageRecord has no id column — only the obligation axis is id-aware, the observation axis is a follow-up) · `messages{id}` unlimited query (assuming the body is not returned) · a group with a comma-containing name cannot be registered.
- spec consistency: §4 (group management semantics) and §6 (error vocabulary, CLI identity, --settings) are updated by this ADR.
