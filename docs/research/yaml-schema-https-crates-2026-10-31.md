# Server-side YAML parsing · JSON Schema validation · HTTPS fetching — library investigation (serde-saphyr · jsonschema · ureq)

- **Status:** medium · 2026-10-31 · 1 collector (worker-scout) + main grounding + codex adversarial review (effort high) — verdict FIX. The server branch of TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a (external library investigation per implementation part).
- **Occasion:** the stage where the S25 preset library server chooses crates to use for (1) reading preset YAML (2) validating external JSON Schemas (the claude · codex settings schemas) (3) fetching the schemas over HTTPS and refreshing them. TRD §1-12 pinned jsonschema 0.59.0, but the current version is 0.60.2.
- **Confidence legend:** certain = primary source + independent cross-confirmation (main grounding or codex recheck agrees) · likely = single primary source, no rebuttal · uncertain = guess/untested · contested = the review rebutted with counter-evidence.

## Conclusion

★The six corrections the review (FIX) forced are marked 「리뷰로 정정」 (corrected by review) in the relevant items.★

| Need | Choice | Reason | Runner-up |
|---|---|---|---|
| YAML reading | **serde-saphyr 2.0.0** (`default-features = false`, `["deserialize"]`) | Line · column + excerpt in errors, duplicate keys rejected by default, a budget cap (billion-laughs), no unsafe, fuzzed. Version check: 2026-10-13 (main grounding) | saphyr 0.3.3 (YAML 1.2 · has positions · no serde) |
| JSON Schema validation | **jsonschema 0.60.2** (`default-features = false`, `["resolve-file","idna"]`) | Blocks the default features from pulling in reqwest + rustls + aws-lc. Drafts 2020-12/2019-09/7/6/4 · `validator_for` auto-detection · `iter_errors` + `instance_path()` | boon 0.6.1 (2025-01) |
| HTTPS fetching | **ureq 3.5.0** blocking (`default-features = false`, `["rustls","platform-verifier","gzip"]`) | The refresh runs on one ordinary daemon thread rather than tokio, keeps the async-import gate (⑦) as is, and the dependency tree is small | reqwest 0.13.5 |

