# ADR-0109: Group v1 management semantics and dual-entry contract — everyone can edit, implicit creation, @ reserved, a single normalization point, --settings file injection

- 상태: 확정 (2026-07-27, basis: 5 user decisions (2026-07-26~27) + real findings from the D adversarial review) · 부분 폐기 by ADR-0111 (decisions 1-5 group management semantics)
- 관련: ADR-0086(dual entry) · ADR-0094(grants least privilege — kept unextended) · ADR-0099(mcp-config file lifecycle — settings rides along) · ADR-0101(WYSIWYA name addressing) · ADR-0103(the original decision for the §6 entries and fragment — this ADR makes it concrete as the D-stage contract) · spec §4·§6 · Amended by ADR-0111 (decisions 1-5 group management semantics)

## 맥락

While implementing the `group` (roster management) and `messages` (open-items query) tools, their CLI mirrors, and the spawn `--settings` injection, D (entry completion) had to settle the policy and contract forks that were still open. The review (two adversarial reviewers + 7 blind rounds) found for real the contract divergence caused by having two entries (normalization asymmetry, split error vocabulary).

## 결정

**Group v1 semantics (user decisions 2026-07-26~27):**
1. **Everyone can edit** — anyone creates, edits, and deletes any group. No ACL and no record of the acting party (all at once when the record and permission system is upgraded — step-log backlog).
2. **Quietly** — no notification for a roster change or deletion (only the v1 default principle).
3. **A roster change or deletion has no effect on broadcast items that are already parked** — a consequence of the snapshot-at-send-time principle (a recall feature is not adopted).
4. **`@` = a namespace for groups only** — a group name must have `@` (otherwise `INVALID_GROUP_NAME`), `@` is forbidden in a member name (`INVALID_MEMBER_NAME` — nested groups are unsupported, and this blocks a literal phantom member), and the built-in `@all` is protected (`GROUP_BUILTIN`).
5. **Implicit creation** — there is no create verb: add to a group that does not exist creates it, remove and query do not create. No downgrade to an empty add (`INVALID_GROUP_ARGS`).

**Entry and contract (implementation decisions — reflecting real review findings):**
6. **Semantic validation and normalization = a single daemon point.** Splitting member names on commas, trimming, and removing whitespace happen only in the shared handler (ingress) — the CLI forwards the raw text. The two entries produce byte-identical JSON (pinned by a parity integration test). A batch change is applied all at once after full validation (no partial application).
7. **The `messages` contract** — an id query returns per-recipient rows, and when there are fewer than `expected_rows` it reports `may_be_truncated` (the expected row count is recorded at send time — position-based "proof" was disproved by the two-stage group record and discarded). The no-argument form returns the caller's three open axes (direction tag), **reply obligations match by id first** (blocks misattribution to a same name), and **an obligation follows whoever actually received the envelope** (re-bound at delivery time). Times are relative seconds (the ledger holds only a monotonic clock). No ownership restriction, body not returned.
8. **CLI identity = ENGRAM_TOKEN only** — no spoofing with `--as` (derived server-side as BoundIdentity). Exit code: a 2xx response with `status=="error"` can never be 0, in any form (common to send and query).
9. **`--settings` = file path injection** (not inline JSON) — quote collapse in the Windows `cmd /c` wrapping is a silent failure mode, so it is excluded. The fragment keeps the single key `{"allowedMcpServers":[{"serverName":"engram"}]}` (its priority is high, so no junk drawer — pinned by a regression test), and rides along on the mcp-config directory, sweep, and revoke lifecycle (ADR-0099). A write failure is warn + continue (unlike mcp-config, a missing fragment is a degradation, not a block).
10. **grants (ADR-0094) not extended** — `messages`/`group` are not sender entries, so they are not added to the `--allowedTools` list. It has no effect under the current spawn permission mode, and a user decision is needed when a constraint layer is introduced (code comment flag).

## 거부한 대안

- **Include ACL / acting-party record in v1** — deferred by user decision (no stopgap patches — all at once). Future direction: keep the edit ability open, and block it systemically with custom settings + not exposing it in priming.
- **An explicit create verb** — grows the API surface and only produces the fork "re-create an existing name". With implicit creation the question disappears.
- **Group change notification** — notification spam + the cost of stripping it out. Confirm by query.
- **Implementing member-name normalization per entry** — the asymmetry where only the CLI split names actually produced a phantom member (`"alice,bob"` as one person) (found for real). Only a single point guarantees parity by structure.
- **`--settings` inline JSON** — `cmd /c` quote collapse silently drops the settings (spike measurement, documented). A file path is the same pattern as the three already-verified mechanisms.
- **Splitting a name that contains a comma instead of rejecting it** — priming teaches comma notation, so a reject would be a fight between the docs and the implementation. The cost (a comma-containing name cannot be registered in a group) is accepted by documenting it.

## 근거

- Real findings of the review: normalization asymmetry (phantom member) · silent registration of an `@` member · `--delete` parity divergence · the exit code hybrid hole · partial batch application — all passed re-verdict after being fixed (round 7 PASS).
- Gate: rust 1,079 (core 290 · daemon 691 · proto/discovery 98) green · parity integration test (two entries byte-identical) · mutation verification.

## 영향 / 불변식

- Code anchors `// ADR-0109` = `ingress.rs` (the single point for normalization and validation) · `groups.rs` (update_members atomicity · @ guard) · `mcp_config.rs` (the settings fragment) · `backend/claude.rs` (--settings injection). Even if a third surface (UI etc.) is attached to the dual entry, the principle that semantic validation is a single daemon point stays.
- Accepted residuals: the `outbound_pending` axis is name-based (MessageRecord has no id column — only the obligation axis is id-aware, the observation axis is follow-up) · unlimited `messages{id}` query (on the premise that the body is not returned) · a comma-containing name cannot be registered in a group.
- Spec alignment: §4 (group management semantics) and §6 (error vocabulary · CLI identity · --settings) are updated by this ADR.
