# Open-source license notice file — peer approaches and tools

- **Status:** light (2 collectors · no adversarial review) · survey for T-62 (`docs/tracking.md`) · **decision (user 2026-10-15) = extract by hand and commit + add to the build-release manifest · updates also by hand · automation later**
- **Date:** 2026-10-15
- **Method:** 2 collectors (① how peer apps actually do it ② comparison of Rust · JS tools) · main grounding (Zed script original · cargo-about config docs · generate-license-file README).
- **Trigger:** the distribution (portable zip) has no third-party license notice. There are two dependency sets, Cargo and npm.

## Conclusion

1. **The standard on the Rust side is cargo-about.** It extracts the full license text from a config (`about.toml`) + a template. `accepted` = "Priority list of all the accepted licenses for a project" — a license outside the list makes `--fail` fail. `ignore-dev-dependencies` · `ignore-build-dependencies` remove development dependencies (config docs original — checked by main). **Likely.**
   - **Zed** — `script/generate-licenses` runs cargo-about (pinned to 0.8.2) as `cargo about generate --fail -c script/licenses/zed-licenses.toml script/licenses/template.md.hbs` to make `assets/licenses.md`, and prepends the theme · icon licenses · it fails if even one line appears on stderr (turned off by `ALLOW_MISSING_LICENSES`) (script original — checked by main). The bundle script runs this first and puts it in the app, and the app has an 「라이선스 열기」 (open licenses) action (collector · **likely**). A comment "AGPL should not be added" in the allowlist (collector).
   - **cargo-deny** does **only a check** and does not create a notice (an allowlist gate) — GitButler runs it in CI (collector · **likely**).
2. **On the npm side, generate-license-file fits best.** "Generates a text file containing all of the licences for your production dependencies" · `npx generate-license-file --input package.json --output third-party-licenses.txt --overwrite` · replace · exclude · append through config (README original — checked by main). ISC · 4.2.5 (2026-08-29 — collector). **Likely.** license-checker-rseidelsohn is effectively finished (the successor `@lizenz/checker` is 0.0.x).
3. **Most Tauri peers do not ship a notice file (by file-name search).** Not found in Spacedrive (the root `NOTICE` is only its own notice · About links to its own LICENSE) · Cap · Clash Verge Rev. The small Tauri app vmark has `scripts/gen-third-party-licenses.mjs` make `THIRD_PARTY_LICENSES.txt` with cargo-about (`src-tauri/about.toml`) + its own npm script (collector · **uncertain** — a file made only at release time is not caught by a name search).
4. **VS Code (for contrast)** — commits `ThirdPartyNotices.txt` at the root and ships it in the app. Generation is an in-house pipeline, so it cannot be reproduced from outside (collector).
5. **The handling of the NOTICE file of Apache-2.0 is in no tool's docs** — we have to separately find which actual dependencies have a NOTICE file and check.
6. We could not find third-party license guidance in the official Tauri docs (collector · **likely**).

## If we carry this to Engram (session interpretation — the decision is the user's)

- cargo-about (Rust · excluding dev/build dependencies · allowlist + `--fail`) + generate-license-file (npm production dependencies) → merged into one file `THIRD_PARTY_LICENSES.txt` and **committed to the repository** (user 「텍스트 파일 하나 추가」 (add one text file)) → add that file to the `scripts/build-release.ps1` manifest so it is shipped in the zip (the manifest allows only an exact set — ADR-0100).
- When dependencies change, extract again with the same command and commit. A CI check of 「커밋된 파일이 최신인가」 (is the committed file up to date) comes later.
- After generating, separately find the dependencies that have a NOTICE file and check.

## Limits

- No adversarial review. Because of the GitHub API limit, we could not check whether Zed commits `assets/licenses.md`, or the actual shipped artifacts of GitButler · Clash Verge. A notice made only at release time is not caught by a file-name search.
