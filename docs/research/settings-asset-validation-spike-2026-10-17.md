# ★ Checking settings asset contents — measured on claude and codex (schema and reaction at run time)

| Item | Value |
|---|---|
| Status | Measurement complete · awaiting decision (user) |
| Date | 2026-10-17 |
| Method | Spike — one worker ran the actual commands in order (claude 2.1.284 · codex-cli 0.156.1 · Windows) · downloaded the schemas · cross-checked with ajv 8.20 · main spot-checked the output files (`Invalid settings` output · codex warning · `--strict-config` error text). Cost ≈ claude $0.33 + one codex run of 21k tokens · no global settings file changed |
| Question | Can the contents of a settings asset's raw field (claude = the object passed with `--settings` · codex = the list of `-c 키=값` (key=value)) be checked before running — download the public schema up front and use it · if that does not work, errors during the run · problems shown as icons in the list |
| Artifacts | Session scratch (may be deleted) — the commands and raw outputs copied into this document are the canonical record |

## Conclusion

- **claude — ★at run time it fails silently★.** If even one key has the wrong type, it **silently discards the whole** `--settings` object and runs with exit 0 (`{"cleanupPeriodDays":"ten","model":"haiku"}` → ran with opus · control `{"model":"haiku"}` → haiku · nothing in stderr or in `--debug-file`). An unknown key is ignored on its own, only that key. So **a check before running is a must.**
- **claude — the check that makes no model call = `claude --settings '<json>' doctor`** (about 2 seconds) — it prints `Invalid settings` / `- …\claude-settings-<hash>.json › cleanupPeriodDays: Expected number, but received undefined`. ★The exit code is 0 even when the settings are wrong★ — the output text has to be read (the wording is not a documented contract). It does not catch unknown keys.
- **claude public schema = SchemaStore** `https://json.schemastore.org/claude-code-settings.json` (→ 301 → 200 · 231,533 B · draft-07 · 142 top-level properties · top-level `additionalProperties: true`). The Anthropic settings docs point to this schema and say 「최신 CLI 보다 늦을 수 있다」 (it may lag behind the latest CLI). It catches type errors (`cleanupPeriodDays must be integer` · `permissions/allow must be array`) and cannot catch unknown keys. ajv strict mode fails to compile (`unknown keyword: "allowTrailingCommas"`) → use non-strict.
- **codex — a schema matching the version is public, and it is strict** — `https://github.com/openai/codex/releases/download/rust-v0.156.1/config-schema.json` (200 · 220,935 B · latest = `https://developers.openai.com/codex/config-schema.json` · source `codex-rs/core/config.schema.json` · draft-07 · `additionalProperties:false`). It catches unknown keys · enum value errors · type errors on integer fields, and lets free-string fields (`model` · `model_reasoning_effort`) through.
- **codex at run time:** unknown key = warning, then continues (`Codex is ignoring 1 unrecognized configuration setting … notARealKey is ignored`) · with `--strict-config`, exit 1 before the model call (`unknown configuration field \`notARealKey\` in -c/--config override`) · enum value error (`approval_policy=bogus`) = exit 1 before the model call · `model_reasoning_effort=hihg` · `model=[unclosed` = pass locally → exit 1 **at the API** (the list of allowed values appears in the error). A value that is not TOML becomes a plain string (behavior stated in `--help`).
- **codex check that makes no model call:** `codex doctor --json -c <kv>` (about 2 seconds · unknown key = warning · enum error = fail but the detail is 「invalid data」 · `hihg` = ok) · `codex debug models` (under 1 second · model names and the `supported_reasoning_levels` list).
- **yaml syntax = serde-saphyr 2.0.0** (MIT OR Apache-2.0 · MSRV 1.89) — `Error::location() -> Option<Location>` · `line()/column()` are 1-based · character-based → 「N째 줄」 (line N) is possible. Because the location is an `Option`, some errors have no line. (docs.rs — not compiled)
- **Rust schema-check candidates:** `jsonschema` 0.59.0 (MIT · MSRV 1.85 · its default features pull in reqwest/rustls, so turn off `resolve-http`) · `boon` 0.6.1 (MIT OR Apache-2.0 · last release 2025-01) · `valico` 4.0.0 (stagnant since 2023-05).

