# ADR-0137: A separate bundle identifier for the dev build — running alongside the release app

- 상태: 확정 (2026-08-16, grounds: measured + plugin source check 2.4.2/2.4.3) · 부분 폐기 by ADR-0283 (the WebView2 data folder cost statement in 영향)
- 관련: ADR-0134(single-instance scope) · ADR-0135(lock file = connection key) · `src-tauri/tauri.dev.conf.json` · `scripts/tauri-cli.mjs` · step-log S21 · Amended by ADR-0283 (the WebView2 data folder cost statement in 영향)

## 맥락

When the dev app was launched while the release app was running, **the dev process exited immediately** — no window appeared and its own daemon did not start either (the dev-side connection key file was byte-identical before and after the run. Measured 2026-08-16).

The cause is that `tauri-plugin-single-instance` builds the Windows global mutex name **from the bundle identifier** (`{identifier}-sim`). When two builds use the same identifier they grab the same mutex, so the one that starts later is decided to be "already running" and kills itself. We checked the plugin source (`platform_impl/windows.rs`, 2.4.2 and 2.4.3): the Builder exposes only `callback` and the **Linux-only** `dbus_id`, so there is **no way to swap the mutex name on Windows** — splitting the identifier is the only lever.

## 결정

1. **The dev build uses the identifier `com.engram.dashboard.dev`.** The release identity `com.engram.dashboard` stays as it is.
2. **Means = a config overlay file** (`src-tauri/tauri.dev.conf.json`). It overrides only the dev values without touching the release config.
3. **Injection is done by `scripts/tauri-cli.mjs` for the `dev` subcommand only.** `build` is passed through so the distribution identity can never change.

## 거부한 대안

- **★Switch the single-instance key to the data folder (the originally preferred direction)★** — investigation showed **the key cannot be overridden**. Doing it would mean reimplementing the whole plugin mechanism, including the behavior that brings the window to the front. Grounds for rejection = plugin source check (no name override on the public surface).
- **Leave it as is** — developing requires closing the release app. Worse, **nothing on screen explains it** — the second app simply vanishes, so each person rediscovers this fact from scratch. Grounds for rejection = measured (silent exit).
- **Remove the single-instance plugin** — two windows attach to the same data folder and share one daemon. The tray and window ownership assumptions are shaken wholesale. Grounds for rejection = logic (the per-folder exclusion that ADR-0134/0135 established collapses).
- **Lean on the plugin's `semver` feature** (mix the package version into the mutex name) — **it does not help at all if the versions of the two builds are the same**. That is exactly our case. Grounds for rejection = invalid by the feature's definition.

## 근거

- **The official Tauri docs present a per-flavour config overlay in exactly this pattern** — it is not a structure we invented.
- **Mature apps ship their stable/insider channels under separate identifiers** — it is the standard way to let them coexist on the same machine.
- Dev and release are **independent distributions that differ starting from the data folder** (discovery `default_data_dir`). Splitting the identifier acknowledges in the name too what is already split.

## 영향 / 불변식

- **The identifier also decides the WebView2 data folder** → the dev build's stored web state is **reset once** (an intended cost). The daemon data folder is unrelated to the identifier and stays as it is.
  - ★(2026-10-08 revision note — ADR-0283) This is no longer so★ — the WebView2 data folder is `webview\` under the data root (dev = `.engram-dev\webview`) and is unrelated to the identifier. Only a run that cannot write to that folder falls back to the identifier's Tauri default location (`%LOCALAPPDATA%\<identifier>`).
- **OS integrations keyed on the identifier (deep links, notifications, file associations) treat dev and release as different apps** — that is the purpose of this decision.
- **★Remaining open issue (intentional):★** **two release distributions** unpacked into different folders still share the same identifier and block each other. This decision opens only the dev↔release axis.
- **If the overlay leaks into `build`, the distribution identity changes silently** — the subcommand decision and the `--` boundary decision in `scripts/tauri-cli.mjs` are that wall. CI's `build-release.ps1` also goes through this wrapper.
- **Constraints on the identifier value** (written into the overlay comment): do not end it with `.app` (macOS mistakes the directory for an app — tauri#12674, the CLI rejects it since PR#13618) · no single token without a dot (Linux crash — tauri#10353).
- **This decision makes running dev and release together the normal state** → the blast radius of tools that kill processes by image name grows. ADR-0139 is the consequence of that.
