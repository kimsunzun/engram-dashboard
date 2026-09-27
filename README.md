# Engram Dashboard

여러 AI 에이전트(Claude Code·Codex)를 동시에 띄워 관리하는 Windows 데스크톱 앱입니다. 에이전트를 돌리는 데몬과 화면을 그리는 클라이언트가 분리되어 있어, 창을 닫아도 에이전트는 계속 돕니다.

![Platform](https://img.shields.io/badge/platform-Windows%20x64-blue) ![Release](https://img.shields.io/github/v/release/kimsunzun/engram-dashboard) ![Status](https://img.shields.io/badge/status-WIP-orange) ![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB) ![React](https://img.shields.io/badge/React-19-61DAFB) ![Rust](https://img.shields.io/badge/Rust-stable-DEA584)

> 개발 중입니다. Windows 전용입니다.

## 시연 영상

<p align="center">
  <a href="https://youtu.be/qK6lqWbdRrc">
    <img src="https://img.youtube.com/vi/qK6lqWbdRrc/hqdefault.jpg" alt="Engram Dashboard 시연 영상" width="640">
  </a>
  <br>
  <em>▶ 시연 영상 (약 5분 31초)</em>
</p>

| 구간 | 내용 |
|---|---|
| [0:00](https://youtu.be/qK6lqWbdRrc) | 에이전트 띄우기와 화면 배치 — Claude Code와 Codex를 터미널·채팅 두 방식으로 띄우고, 창을 열어 슬롯을 나눠 배치 |
| [2:45](https://youtu.be/qK6lqWbdRrc?t=165) | 데몬 테스트 |
| [3:37](https://youtu.be/qK6lqWbdRrc?t=217) | 오케스트레이션 |

## 주요 기능

- **화면 배치** — 창을 나눈 칸마다 에이전트를 띄우고, 칸을 별도 창으로 떼거나 탭으로 여러 배치를 오갑니다
- **터미널 또는 채팅** — 에이전트 출력을 터미널 그대로 보거나, JSON 출력을 받아 채팅 화면으로 봅니다
- **Claude Code·Codex** — 두 종류의 에이전트를 한 화면에 나란히 띄웁니다
- **에이전트가 직접 조작** — 에이전트가 `engram` CLI로 동료를 깨우고, 새 에이전트를 만들고, 화면 배치까지 바꿉니다
- **에이전트 간 메시징** — 상대가 작업 중이면 데몬이 메시지를 맡아두었다가 손이 비는 시점에 전달합니다

## 설치·실행

[Releases 페이지](https://github.com/kimsunzun/engram-dashboard/releases/latest)에서 `engram-dashboard-*-windows-x64.zip`을 받아 압축을 풀고 `engram-dashboard.exe`를 실행하면 됩니다. 설치 과정은 없고, 아래만 미리 준비하면 됩니다.

- Windows 10 또는 11 (x64)
- **Claude Code 설치 및 로그인** — `claude` 명령이 `PATH`에 있어야 합니다
- **Codex 설치 및 로그인**(선택) — Codex 에이전트를 쓸 때만 필요합니다. `codex` 명령이 `PATH`에 있어야 합니다
- **WebView2 런타임** — 최근 Windows에는 기본 포함되어 있지만, 없는 환경(LTSC·N 에디션 등)에서는 창이 뜨지 않습니다. [Microsoft 배포 페이지](https://developer.microsoft.com/microsoft-edge/webview2/)에서 받으세요

`claude`·`codex` 명령이 없으면 에이전트가 원인 안내 없이 뜨지 못합니다(안내 메시지가 아직 없습니다).

**창을 닫아도 앱은 종료되지 않습니다.** 트레이로 내려갈 뿐이고 데몬과 에이전트는 계속 돕니다. 완전히 끄려면 트레이 아이콘 메뉴에서 **「완전 종료」**를 고르세요 — 실행 중인 에이전트도 함께 내려갑니다.

명부와 프리셋은 실행파일 옆 `data\` 폴더에 저장됩니다. 지우거나 옮기려면 먼저 앱을 완전히 종료하세요 — 켜져 있는 동안에는 데몬이 폴더를 붙들고 있습니다.

코드 서명을 하지 않아서 첫 실행 때 SmartScreen 경고가 뜰 수 있습니다.

## 개발

Node.js 22.12+(테스트가 플래그 없는 `require(ESM)`에 의존합니다) · Rust stable · Windows가 필요합니다.

```bash
git clone https://github.com/kimsunzun/engram-dashboard.git
cd engram-dashboard
npm install
scripts\rebuild-run-debug.bat            # 데몬·클라이언트 빌드 + dev 서버 + 앱 실행까지 한 번에
```

런처 목록과 빌드·테스트를 돌리는 방법은 [scripts/README.md](scripts/README.md)에 있습니다.

## 문서

- [아키텍처 개요](docs/reference/architecture-overview.md) — 전체 구조·crate 구성
- [문서 인덱스](docs/README.md) — 설계 결정·개발 기록을 포함한 전체 문서

## 라이선스

Copyright © 2026 kimsunzun. All rights reserved.

오픈소스 라이선스는 정식 공개 시점에 부여할 예정입니다. 그 전까지 저장소 내용에는 기본 저작권 규칙이 적용되며, 외부 기여(PR)는 받지 않습니다.
