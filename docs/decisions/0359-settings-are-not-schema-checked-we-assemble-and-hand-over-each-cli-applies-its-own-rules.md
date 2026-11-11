# ADR-0359: Settings are not schema-checked — we assemble and hand over, each CLI applies its own rules

- 상태: 확정 (2026-11-11, 근거: user decision 2026-11-11 (S25 step 7, P4 review question 1))
- 관련: Supersedes ADR-0332 · ADR-0334 · ADR-0336 · ADR-0323 · ADR-0331 · ADR-0324 · ADR-0311 · ADR-0314 · child TRD `docs/process/S25-preset-library/trd-asset-library.md` §3-6 · parked P4 branch `v0.5.0/wip/s25-p4-schema-validate-parked` (`002cd9f9`) · Amends ADR-0334 (decision 3 schema source and unknown-key handling) · Amends ADR-0336 (jsonschema row and ureq row) · Amends ADR-0323 (decisions 2 and 3)

## 맥락
ADR-0332 had Engram check every settings asset (claude raw-text JSON · codex `-c` lines) against the CLI's JSON schema and block the run on a type or range error. Step 7 P4 implemented it (`validate.rs`, `jsonschema` 0.60.2, +26 packages) and needed nine deep-review rounds: a supported-keyword gate, `$ref` limits, work and copy budgets, merged-value attribution, a refresh mutex. Its open questions showed the check is stricter than the CLIs (a value the CLI may accept is refused) and adds policy of its own (split settings failing `required` per asset, refused newer schema copies, keywords outside the gate). Asked whether a wrong value should keep blocking the run, the user asked why Engram filters at all when the CLI already reports a broken settings file.

## 결정
1. Engram does not check settings assets against any schema — not claude's, not codex's. A wrong key or value is handled by the CLI by its own rules when it starts.
2. Engram's job is to assemble the launch materials correctly and hand them over. It keeps only the checks without which it cannot build or pass the materials: YAML syntax of an asset, the shape it reads (front matter · fields), the codex skill format (with codex's repair rules — ADR-0336 row `SKILL.md`), and the command-line guard and length bound.
3. Nothing downloads, caches or refreshes schemas. No notice exists for unknown keys, an older schema or a missing schema.
4. The decision applies to every backend, present and future — a new backend gets no schema check by default.

## 거부한 대안
- **Keep ADR-0332: block on a type or range error** — the check is stricter than the CLI and its maintenance is large (P4: 9 review rounds, 5 open policy questions). User 「구지 우리가 필터링을 해서 유지보수를 힘들게 하기보다는 그냥 클로드 룰을 따르자」. 약함 — 사용자 직접 기각.
- **Keep the check, warn only (⚠, never block)** — presented as option B; same maintenance cost for a notice the CLI already gives at start. The user chose to drop the check for both backends. 약함 — 사용자 직접 기각.
- **Block only files that carry permission rules (`permissions.deny` etc.), warn on the rest** — presented as option C because claude skips an invalid file whole, so a deny rule in it is lost until the file is fixed. User 「만약에 문제가 있으면 클로드에서 못하게 막았겠지」. 약함 — 사용자 직접 기각.
- **Drop the check for claude only, keep codex's** — presented; user 「우리는 잘 조합해서 넘기는 것에 집중하자. 문제 생기면 codex claude가 알아서 그쪽룰로 처리하게. 그쪽에서 룰이 잘 되있는걸 구지 우리가 막을 필요는 없다고 생각.」 약함 — 사용자 직접 기각.

## 근거
- User decision 2026-11-11 (quotes above).
- Claude Code validates its settings files and skips a file that fails its schema whole, with a 「Found N invalid settings files」 warning, and still starts (issue reports anthropics/claude-code #14798 · #10096 · #88564 — read from issue pages, not measured here). Not verified whether a file passed with `--settings` gets the same treatment.
- codex's handling of an invalid `-c` value: not measured.
- Cost seen: P4 as coded is 17,060 inserted lines across 12 files (parked commit `002cd9f9`).

## 영향 / 불변식
- Step 7 P4 shrinks to the hand-over checks of decision 2; the schema part is parked on `v0.5.0/wip/s25-p4-schema-validate-parked` (do not merge; reuse parts from it).
- Not added: `jsonschema`, and `ureq` for schema fetching (ADR-0336 rows); step 17 (schema files `{cli_version, etag, schema}`) is dropped; `daemon\cache\` has no first occupant (ADR-0323); the asset-library fetch port for schemas is not built (ADR-0311 decision 4).
- The settings-check port (ADR-0334 decision 3) loses the schema source and unknown-key handling; raw text → JSON, claude merging, the codex skill format and the command-line guard stay. The CLI version probe stays only if another use needs it — the TRD pass decides.
- Reason codes `settings-unknown-key` · `settings-schema-outdated` · `settings-schema-unavailable` and the schema dimming notice (ADR-0331 decision 3) are not built. A new schema re-check as a `Catalog` change (ADR-0324 decision 8) does not exist. Stored per-file errors are parse and shape errors only (ADR-0314 decision 7).
- Accepted consequence: a wrong settings value surfaces only when the agent starts, in the CLI's own message; for claude the whole file is skipped until fixed, including any deny rules in it.
- Do not add a schema check, a schema fetch or a schema-based notice for a backend without a new ADR.
