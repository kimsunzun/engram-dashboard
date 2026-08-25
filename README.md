# Engram Dashboard

여러 AI 에이전트를 동시에 띄워 관리하는 Windows 데스크톱 앱입니다. 에이전트를 돌리는 데몬과 화면을 그리는 클라이언트가 분리되어 있어, 창을 닫아도 에이전트는 계속 돕니다.

![Platform](https://img.shields.io/badge/platform-Windows%20x64-blue)
![Release](https://img.shields.io/badge/release-v0.2.0-brightgreen)
![Status](https://img.shields.io/badge/status-WIP-orange)
![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB)
![React](https://img.shields.io/badge/React-19-61DAFB)
![Rust](https://img.shields.io/badge/Rust-stable-DEA584)

> 개발 중입니다. 현재는 Windows와 Claude Code만 지원합니다.

<p align="center">
  <a href="https://youtu.be/40I0b4Sf5cI">
    <img src="https://img.youtube.com/vi/40I0b4Sf5cI/hqdefault.jpg" alt="Engram Dashboard 시연 영상" width="640">
  </a>
  <br>
  <em>▶ 시연 영상 (약 5분 25초)</em>
</p>

| 구간 | 내용 |
|---|---|
| [0:00](https://youtu.be/40I0b4Sf5cI) | 클로드코드 JSON · 터미널 띄우기 |
| [0:36](https://youtu.be/40I0b4Sf5cI?t=36) | 창과 슬롯 배치 |
| [2:10](https://youtu.be/40I0b4Sf5cI?t=130) | 데몬 테스트 |
| [2:59](https://youtu.be/40I0b4Sf5cI?t=179) | 에이전트 제어와 오케스트레이션 |

## 현재 구현된 것

- **화면 배치** — 창을 가로·세로로 나누면 칸이 생기고, 칸마다 에이전트를 하나씩 띄웁니다. 칸은 별도 창으로 떼어낼 수 있고, 탭을 여러 개 두면 서로 다른 배치를 오갈 수 있습니다
- **터미널 또는 채팅** — 에이전트 출력을 터미널 그대로 볼지, JSON 출력을 받아 채팅 화면으로 볼지 만들 때 고릅니다
- **에이전트가 직접 조작** — `engram` CLI로 명부를 읽고, 동료를 깨우고, 새 에이전트를 만들고, 화면 배치까지 바꿉니다. 사람만 쓸 수 있는 조작을 따로 만들지 않는 것이 설계 원칙입니다
- **에이전트 간 메시징** — 서로 메시지를 주고받고, 상대가 작업 중이면 데몬이 맡아두었다가 손이 비는 시점에 전달합니다

## 받아서 실행하기

[Releases 페이지](https://github.com/kimsunzun/engram-dashboard/releases/latest)에서 `engram-dashboard-*-windows-x64.zip`을 받아 압축을 풀고 `engram-dashboard.exe`를 실행하면 됩니다. 설치 과정은 없습니다.

미리 준비해야 하는 것:

- Windows 10 또는 11 (x64)
- **Claude Code 설치 및 로그인** — `claude` 명령이 `PATH`에 있어야 합니다. 없으면 에이전트를 띄울 때 원인을 알기 어려운 오류가 납니다(안내 메시지가 아직 없습니다)
- **WebView2 런타임** — 최근 Windows에는 기본 포함되어 있지만, 없는 환경(LTSC·N 에디션 등)에서는 창이 뜨지 않습니다. [Microsoft 배포 페이지](https://developer.microsoft.com/microsoft-edge/webview2/)에서 받으세요

**창을 닫아도 앱은 종료되지 않습니다.** 트레이로 내려갈 뿐이고 데몬과 에이전트는 계속 돕니다. 완전히 끄려면 트레이 아이콘 메뉴에서 **「완전 종료」**를 고르세요 — 실행 중인 에이전트도 함께 내려갑니다.

코드 서명을 하지 않아서 첫 실행 때 SmartScreen 경고가 뜰 수 있습니다.

## 개발

Node.js 22.12+와 Rust stable 툴체인이 필요합니다(테스트가 플래그 없는 `require(ESM)` 에 의존합니다 — 그 아래 버전에서는 `npm test` 가 깨집니다. CI·로컬 실사용은 Node 24).

```bash
git clone https://github.com/kimsunzun/engram-dashboard.git
cd engram-dashboard
npm install
scripts\rebuild-run-debug.bat            # 데몬·클라이언트 빌드 + dev 서버 + 앱 실행까지 한 번에
```

**실행은 `scripts/`의 런처로 합니다**(Windows). 앱을 셸에서 직접 띄우지 않습니다 — 그 호출이 앱 수명에 매달리고 앱 출력이 셸로 계속 거슬러 올라옵니다. 런처는 작업 스케줄러로 앱을 프로세스 트리 밖에 띄우고 출력을 파일로만 보내므로, 로그에서 필요한 줄만 읽으면 됩니다.

| 런처 | 하는 일 |
|---|---|
| `scripts\run-debug.bat` | 클라이언트만 빌드 + dev 서버 확인 + 실행 |
| `scripts\rebuild-run-debug.bat` | 데몬까지 재빌드(백엔드 수정 후) + 실행 |
| `scripts\rebuild-run-debug-log.bat` | 위와 같되 앱·데몬을 `debug` 로그로 실행 |
| `scripts\run-release.bat` | 이미 빌드된 릴리즈 실행 |
| `scripts\rebuild-run-release.bat` | 릴리즈 새로 빌드 + 실행 |

```bash
# 빌드·테스트도 앱과 같은 규칙 — 셸에서 직접 돌리지 않고 scripts/run-detached.ps1 로 프로세스 트리 밖에서 돌리고 출력은 파일로만 받습니다(빌드 로그 전체를 삼키지 않고 필요한 줄만 읽습니다).
#   완료 판정 = 로그 마지막 줄의 `__EXIT=<코드>`(프로세스가 사라진 것으로 판정하지 않습니다). 규칙 = CLAUDE.md 「빌드·검증 명령」, 사용법 = scripts/run-detached.ps1 헤더
# src-tauri만 제외(그 패키지의 단위 스위트에 알려진 실패가 남아 있어서입니다 — 제품 결함이 아니라 하네스 부패, 별도 수정 대기. 옛 사유였던 Windows 크래시는 2026-08-24에 해소됐습니다. 수치·현황·별도 실행 명령의 정본 = CLAUDE.md 「빌드·검증 명령」의 lib_unit 줄) · 실행 중인 데몬이 있으면 먼저 종료(파일 잠금)
#   `-- --test-threads=4` 는 로컬 전용이며 빼지 마세요(근거 = 같은 절). CI는 그것을 쓰지 않으며 그 차이가 의도입니다.
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "cargo test --workspace --exclude engram-dashboard -- --test-threads=4" -WorkDir . -LogFile test.log
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "npm test" -WorkDir . -LogFile vitest.log
#   ↑ vitest 는 `__EXIT` 가 안 붙습니다(자식이 래퍼보다 오래 삽니다) — 로그에 찍힌 vitest 자신의 pass/fail 요약으로 판정하세요.
```

## 문서

- [아키텍처 개요](docs/reference/architecture-overview.md) — 전체 구조·crate 구성
- [문서 인덱스](docs/README.md)
- [설계 결정 기록](docs/decisions/)
- [개발 진행 기록](docs/process/step-log.md)

## 라이선스

Copyright © 2026 kimsunzun. All rights reserved.

오픈소스 라이선스는 정식 공개 시점에 부여할 예정입니다. 그 전까지 저장소 내용에는 기본 저작권 규칙이 적용되며, 외부 기여(PR)는 받지 않습니다.
