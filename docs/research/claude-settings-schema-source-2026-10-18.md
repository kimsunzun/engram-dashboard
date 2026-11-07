# claude settings schema — where to get it · whether we may check it ourselves

- **Status:** light investigation + measurement · 2026-10-18 · collector 1 (source/binary decision) + main grounding (official-doc sentences · extracted-file comparison) · no adversarial review
- **Question:** The reason we run `claude --settings <json> doctor` (about 2 s) is to have settings checked against the settings schema inside claude. Can Engram get that schema directly, or use a public schema, to check immediately (syntax · whether the key exists · whether the value is right)?

## Conclusion

- **claude's internal schema (zod) can be extracted as a JSON schema — but it is not a public route.** The bundled `update-config` skill converts the internal zod schema to a JSON schema at run time and puts it in the prompt. We captured it by having the model call that skill through a fake local API (no model call · about 5 s · claude 2.1.284 → draft 2020-12 · 261 KB · 176 top-level keys). There is no command, flag, or schema file in the package that prints the schema. It may break with any version. — certain (measured · extracted file checked)
- **The SchemaStore schema is a different, community-maintained hand copy and lags behind.** It is matched by hand from the docs and changelog (PR #6131 「reconciled against the full settings.md and env-vars.md reference tables」), and the last version sync = v2.1.220 (2026-07-27). Compared with the 2.1.284 internal schema: 48 keys missing · 14 keys left over · `managedMcpServers` has a different type. — certain (measured comparison · commit history)
- **The official docs point to SchemaStore** — 「The `$schema` line points to the published JSON schema for Claude Code settings, which gives you autocomplete and inl[ine validation]」 (https://code.claude.com/docs/en/settings · checked against the original text). It recommends it for editor autocomplete · validation. There is no sentence saying it is official · auto-generated. — certain
- **We could not find any place where Anthropic publishes a per-version claude settings schema** — likely
- **codex ships a `config-schema.json` with every release** (generated from code · matches that version exactly). — certain (asset exists) · likely (how it is generated)

## Implications

- Using the internal schema checks almost the same as claude does, immediately, but the way to extract it relies on internal implementation (skill name · prompt header) and can break at any time.
- Using SchemaStore is light and the route the official docs recommend, but it may treat new keys as unknown keys (48 of them) and some keys have mismatched types → flagging 「모르는 키」 as an error causes false alarms.
- Both allow unknown keys at the top level — 「키가 있나」 we must compare ourselves, separately against `properties`.

## Gaps

- We did not run a Rust `jsonschema` check with the extracted internal schema. Unconfirmed: the extent of rules that do not translate into JSON schema, such as zod `.catch` · refine, and what the skill's post-processing removes.
- Extraction measurement files (session scratch · outside the repository): `internal-settings-schema.json` · `cap2.js`.
