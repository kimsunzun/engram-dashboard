# ADR-0376: library.json and identity.json follow the base data-file rules; a newer or unreadable library.json makes set_root read-only for the run, and a newer or unwritable identity.json gives a run-only daemon id with a warn

- 상태: 확정 (2026-11-17, 근거: S25 step-9a TRD §8 (b) B3, PRD+TRD review PASS 2026-11-17 (full))
- 관련: ADR-0291 (the base data-file rules, decision 6 R17) · ADR-0264 (decision 4: a file name used by one store belongs to that store) · ADR-0311 (decision 5: `daemon_id` owned by the daemon and carried as a value) · ADR-0375 (what `library.json` stores) · step-9a TRD `docs/process/S25-preset-library/trd-library-edit.md` §1 · §2 · §3.1 · §8 (b) B3 · §10 K4

## 맥락
Step 9a adds two persisted state files. `<data>\daemon\state\library.json` holds the library root setting (ADR-0375); asset-library owns its name and format, joined onto the state folder the daemon hands in. `<data>\daemon\state\identity.json` holds the daemon identity `{"schema_version": 1, "daemon_id": "<uuid v4, lowercase hyphenated>"}`; the daemon owns its name and format in a daemon module, not `DataLayout` (ADR-0264 decision 4). Each needs a rule for what happens when the file is missing, corrupt, written by a newer version, or cannot be read or written.

## 결정
1. **Both files follow the base data-file rules (ADR-0291)** — `base::file` `Spec` / load / `judge` — as agent `persistence/mod.rs` does.
2. **`library.json`**: `Spec` with `version_key = "schema_version"`, `current = 1`, a small cap (64 KiB). Load outcomes:

   | Load result | Root state | Writes this run |
   |---|---|---|
   | Missing (never written) | first-start default | yes |
   | Usable | that root | yes — re-`judge` before each write |
   | Corrupt | `unset` | allowed: `CopyAsideFirst`, i.e. `<name>.corrupt` |
   | Newer / read failed | `unset` | **no** — `set_root` refused with `library-config-read-only` (ADR-0291 R17, as the profile store does) |

   A newer or unreadable `library.json` makes `set_root` read-only for the rest of the run; the refusal comes before anything is created (TRD §3.1 step 3).
3. **`identity.json`**, same rules:

   | Load result | `daemon_id` this run |
   |---|---|
   | Missing | generate one and write it |
   | Corrupt | copy aside, generate, write |
   | Newer / read failed | a fresh id for this run only, not written, plus a `warn` |

   B3 states the run-only case as 「an unwritable or newer `identity.json`」: a newer file, or one that cannot be read or written, gives a run-only id plus a `warn`.

## 거부한 대안
- None recorded. The step-9a TRD (§8 (b) B3) names no alternative; its reason line gives only the ground for the chosen rule (see 근거).

## 근거
- One rule for every data file (TRD B3).
- A run-only id is the least harm: it only orphans client drafts for that run (TRD B3, K4).
- Step-9a PRD+TRD review (full, Claude lead + codex) PASS on `43f64c4f`, 2026-11-17 (TRD §13).

## 영향 / 불변식
- `status` reports the `library.json` state now: `config` = `ok` (usable, or not written yet) · `read-only` with `why` (newer or unreadable at load) · `corrupt` (to be copied aside before the next write); after a successful `set_root` over a corrupt or missing file it is `ok`. A read-only run is visible here, not only in the `set_root` refusal (TRD §3.1).
- `set_root` on a read-only config answers `Unavailable` · `library-config-read-only` before creating anything (TRD §3.1, §5).
- Risk K4: a run-only `daemon_id` orphans the client's drafts for that run; the `warn` at boot is the signal.
- Tests that must hold (TRD §9 U1): `library.json` load outcomes (missing · usable · corrupt · newer · read failure) and write policies; `set_root` on a read-only config refuses before creating anything; the `identity.json` cases.
- Implemented in step 9a U1 (asset-library `config.rs`; a daemon identity module).
