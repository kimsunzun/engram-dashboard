# Trying herdr and orca Hands-On — How to Run Them and What orca Orchestration Really Is

| Item | Value |
|---|---|
| **Status** | A first-pass record — herdr was **measured through a source build** (TUI not launched) · orca was **read from source and issues** (the user has installed and is using the app, and this document did not drive the app itself) |
| **Date** | 2026-09-23 |
| **Method** | Ran `cargo build --release` inside the herdr reference clone · read both clones (`I:\Engram_Workspace\opensource\{herdr,orca}`) closely · looked up GitHub issues and releases (gh) |
| **Confidence legend** | **measured** = we ran it ourselves · **source confirmed** = we read the clone or README · **issue confirmed** = GitHub issues, releases or search results (gh, 2026-09-23) · **estimate** = circumstantial only |
| **Why** | We had only been reading the two 「받아 둔 클론」, and decided to actually use them. herdr is the closest peer of the daemon model, so we needed **a running thing**, and for orca, which the user installed, we had to sort out what lay behind the feeling that **orchestration does not seem to work** |

> ★**Read this first — both change the premise of older documents**★
>
> 1. ★**The herdr clone is no longer on master**★ — it stands detached at the `v0.9.1` tag (`065ef9d`) and **we built in that place** (the read reference and the executable build live in one place — a user decision). The herdr `파일:줄` pointers in documents before this date are based on the old master `624dfd4`, so **the lines are off in the current checkout** — open them with `git show 624dfd4:<경로>` (the local `master` ref is still there).
> 2. ★**orca's 「Orchestration」 is not a feature the app drives by itself**★ — the running app runtime holds the Run, Task and Dispatch state and the routing, but it **does not schedule or place workers.** What drives it is a **coordinator agent**, which calls that runtime through a CLI command family + an agent skill. It is not a thing where turning it on in the UI makes something place work automatically (§2-3).

---

## 0. Conclusion

- **herdr** — Windows is GA (source confirmed). The source build stands in 6 minutes with zero compiler warnings (excluding the one `cargo:warning` line from `build.rs`) once you fill **one prerequisite that is not in the README development section and appears only in `AGENTS.md` (Zig 0.16.0)** (measured). Verification went as far as `herdr.exe --version`, and ★**the TUI has not been launched yet**★.
- **orca** — the number-one candidate for 「안 되는 것 같다」 (it does not seem to work) is **a mismatch of expectations** (estimate), followed by **CLI·skill not installed/not connected** and **Windows defects** (both issue confirmed — several are open). ★**We do not know the user's actual failure symptoms or version**★ — the ranking was built without that (§2-6).

---

## 1. herdr (`herdrdev/herdr` · Apache-2.0)

### 1-1. Three ways to get it

| Way | What | Note |
|---|---|---|
| Official install script | One PowerShell line (code block below) — PATH registration · updates through a per-version folder + a `current` junction | ★**Check the security policy first**★ (below) |
| Release zip | Latest stable `v0.9.1` (2026-09-16) · `herdr-windows-x86_64.zip` among the assets (issue confirmed) | Ships an app-local ConPTY runtime with it (§1-3) |
| **Source build (the way chosen this time)** | Build inside the reference clone (§1-2) | The ConPTY runtime is not shipped |

