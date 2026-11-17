# scripts

Scripts that launch the app, builds, and tests (Windows).

Do not launch the app directly from the command shell — if it is launched from the shell, that call is tied to the app's lifetime and the app's output keeps flowing back up into the shell. The launcher starts the app outside the process tree through WMI (`Win32_Process.Create`) and sends the output to files only, so you only need to read the lines you need from the log.

## Launchers

| Launcher | What it does |
|---|---|
| `scripts\run-debug.bat` | Builds the client only + checks the dev server + runs |
| `scripts\rebuild-run-debug.bat` | Rebuilds the daemon and CLI too (after a backend change) + runs |
| `scripts\rebuild-run-debug-log.bat` | Same as above, but runs the app and daemon with `debug` logging |
| `scripts\run-release.bat` | Runs the already-built release |
| `scripts\rebuild-run-release.bat` | Builds the release fresh + runs |

## Builds and tests

Builds and tests are run the same way, outside the process tree through [`run-detached.ps1`](run-detached.ps1), and their output is received only as files — so that you read only the lines you need instead of the whole build log. Usage is in that script's header, and which command runs at which intensity is set by the qa binding ([`.claude/skill-bindings/qa.md`](../.claude/skill-bindings/qa.md)).

## Third-party license notices

| Script | What it does |
|---|---|
| `node scripts/gen-third-party-licenses.mjs <출력 파일>` | Generates the `THIRD_PARTY_LICENSES.txt` that goes into the release zip. Settings are in [`licenses/`](licenses/) |

You will rarely need to run it by hand — `build-release.ps1` runs it before the long build on every release build and writes it into the cargo output directory, and CI runs the same generator on every push and only looks at failures. So **a local release build also needs cargo-about** — the required tool, version, and install command are in the script header. When the notice fails, it is usually the settings in [`licenses/`](licenses/) that need fixing, and the failure message says what to fix.

## Other files

The remaining `*.mjs` and `*.ps1` files are helper scripts that the launchers and builds call, plus development tools. Their purposes are in each file's header.
