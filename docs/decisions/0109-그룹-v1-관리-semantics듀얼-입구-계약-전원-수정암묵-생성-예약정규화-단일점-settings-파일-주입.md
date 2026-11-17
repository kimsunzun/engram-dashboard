# ADR-0109: Group v1 management semantics·dual entrance contract — everyone can edit·implicit creation·@ reserved·single point of normalization·--settings file injection

- 상태: 확정 (2026-07-27, basis: 5 user decisions (2026-07-26~27) + D adversarial review real findings) · 부분 폐기 by ADR-0111 (decisions 1-5 group management semantics)
- 관련: ADR-0086(dual entrance) · ADR-0094(grants least privilege — kept unextended) · ADR-0099(mcp-config file lifecycle — settings rides along) · ADR-0101(WYSIWYA name addressing) · ADR-0103(original decision of the §6 entrance · fragment — this ADR makes it concrete as the D-stage contract) · spec §4·§6 · Amended by ADR-0111 (decisions 1-5 group management semantics)

## 맥락

D (entrance completion) implemented the `group` (list management) and `messages` (pending query) tools and their CLI mirrors and the spawn `--settings` injection, and had to settle the policy and contract forks that were open. The review (2 adversarial reviewers + blind round 7) found real contract divergences caused by the dual entrance (normalization asymmetry · error vocabulary divergence).

## 결정

**Group v1 semantics (user decisions 2026-07-26~27):**
1. **Everyone can edit** — anyone creates · edits · deletes any group. No ACL, no record of the acting party (all at once when the record · permission system is advanced — step-log backlog).
2. **Quietly** — no notice on list change or deletion (the v1 principle of defaults only).
3. **List change · deletion has no effect on already-parked broadcast items** — a consequence of the snapshot-at-send-moment principle (a recall feature is not adopted).
4. **`@` = a group-only namespace** — a group name requires `@` (otherwise `INVALID_GROUP_NAME`), `@` is forbidden in member names (`INVALID_MEMBER_NAME` — blocks it from becoming a literal ghost member since nested groups are unsupported), `@all` is built-in protected (`GROUP_BUILTIN`).
5. **Implicit creation** — no create verb: add to a nonexistent group creates it, remove · query do not create. Downgrading to an empty add is forbidden (`INVALID_GROUP_ARGS`).

**Entrance · contract (implementation decisions — reflecting real review findings):**
6. **Semantic validation · normalization = a single daemon point.** Splitting member names on commas, trimming and removing whitespace happen only in the shared handler (ingress) — the CLI forwards the raw text. The two entrances produce byte-identical JSON (pinned by a parity integration test). A batch change is applied all at once after full validation (no partial application).
7. **`messages` contract** — an id query returns per-recipient rows + `may_be_truncated` when short against `expected_rows` (the expected row count is recorded at send time — position-based "proof" was disproved by the group 2-stage record and discarded). With no argument it returns the caller's 3 pending axes (direction tag), **the reply obligation matches id first** (blocks misattribution to a namesake) and **the obligation follows whoever actually received the envelope** (re-binding at delivery time). Times are relative seconds (the ledger holds only a monotonic clock). No ownership restriction · the body is not returned.
8. **CLI identity = ENGRAM_TOKEN only** — no spoofing via `--as` (server-side BoundIdentity derivation). exit code: a 2xx response with `status=="error"` cannot become 0 in any form (common to send · query).
9. **`--settings` = file path injection** (not inline JSON) — excluded because quote collapse in the Windows `cmd /c` wrapping is a silent failure mode. The fragment stays a single key `{"allowedMcpServers":[{"serverName":"engram"}]}` (it has high precedence, so no junk-drawer — pinned by a regression test), and rides along with the mcp-config directory · sweep · revoke lifecycle (ADR-0099). A write failure is warn+continue (unlike mcp-config, a missing fragment = degradation, not a block).
10. **grants (ADR-0094) not extended** — `messages`/`group` are not sending entrances, so they are not added to the `--allowedTools` list. No effect under the current spawn permission mode; a user decision is needed when a constraint layer is introduced (code comment flag).

## 거부한 대안

- **Including ACL / acting-party record in v1** — deferred by user decision (no temporary patches — all at once). Future direction: keep the edit capability open but block it systemically with custom settings + not exposing it in priming.
- **An explicit create verb** — only increases API surface and creates a fork of "re-creating an existing name". With implicit creation the question itself disappears.
- **Group change notice** — notice overuse + removal cost. Check by query.
- **Per-entrance implementation of member-name normalization** — the asymmetry where only the CLI split names actually created a ghost member (`"alice,bob"` as 1 person) (real finding). Only a single point guarantees parity by structure.
- **`--settings` inline JSON** — `cmd /c` quote collapse silently discards the setting (basis: spike measurement document). The file path is the same pattern as the 3 already-verified mechanisms.
- **Decompose instead of rejecting names that contain a comma** — priming teaches comma notation, so rejection would pit the document against the implementation. The cost (a name containing a comma cannot be registered as a group) is accepted by documenting it.

## 근거

- Real review findings: normalization asymmetry (ghost member) · quiet registration of `@` members · `--delete` parity divergence · exit code hybrid hole · batch partial application — all fixed and re-decided as passing (round 7 PASS).
- Gate: rust 1,079 (core 290 · daemon 691 · proto/discovery 98) green · parity integration test (two entrances byte-identical) · mutation verification.

## 영향 / 불변식

- Code anchor `// ADR-0109` = `ingress.rs`(single point of normalization·validation)·`groups.rs`(update_members atomicity·@ guard)·`mcp_config.rs`(settings fragment)·`backend/claude.rs`(--settings injection). Even if a third surface (UI etc.) attaches to the dual entrance, the principle that semantic validation is a single daemon point holds.
- Accepted remainder: the `outbound_pending` axis is name-based (MessageRecord has no id column — only the obligation axis is id-aware, the observation axis is a follow-up) · unbounded `messages{id}` query (premise: the body is not returned) · a group with a comma-containing name cannot be registered.
- spec alignment: §4 (group management semantics) · §6 (error vocabulary · CLI identity · --settings) are updated by this ADR.