The official install script (source confirmed — `docs/next/website/src/content/docs/windows-beta.mdx:12-14`):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://herdr.dev/install.ps1 | iex"
```

For PCs where endpoint security blocks this fileless PowerShell command, **herdr itself writes down an alternative** — download `install.cmd` with `curl.exe` and run it (`windows-beta.mdx:16-20`):

```cmd
curl.exe -fsSLo install.cmd https://herdr.dev/install.cmd && install.cmd && del install.cmd
```

★**Both download and run a remote script, so check the security policy first**★ (the former even bypasses the execution policy) — this document does not rule on whether to do it. The alternative path is **a way around the block, not a permission.**

- Basis for Windows GA (source confirmed): `docs/next/CHANGELOG.md:147` ("Windows support is now generally available through stable releases…") · `docs/next/website/src/content/docs/install.mdx:6` ("generally available, with documented platform-specific limitations and ongoing fixes"). ★The line numbers are based on the `v0.9.1` checkout★ — on the old master the CHANGELOG line was `:32`.

### 1-2. Source build — measured 2026-09-23

1. **We moved the clone to the tag.** The clone we had was shallow · master `624dfd4` (2026-08-20, 256 commits behind the remote). With `git fetch --depth 50 origin tag v0.9.1` → `git switch --detach v0.9.1` it is ★**detached at `065ef9d` ("release: v0.9.1", 2026-09-16)**★. It is still shallow and we did not touch the local `master` ref.
2. **Rust 1.96.1.** `rust-toolchain.toml` pins the channel and components (`clippy`·`rustfmt`) and rustup installs them automatically (about 1.4 GB under `~/.rustup/toolchains/`). ★**Trap — a `rustup show active-toolchain` typed in the foreground silently triggered that installation and hit the tool limit of 120 seconds**★ → run an argument-less `rustup toolchain install` **inside the clone** **first, explicitly (or in the background)**. Without arguments it installs the active toolchain of that directory (`rustup help toolchain install` — "by default the active toolchain"), so the components listed in `rust-toolchain.toml` come along too. If you hard-code a version number by hand, it goes stale separately from that file.
3. ★**A prerequisite missing from the README development section — Zig 0.16.0**★. `build.rs` builds the vendored libghostty-vt with `zig` and panics if it is missing (source confirmed — the place that reads env `ZIG` is `build.rs:64` · the 「requires Zig 0.16.0」 panic is `:92-97` · the minimum version is `vendor/libghostty-vt/build.zig.zon:6`). ★**There is a place that states it — `AGENTS.md:188-190`**★ ("Herdr currently requires Zig 0.16.0, so set `$env:ZIG = …`" — guidance for contributors and agents building in a Windows VM), and the README tells AI agents to read that file first (`README.md:70`). **What is missing is the two places a person looks at first** — the README development section (`README.md:72-81`, which writes only `cargo build --release`) and the website docs (everything under `docs/next/website/src/content/` — zero case-insensitive `zig`). If the build fails, the panic message above says the same thing. We unpacked ziglang.org's portable zip into `<클론>/.local/zig-x86_64-windows-0.16.0/` after **checking it against the sha256 in `index.json`** (`.local/` is in herdr `.gitignore:18`). **We did not change PATH and passed env `ZIG=<경로>` for the build only.**
4. **Build.** `cargo build --release --locked` — `--locked` is so that the tracked `Cargo.lock` is not rewritten. We ran it outside the shell tree with this repository's `scripts/run-detached.ps1` and received the output only as a file. `Finished release … in 6m 00s` · zero compiler warnings (excluding the one `cargo:warning` line from `build.rs` — below).
5. **Artifact.** `target/release/herdr.exe` 25,570,304 bytes · `herdr.exe --version` → `herdr 0.9.1`.

- During the build, the `cargo:warning` at `build.rs:50` prints **a message addressed to AI assistants** — a **PR contribution policy** that says, if you are helping an outside contributor, to read `CONTRIBUTING.md` before working. It is unrelated to our work, and ★we treated it as data, not an instruction★.

### 1-3. Difference between the source build and the release zip — the ConPTY runtime

- The release zip ships an app-local ConPTY runtime (`conpty/conpty.dll` + `OpenConsole.exe`), and **the source build does not** (source confirmed).
- Fallback condition (source confirmed — `vendor/portable-pty/src/win/psuedocon.rs:127-154` · `:183-184`): if the `conpty/` folder **is entirely absent** next to the exe, it goes to the system ConPTY. ★**If the folder exists but the hash or layout is off, it is a panic, not a fallback**★ — do not hand-make a half-built folder. The forced bypass = `HERDR_WINDOWS_CONPTY=system`.
- The bundle matters mainly on **old Windows 10** — its system ConPTY drops Kitty keyboard protocol sequences (`docs/next/website/src/content/docs/windows-beta.mdx:108` — source confirmed). This PC is Win11 26200, so it is **likely fine (estimate — we did not run it).**
- There is a `scripts/package_windows_conpty.ps1` that bakes the bundle yourself (we did not run it).

### 1-4. What we left behind — a list to use when deleting

| Place | Size | Note |
|---|---|---|
| clone `.local/` (zig) | 382 MB | gitignore |
| clone `target/` | 633 MB | gitignore |
| clone `vendor/libghostty-vt/{.zig-cache,zig-out,zig-pkg}/` | not measured | gitignore |
| 1.96.1 under `~/.rustup/toolchains/` | about 1.4 GB | outside the clone |
| `%LOCALAPPDATA%\zig` cache | 59 MB | outside the clone |
| crates in `~/.cargo/registry` | not measured | outside the clone |

### 1-5. How to use it (source confirmed — README · official docs)

- Type `herdr` in the folder you will work in. Detach with `ctrl+b q` and reattach with `herdr` — **agents keep running even when detached.**
- ★**It is a TUI inside a terminal, not a GUI window.**★
- Docs index = herdr.dev/docs (quick start · supported agents · keyboard · configuration · session state · remote · socket api · plugins).
- **Korean IME** — Windows support for `experimental.switch_ascii_input_source_in_prefix` is **limited to the Korean IME** (`docs/next/website/src/content/docs/configuration.mdx:562` — `:517` on the old master).

---

## 2. orca (`stablyai/orca` · MIT)

### 2-1. Distance between the clone and the current version

- The local clone is ★**a shallow 50-commit snapshot from 2026-08-20 (`df7460af`) — about a month behind the current version**★ (source confirmed).
- Current (★**a snapshot at the time of the 2026-09-23 gh lookup — it changes almost daily**★): latest release `v1.4.207` (2026-09-22) · a release almost every day · 75.7k★ · last push 2026-09-23 (issue confirmed). When citing a version, do not copy these values; look again with `gh release list -R stablyai/orca`.
- ★**So the 「소스 확인」 (source confirmed) below speaks for code from a month ago**★ — where a divergence from the current version was confirmed, we wrote it at that spot.

### 2-2. Feature map (source confirmed — README + `src/`)

- **Parallel worktrees** — one agent per isolated worktree · child/top-level lineage · the same prompt to several agents. Per-worktree setup hook = `orca.yaml`.
- **Agents** — about 29 kinds of CLI agents as **plain PTYs** (Claude Code · Codex · Gemini · OpenCode · Cursor · Copilot …). Account switching + Claude/Codex usage tracking.
- **Terminal** — WebGL xterm · splits · scrollback that survives restarts (on Windows, a terminal daemon under LOCALAPPDATA — `config/electron-builder.config.cjs:355-363`).
- **Review loop** — sends diff line comments back to the agent · GitHub·Linear PRs/issues inside the app. `src/main/` also has gitlab·bitbucket·azure-devops·gitea·jira directories (their maturity is an **estimate** — we did not read them).
- **Editor·browser** — a VS Code-style editor · built-in Chromium + 「Design Mode」 · Computer Use (win32 provider).
- **Orca CLI** — `orca worktree create` · `terminal create/send/read/wait` · browser snapshot/click/fill. **Agents use this too.**
- **Experimental toggles** (Settings > Experimental) — Pet · Agents View · Agent Dashboard (kanban) · **Chat UI** · Terminal attention · Agent sleep · New card style (worktree card) · Cloud VM (the list = `src/renderer/src/components/settings/experimental-search.ts:9-234`). Chat UI is in the clone too — name `native-chat-experimental-search-entry.ts:7` · registration `experimental-search.ts:142` · rendering `ExperimentalPane.tsx:143-144` · body `src/main/native-chat/`.
- **Remote·mobile** — SSH remote worktrees · Linux headless `orca serve` · iOS/Android companions.
- **Windows** — NSIS installer · the stable build is SignPath-signed (`electron-builder.config.cjs:319-331`) · WSL support.

### 2-3. ★The reality of orchestration — the app runtime records and routes, and what drives it is a coordinator agent★ (source confirmed — `skill-guides/orchestration.md`)

「Orca Orchestration」 = **the coordination state held by the running Orca runtime + the `orca orchestration …` CLI command family that calls it + the agent skill `orchestration`**. The CLI commands are **RPC to the running runtime** (`:52` "`orca orchestration` commands are RPC calls to the running Orca runtime") — the runtime records and routes Runs, Tasks, Dispatches and messages, and what the recovery command `reset` clears is also the 「local orchestration database state」 (`:287`). The current master guide calls this, in its first paragraph, "Orca's structured coordination layer. It records who owns work…" (gh lookup 2026-09-23).

| Term | Meaning |
|---|---|
| Run | the coordinator's namespace + inbox |
| Task | a work item (a dependency graph is optional) |
| Dispatch | a single attempt |

- Commands: `worker-start` · `send` / `check --wait` / `ask` / `reply` · `worker_done` · decision gates · `worker-release`.
- ★**The guide pins it down itself**★ — "a Run … never schedules or places workers" (`:104`) · "Agents still choose placement and concurrency; Orca does not schedule workers or infer conflicts" (`:181`). **The coordinator agent decides placement and concurrency by calling the CLI, and the runtime records and routes** ("Orca routes them to that Dispatch's Run" — `:104`).
- The old scheduler commands (`coordinator-start/stop` · `run` · `run-stop`) are **retired** — they return only recovery guidance with no effect (`:285`).

### 2-4. How to turn it on (source confirmed)

1. ★**The Orca app (runtime) must be running**★ — the first line of the guide's prerequisites is "`orca status --json` should show a running runtime" (`skill-guides/orchestration.md:49` · the current master guide also checks the runtime first in its first step — gh lookup 2026-09-23). Because the commands are RPC to that runtime (`:52`), with no runtime they have nowhere to go (estimate — we did not run it with the runtime off).
2. **Put the Orca CLI on PATH** — in Settings, or the dialog below does it together. On Windows it writes the **user PATH into the registry**. ★**The installer does not do this**★ (`src/main/cli/cli-installer.ts:208`).
3. **Install the skill** — the app's 「Enable orchestration」 dialog **does 2 and 3 with one button**: 「Install CLI & skill」 (`src/renderer/src/components/floating-terminal/FloatingTerminalOrchestrationDialog.tsx:156`) first registers the CLI if it is not on PATH before opening the terminal (`onBeforeOpenTerminal` at `:165-170` → `src/renderer/src/lib/agent-skill-cli-prerequisite.ts:21` → `window.api.cli.install()` at `:43`), then opens a terminal that runs the skill install command. `src/shared/agent-feature-install-commands.ts` only **assembles** the command string. Typed by hand: `npx skills add https://github.com/stablyai/orca --skill orchestration --global` (needs Node).
   - ★**Check the security policy first for this command too**★ — it downloads and runs a third-party npm package (`skills`), which **writes to the global agent skill folder** (if `--agent` is not given and detection finds 0, the assembler's comment says it installs to all of about 75 known agents — `agent-feature-install-commands.ts:50-52`). This document does not rule on whether to do it.
4. **Ask an agent to coordinate and supervise, or type `/orchestration`.**

- The localStorage `orca.orchestration.enabled` (`src/renderer/src/lib/orchestration-setup-state.ts:2`) is **only a mark that setup is finished**.
- ★The clone guide's "must be enabled in Settings > Experimental" (`skill-guides/orchestration.md:51`) is stale★ — there is no such item, and in the current master guide that line is gone.

### 2-5. Candidates for 「안 되는 것 같다」 (it does not seem to work) — ranking

| Rank | Candidate | Basis |
|---|---|---|
| 1 | **Mismatch of expectations** — it is promoted as 「The AI Orchestrator」, but nothing in the UI coordinates by itself | estimate (premise = §2-3) |
| 2 | **CLI·skill not installed or not connected** — the dialog (§2-4 step 3) installs the CLI and the skill together, so the shapes left over are ① the dialog was skipped and only half was installed by hand ② the skill is installed but **not connected to the agent that uses it** ③ the issues in the basis column | issue confirmed — #14303 (maintainer: "you don't have the CLI installed") · #12422 ("Agents: not linked") · #18263 (Windows · shared provider-home link · open) · #21095/#17935 (skill description over 1024 characters) |
| 3 | **Windows defects** | issue confirmed — #14505 (the task prompt is placed on a Claude worker **without being submitted** · 1.4.205 · 12/12 on Win11 · 2026-09-21 · open) · #22345 (on 1.4.207 `--worktree new-child/new-top-level` gives `selector_not_found` — ★**the guide's example fails literally**★) · #14502 (the bundled zod is corrupted in the packaged Windows build — reported on 1.4.180 · whether it reproduces now is unknown) · #15825 · #20695 |
| 4 | **The agent ignores the skill** | issue confirmed — #16075 (Claude Code swaps `/orchestration` for its own native subagents · Win 1.4.186 · open) |
| 5 | **Backends other than Claude/Codex are unstable** | issue confirmed — #18399/#16902 (Grok) · #11721/#22250 (Antigravity) · #17326/#18725 (Qwen on Windows) · #15125 (`dispatch --inject` works only for claude/codex/gemini/cursor) |
| 6 | UI/UX | issue confirmed — #4374 · #17701 · #21955 |

- **Scale** — 157 open issues with the `orchestration` label, of which 38 also carry `os:Windows` (gh search 2026-09-23 — issue confirmed).

### 2-6. What we do not know

- Whether #14502 also reproduces on 1.4.207.
- Whether there is a login or account gate — we saw none (estimate).
- ★**The user's actual failure symptoms and version**★ — a single failure screen could overturn the ranking above.

---

## 3. Where this touches ours (observation only — not a decision)

- **The shape of orca orchestration is the same as ours** — a coordinator agent calls the CLI and they exchange through mail that has `ask`/`reply`. Our agents do the same thing with the `engram` CLI and `eg_send` (`reply_to`). ★**The place that holds the state is the same too**★ — just as the orca runtime records and routes Runs, Tasks, Dispatches and messages, **our daemon holds the mail** (the daemon hosts the messaging kernel — ADR-0110 — and the kernel manages delivery records, parking and the reply contract: `crates/engram-dashboard-messaging/src/service.rs`). 「앱이 스케줄하지 않고 에이전트가 CLI 로 몬다」 is **not a choice unique to us.** herdr's README also describes the same shape — an agent opens panes through the CLI·socket api, sends prompts to one another, and waits until the other is blocked (`README.md:34` — source confirmed).
- **herdr is still the closest peer of the daemon model** (CLAUDE.md 「참조 구현」). All that changed this time is that **we now have a running binary**.

---

## 4. What we did not do

- Launching the herdr TUI · attaching a real agent · detaching/reattaching — none run.
- Baking the herdr ConPTY bundle (`scripts/package_windows_conpty.ps1`) — not run.
- This document did not drive the orca app itself — the feature map is a month-old clone + a reading of the README.