- **Correction 1 (corrected by review — ureq certificates):** the `platform-verifier` feature alone is not enough. `TlsConfig::builder().root_certs(RootCerts::PlatformVerifier)` must be **specified explicitly** on the agent. If not, the Mozilla roots remain the default. Certain (codex refuted it with docs.rs ureq 3.5.0 + it agrees with the collector's feature description).
- **Correction 2 (corrected by review — format validation):** 「포맷 검증은 `should_validate_formats(true)` 를 주기 전엔 꺼져 있다」 (format validation is off until it is explicitly enabled) was wrong. The default **differs by draft.** Since both target schemas are draft-07, **specify the policy (on/off) explicitly in code**, and decide how to handle unknown formats with a separate setting.
- **Correction 3 (corrected by review — system proxy):** the Windows system proxy (`win-system-proxy`) reads only `ProxyEnable`/`ProxyServer` under HKCU and builds **a single HTTP proxy URI**. PAC · WPAD · `ProxyOverride` do not work → write it as 「제한된 수동 프록시 발견」 (limited manual proxy discovery) (narrowing the collector's vague 「미상」 (unknown)).
- **Correction 4 (corrected by review — body size):** ureq's default body cap of 10 MiB applies only to `read_to_string`·`read_to_vec`·`read_json`, and the reader API is **unlimited**. Name the reading API to be used and set a schema size cap (based on the claude schema at about 231 KB) **explicitly**.
- **Correction 5 (corrected by review — reqwest):** reqwest also has a blocking client (`reqwest::blocking`) and a `rustls-no-provider` + ring combination. 「async 라서 탈락」 (dropped because it is async) was an overstatement. The ground on which ureq wins is narrowed to **the number of dependencies and no runtime needed**.
- **Correction 6 (corrected by review — omitted candidate):** `yaml_serde` was missing from the comparison. No actual comparison was made → raise it as 「비교 필요」 (comparison needed) in 「공백」 (gaps) below (no verdict).
- **Premise (supported by main grounding):** this repository's `Cargo.lock` has **none** of rustls · native-tls · schannel · ring · aws-lc · webpki-roots · ureq · a YAML crate · jsonschema. Whichever HTTPS client is chosen, a new TLS stack comes in.

## 1. YAML reading

| Name | Version · date | License | In the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|
| **serde-saphyr** | 2.0.0 · 2026-10-13 | Not in the input | No | Chosen. Error line · column + excerpt · `Spanned<T>` (a position per value) · duplicate keys rejected by default · budget cap · no unsafe · fuzzed. Parser = granit-parser (a saphyr fork). MSRV 1.89 | certain (crates.io version = main grounding · MSRV · deserialize-only configuration · duplicate-key rejection · strict boolean = codex recheck) | crates.io · github bourumir-wyngs/serde-saphyr |
| saphyr | 0.3.3 | Not in the input | No | Runner-up. YAML 1.2 · has positions · no serde | likely | crates.io |
| yaml_serde | 0.10.7 (offered by the review) | MIT/Apache | No | **Not compared** — a serde_yaml fork maintained by the YAML organization, MSRV 1.82, parser libyaml-rs | uncertain (not evaluated by the collector · only the review's description) | docs.rs yaml_serde · github yaml/yaml-serde `Cargo.toml` |
| serde_yaml | 0.9.34+deprecated | — | No | Eliminated — archived (dtolnay/serde-yaml archived = true, supported by main grounding) | certain | github |
| serde_yml | 0.0.13 | — | No | Eliminated — archived · a deprecated shell pointing to noyalib. RUSTSEC-2025-0068 (unsound, ≤0.0.12) | certain (RustSec) | rustsec RUSTSEC-2025-0068 |
| noyalib | 0.0.58 | Unconfirmed | No | Eliminated — same author as serde_yml, 0.0.x | likely | crates.io |
| serde_norway | 0.9.42 | — | No | Eliminated — stalled updates | likely | crates.io |
| saphyr-serde | 0.0.0 | — | No | Eliminated — a placeholder | likely | crates.io |
| yaml-rust2 | 0.13.0 | — | No | Eliminated — no serde | likely | crates.io |
| marked-yaml | 0.8.0 | — | No | Eliminated — little activity | likely | crates.io |

Notes (serde-saphyr):

- **YAML 1.1 shape:** if the target is bool, `yes/no/on/off` is read as bool. `Options { strict_booleans: true }` is recommended (strict boolean behavior is a codex recheck).
- **`Spanned<T>` limit:** positions vanish inside untagged · flatten. If M14's kind discrimination is implemented with an untagged enum, error positions will not come out (likely — before measurement).
- **Feature configuration:** `comments` is on by default. For the reading side, `default-features = false` + `["deserialize"]`.

## 2. JSON Schema validation

| Name | Version · date | License | In the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|
| **jsonschema** | 0.60.2 · 2026-10-26 | Not in the input | No | Chosen. Drafts 2020-12/2019-09/7/6/4 · auto-detection · `iter_errors` · `instance_path()`. Features `resolve-file`·`idna`·`resolve-http`·`tls-aws-lc-rs`·`tls-ring` exist | certain (version = main grounding · feature names and defaults = codex recheck) | docs.rs jsonschema 0.60.2 · github Stranger6667/jsonschema |
| boon | 0.6.1 · 2025-01 | Not in the input | No | Runner-up | likely | crates.io |
| valico | 4.0.0 | — | No | Eliminated — outdated | likely | crates.io |

Actual schema notes:

- **claude settings schema** (schemastore `claude-code-settings.json`): `$schema` is draft-07 but the definitions are in `$defs` (2019-09 keyword), the references are `#/$defs/...`, and there are 5 `if`. It passed the spike on 0.59 → the same test must be **pinned as a test** on 0.60.2. Certain (draft-07 `$schema` = main grounding · the rest = collector confirmed).
- **Server response:** gives ETag · Last-Modified and `Cache-Control: max-age=600` → conditional GET is possible. About 231 KB. `json.schemastore.org` → `www.schemastore.org` redirect. Likely (collector's curl).
- **codex side:** main's `core/config.schema.json` is draft-07 and uses `definitions`. The release asset path is **unconfirmed.**
- **TOML parsing:** reuse toml 1.1.2 · toml_edit 0.25 already in this repository (no new crate).
- **TRD drift:** §1-12's 0.59.0 must be corrected to 0.60.2.

## 3. HTTPS fetching

| Name | Version · date | License | In the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|
| **ureq** | 3.5.0 · 2026-10-21 | Not in the input | No | Chosen. Blocking · rustls (ring — only a C compiler, NASM/CMake not needed) · supports system proxy environment variables. The Windows system proxy is the optional dependency `win-system-proxy` (winreg) | certain (version = main grounding · blocking · ring · optional winreg = codex recheck). That `win-system-proxy` does not trip the 4g gate (which counts only direct dependencies) = likely | docs.rs ureq 3.5.0 · aws-lc-rs windows requirements · rustls#1913 |
| reqwest | 0.13.5 | Not in the input | 0.13.4 is in the shell graph (no TLS), 0.12.28 is daemon dev-only (no TLS) | Runner-up — async. Default TLS = rustls + aws-lc-rs (needs NASM on Windows, exception if a prebuilt exists) · system proxy on by default. It also has a blocking client (correction 5). When running on top of tokio or when daemon HTTPS grows further | likely | crates.io · docs.rs |
| native-tls · schannel | — | — | No | Not recommended — splits per platform | likely | Collector's judgment |

Configuration guidance:

- **Certificates:** specify `RootCerts::PlatformVerifier` as in correction 1 (the Windows certificate store — to handle corporate proxies).
- **Timeouts:** set them explicitly.
- **Proxy:** follow the environment variables. The Windows system proxy has the limit in correction 3.
- **Conditional GET:** implement directly (`If-None-Match` · 304). The 304 handling behavior is **unconfirmed.**
- **Body size:** correction 4 — decide the reading API and set a cap.

## Adversarial review result (codex · effort high · FIX)

| # | Grade | Finding | Handling |
|---|---|---|---|
| 1 | MED | The ureq `platform-verifier` feature alone is not enough — `RootCerts::PlatformVerifier` must be specified | Reflected (correction 1) |
| 2 | MED | The default for format validation differs by draft | Reflected (correction 2) — making the policy explicit an implementation requirement |
| 3 | MED (omission) | `yaml_serde` not compared | **Left** — the input has no comparison result, so raised as a gap with no verdict |
| 4 | MED | win-system-proxy is only a single HKCU manual proxy | Reflected (correction 3) — ureq 3.5.0 `src/proxy.rs` 235–255 |
| 5 | LOW | reqwest also has a blocking client · `rustls-no-provider` + ring | Reflected (correction 5) |
| 6 | LOW | ureq's body cap is only on some APIs | Reflected (correction 4) — `src/body/mod.rs` |

Confirmed by recheck: serde-saphyr MSRV 1.89 · deserialize-only configuration · duplicate-key rejection · strict boolean; jsonschema 0.60.2 feature names and defaults; ureq blocking · ring · optional winreg.

## Gaps (공백) · measurements needed

- **A yaml_serde comparison is needed** (0.10.7 · MIT/Apache · MSRV 1.82 · parser libyaml-rs — only what the review described). No verdict — compare with serde-saphyr on error positions · duplicate keys · bool handling · dependency size · maintenance status.
- A test that the claude schema (draft-07 + `$defs` mixed) passes on jsonschema 0.60.2 — not yet run.
- The codex release asset schema path — unconfirmed.
- The ureq 304 response handling behavior — unconfirmed. After choosing the body-cap API, test the behavior on exceeding the size.
- The actual behavior of `win-system-proxy` on Windows · whether PAC/WPAD environments being unsupported (correction 3) matters in real use.
- Spanned vs M14 untagged parsing — measure in the implementation pilot whether positions actually vanish.
- The noyalib license — unconfirmed (an eliminated candidate, so low priority).
- The licenses of each chosen crate (serde-saphyr · jsonschema · ureq) are not in the input — check them with `Cargo.toml` · LICENSE before adoption.
- Repository state: serde_json 1.0.150 · tokio 1.52.3 · hyper 1.10.1 · tungstenite 0.26.2 · toml 1.1.2 + toml_edit 0.25 · walkdir 2.5.0 already present. Toolchain 1.95.0 (meets serde-saphyr's MSRV 1.89).

## Sources

- crates.io: serde-saphyr 2.0.0 · jsonschema 0.60.2 · ureq 3.5.0 (version · date = main grounding)
- github: bourumir-wyngs/serde-saphyr · Stranger6667/jsonschema · dtolnay/serde-yaml(archived) · yaml/yaml-serde(`Cargo.toml`)
- rustsec RUSTSEC-2025-0068
- docs.rs: jsonschema 0.60.2(`ValidationOptions::should_validate_formats`) · ureq 3.5.0(`#platform-verifier`) · yaml_serde
- ureq 3.5.0 source: `src/proxy.rs` 235–255 · `src/body/mod.rs`
- schemastore `claude-code-settings.json`
- aws-lc-rs windows requirements · rustls#1913