## Measurement table

| Case | Command | Result |
|---|---|---|
| claude broken JSON ① `{"env": {"X": "1"` | `claude -p "reply with the word ok" --max-turns 1 --no-session-persistence --output-format json --settings <x>` | exit 1 · before the model call · `Error: Settings file not found: <cwd>\{"env": {"X": "1"` — it read the text as a file path |
| claude broken JSON ② `{"env": {"X": "1"}, oops}` | same | exit 1 · `Error: Invalid JSON provided to --settings` |
| claude unknown key `{"notARealKey":true}` | same | exit 0 · only that key silently ignored (other keys apply — `+"model":"haiku"` → haiku) |
| claude type error `{"cleanupPeriodDays":"ten"}` | same | exit 0 · **the whole object silently discarded** (confirmed by a control run) |
| claude nonexistent model `{"model":"no-such-model"}` | same | reaches the API · exit 1 · `[claude-code:unrecognized_model]` · 404 · cost 0 |
| codex unknown key `-c notARealKey=1` | `codex exec --skip-git-repo-check --ephemeral -s read-only -c <kv> "reply with the word ok"` | exit 0 · runs after a warning · `--strict-config` = exit 1 (before the model call) |
| codex wrong value `-c model_reasoning_effort=hihg` | same | passes locally · exit 1 at the API — 「Invalid value: 'hihg'. Supported values are: 'none', 'minimal', 'low', 'medium', 'high', 'xhigh', and 'max'.」 |
| codex non-TOML value `-c 'model=[unclosed'` | same | taken as a string, exit 1 at the API · for an integer field, `expected i64` locally |
| codex enum error `-c approval_policy=bogus` | same | exit 1 before the model call · `unknown variant \`bogus\`` |

## Flow proposed by the measurer (not a decision)

1. At save and scan time (daemon · offline): syntax with serde-saphyr → 「N째 줄」 (line N) · content check with the downloaded schema (codex = the release schema matching `codex --version` · claude = SchemaStore + a separate check for 「스키마에 없는 키」 (keys not in the schema)) — the schema may lag behind the CLI, so a **warning** icon.
2. Right before running (no model call): the `Invalid settings` line in the output of `claude --settings '<json>' doctor` → **error** icon · for codex, check the model and effort values with `codex debug models` · `--strict-config` (unknown keys as errors) is the user's decision.
3. During the run: exit code + stderr as a list icon — claude `Invalid JSON` · model 404 · codex `Error loading config.toml` · API 400.

## What was not measured

- claude interactive TUI · stream-json mode (only `-p` was run — the TUI may show a settings dialog).
- Whether the doctor output wording stays the same when the version changes (not documented).
- ~~Rust `jsonschema` and serde-saphyr were not actually compiled and run (ajv stood in) · schemars `uint*` format handling not checked.~~ → measured below in "Rust crate measurement (2026-10-18)".
- Whether `codex debug models` goes over the network.

## Rust crate measurement (2026-10-18)

One spike crate was actually compiled and run (rustc 1.95.0 · Windows · debug). 「실측」 (measured) = seen by running it · 「소스 읽음」 (source read) = read from the crate source or metadata without running.

**Conclusion:** `jsonschema` 0.59.0 and serde-saphyr 2.0.0 both plug in as planned. Three holes remain: ① schemars `uint*` formats are ignored, so integer ranges come only from the schema's `minimum`/`maximum`, and upper bounds with no `maximum` (uint16 · uint32) are not caught ② unknown top-level claude keys have to be caught by us by comparing the keys against the schema `properties` ③ `oneOf` enum errors are a vague message with no list of allowed values.

### Dependencies and build

