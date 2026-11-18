# Engram Dashboard

A Windows desktop app that launches and manages several AI agents (Claude Code·Codex) at the same time. The daemon that runs the agents and the client that draws the screen are separate, so the agents keep running even when you close the window.

![Platform](https://img.shields.io/badge/platform-Windows%20x64-blue) ![Release](https://img.shields.io/github/v/release/kimsunzun/engram-dashboard) ![Status](https://img.shields.io/badge/status-WIP-orange) ![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB) ![React](https://img.shields.io/badge/React-19-61DAFB) ![Rust](https://img.shields.io/badge/Rust-stable-DEA584)

> Under development. Windows only.

## Demo video

<p align="center">
  <a href="https://youtu.be/khL5L42PTcY">
    <img src="https://img.youtube.com/vi/khL5L42PTcY/hqdefault.jpg" alt="Engram Dashboard demo video" width="640">
  </a>
  <br>
  <em>▶ Demo video (about 5 min 15 s)</em>
</p>

| Segment | Content |
|---|---|
| [0:00](https://youtu.be/khL5L42PTcY) | Launching agents and arranging the screen — launch Claude Code and Codex in both terminal and chat modes, open a window and divide it into slots to arrange them |
| [2:34](https://youtu.be/khL5L42PTcY?t=154) | Daemon test |
| [3:37](https://youtu.be/khL5L42PTcY?t=217) | Orchestration |

## Main features

- **Screen layout** — launch an agent in each area you split the window into, detach an area into a separate window, or switch between several layouts with tabs
- **Terminal or chat** — view the agent output as a terminal as it is, or receive JSON output and view it on a chat screen
- **Claude Code·Codex** — launch both kinds of agents side by side on one screen
- **Agents operate directly** — through the `engram` CLI, an agent wakes a colleague, creates a new agent, and even changes the screen layout
- **Messaging between agents** — if the other party is working, the daemon holds the message and delivers it when they are free

## Install and run

Download `engram-dashboard-*-windows-x64.zip` from the [Releases page](https://github.com/kimsunzun/engram-dashboard/releases/latest), extract it, and run `engram-dashboard.exe`. There is no installation step; you only need to prepare the following beforehand.

- Windows 10 or 11 (x64)
- **Claude Code installed and logged in** — the `claude` command must be on `PATH`
- **Codex installed and logged in** (optional) — needed only when you use Codex agents. The `codex` command must be on `PATH`
- **WebView2 runtime** — included by default in recent Windows, but in environments without it (LTSC, N editions, etc.) the window does not appear. Download it from the [Microsoft download page](https://developer.microsoft.com/microsoft-edge/webview2/)

If the `claude`·`codex` command is missing, an agent cannot launch, with no message explaining the cause (there is no guidance message yet).

**The app does not exit when you close the window.** It only drops to the tray, and the daemon and agents keep running. To turn it off completely, choose **「완전 종료」** from the tray icon menu — the running agents go down with it.

The registry and presets are stored in the `data\` folder next to the executable. To delete or move it, fully quit the app first — while it is running, the daemon holds the folder.

The code is not signed, so a SmartScreen warning may appear on first launch.

## Development

Node.js 22.12+ (the tests depend on flagless `require(ESM)`) · Rust stable · Windows are required.

The release build (`launch\빌드.bat` · `scripts/build-release.ps1`) additionally needs cargo-about 0.9.2 (`cargo install cargo-about --locked --features cli --version 0.9.2`) and `npm install` — see [scripts/README.md](scripts/README.md). Node 22.22.2+ or 24.15+ is recommended (anything lower only gives an engine warning at install time).

```bash
git clone https://github.com/kimsunzun/engram-dashboard.git
cd engram-dashboard
npm install
scripts\rebuild-run-debug.bat            # 데몬·클라이언트 빌드 + dev 서버 + 앱 실행까지 한 번에
```

The list of launchers and how to run builds and tests are in [scripts/README.md](scripts/README.md).

## Documentation

- [Architecture](docs/reference/architecture.md) — modules, connections, runtime invariants
- [Documentation index](docs/README.md) — all documents, including design decisions and development logs

## License

Copyright © 2026 kimsunzun. All rights reserved.

The open-source license will be granted at the time of the official release. Until then the default copyright rules apply to the repository contents, and external contributions (PRs) are not accepted.