| Item | Value | Basis |
|---|---|---|
| Resolved versions | `jsonschema` 0.59.0 (+ `referencing` 0.59.0 · `fancy-regex` 0.19.2 · `idna` 1.1.0) · `serde-saphyr` 2.0.0 (parser `granit-parser` 2.0.0) · `toml` 0.8.23 | measured |
| `jsonschema` default features | `resolve-http` · `resolve-file` · `tls-aws-lc-rs` · `idna` | source read |
| Features used | `default-features = false, features = ["resolve-file","idna"]` → no reqwest · rustls · aws-lc · tokio | measured |
| Clean dev build (with serde · serde_json · toml) | 35.7 s · `Cargo.lock` 124 packages · `jsonschema` subtree ≈ 107 lines (regex · fancy-regex · idna) · serde-saphyr ≈ 19 lines | measured |

### Schemas

| Schema | Size and shape | Basis |
|---|---|---|
| codex `https://github.com/openai/codex/releases/download/rust-v0.156.1/config-schema.json` | 220,935 B · draft-07 · 100 top-level properties · top-level `additionalProperties:false` · formats uint×22 · int64×17 · uint64×17 · double×5 · uint16×3 · uri×3 · int32 · uint32 · uint8 | measured |
| claude `https://json.schemastore.org/claude-code-settings.json` | 231,533 B · draft-07 · 142 top-level properties · top-level `additionalProperties:true` | measured |
| Remote `$ref` | none → offline compile · compiling one schema ≈ 28 ms (debug) | measured |

### Unknown formats (`uint` and others)

- Default = **silently ignored** (the same even with `should_validate_formats(true)`). `should_ignore_unknown_formats(false)` → compile error `Unknown format: 'uint'`. (measured)
- A custom format `.with_format(name, |s: &str| …)` runs **only on string instances** → it cannot enforce an integer range (even with a custom `uint32` attached, 5e10 passes). (measured)

### Check results (measured)

| Case | Result |
|---|---|
| codex valid settings | valid |
| codex `project_doc_max_bytes="big"` | not of type integer |
| codex uint field -1 | caught — schema `minimum: 0` |
| codex uint8 field 300 | caught — `maximum` |
| codex uint16 `mcp_oauth_callback_port`=70000 | **passes** — no `maximum` · format ignored |
| codex `approval_policy` wrong enum value | 「not valid under any of the schemas listed in the 'oneOf'」 — no list of allowed values |
| codex unknown top-level key | 「Additional properties are not allowed ('not_a_key' was unexpected)」 |
| codex unknown nested key `features.bogus_flag` | caught |
| claude valid settings | valid |
| claude `includeCoAuthoredBy:"no"` | not boolean |
| claude `cleanupPeriodDays:0` | less than minimum 1 |
| claude `permissions.allow:"Bash"` | not array |
| claude unknown top-level key | **valid (not caught)** → caught by comparing the instance keys with the schema `properties` |
| claude unknown nested key `permissions.allowww` | caught — that object has `additionalProperties:false` |

Error API = `iter_errors` · `instance_path()` · `schema_path()` · Display · `kind()`. (measured)

### serde-saphyr (measured)

- Struct round trip OK (block style). For the front matter, we cut at the `---` line and call `from_str`, which works.
- Errors = `err.location() -> Option<Location>` · `.line()/.column()` are 1-based · Display starts with `line N column M: …` and a caret snippet follows — wrong indentation (5,2) · unclosed quote (3,1) · scalar where a list should be (3,8) · missing field (2,1) · duplicate key = rejected by default (3,1) (`DuplicateKeyPolicy` option). Oddity: a tab at the start of line 2 was reported at (1,7).
- The result of `from_str::<serde_json::Value>` can be fed to `jsonschema` as is.
- ★**Surprise — an unquoted `flag: yes` becomes JSON `true`** (YAML 1.1 boolean)★. Whether an option exists to turn this off is **not checked**.

### Remaining holes

- Upper bounds on uint16 · uint32 with no `maximum` are not caught — catching them takes a separate post-processing step that reads `format`.
- Unknown top-level claude keys = write a separate comparison against `properties`.
- The `oneOf` enum error wording is vague.
