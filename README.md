# Engram Dashboard

여러 Claude Code 에이전트를 한 화면에서 띄우고 관리하는 Windows 데스크톱 앱입니다.

![Platform](https://img.shields.io/badge/platform-Windows%20x64-blue)
![Release](https://img.shields.io/badge/release-v0.2.0-brightgreen)
![Status](https://img.shields.io/badge/status-WIP-orange)
![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB)
![React](https://img.shields.io/badge/React-19-61DAFB)
![Rust](https://img.shields.io/badge/Rust-stable-DEA584)

> 개발 중입니다. Windows와 Claude Code만 지원하며 내부 API는 예고 없이 바뀔 수 있습니다.

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

## 무엇을 하는가

- **여러 에이전트를 한 화면에.** 화면을 가로·세로로 나눠 배치하고, 칸 하나를 별도 창으로 떼어냅니다. 탭으로 화면 구성을 여러 벌 두고 오갑니다
- **출력은 두 가지로.** 터미널(xterm) 또는 구조화된 채팅 중 만들 때 고릅니다
- **에이전트 명부와 트리.** 부모-자식으로 묶어 관리하고, 아직 띄우지 않은 에이전트를 미리 등록해두었다가 필요할 때 깨웁니다
- **닫아도 죽지 않습니다.** 창을 닫으면 트레이로 내려갈 뿐 에이전트는 계속 돕니다. 다시 열면 최근 출력이 되감겨 이어집니다
- **에이전트가 직접 조작합니다.** `engram` CLI로 명부를 읽고, 동료를 깨우고, 새 에이전트를 만들고, 화면 배치까지 바꿉니다. 사람만 쓸 수 있는 조작을 따로 만들지 않는 것이 설계 원칙입니다
- **에이전트 간 메시징.** 서로 메시지를 보내고, 상대가 작업 중이면 데몬이 맡아두었다가 손이 비는 시점에 전달합니다

## 받아서 실행하기

[Releases 페이지](https://github.com/kimsunzun/engram-dashboard/releases/latest)에서 `engram-dashboard-*-windows-x64.zip`을 받아 압축을 풀고 `engram-dashboard.exe`를 실행하면 됩니다. 설치 과정은 없습니다.

미리 준비해야 하는 것:

- Windows 10 또는 11 (x64)
- **Claude Code 설치 및 로그인** — `claude` 명령이 `PATH`에 있어야 합니다. 없으면 에이전트를 띄울 때 원인을 알기 어려운 오류가 납니다(안내 메시지가 아직 없습니다)
- **WebView2 런타임** — 최근 Windows에는 기본 포함되어 있지만, 없는 환경(LTSC·N 에디션 등)에서는 창이 뜨지 않습니다. [Microsoft 배포 페이지](https://developer.microsoft.com/microsoft-edge/webview2/)에서 받으세요

**압축을 푼 폴더의 내용물을 흩뜨리지 마세요.** 실행파일 셋과 `prompts/` 폴더가 서로를 파일 위치로 찾습니다 — `PATH`를 뒤지지 않습니다. 폴더째 옮기거나 바로가기를 만드는 것은 괜찮지만, 실행파일 하나만 빼내면 데몬이나 제어 CLI를 못 찾고 에이전트 간 메시징이 조용히 동작하지 않습니다.

명부와 프리셋은 실행파일 옆 `data\` 폴더에 저장됩니다. **지우거나 옮기기 전에 앱을 완전히 종료하세요** — 켜져 있는 동안 데몬이 `data\daemon.json`을 붙들고 있어 삭제도 이동도 거부됩니다. 쓰기 권한이 없는 곳(`C:\Program Files` 등)에 풀면 이 폴더를 만들 수 없어 데몬이 기동하지 않습니다. 클라우드 동기화 폴더(OneDrive·Dropbox 등)도 피하세요 — 데몬이 켤 때마다 새로 쓰는 접속 토큰이 그 서비스의 버전 기록에 그대로 쌓입니다.

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

## 문제가 생겼을 때

**로그를 켜고 재현합니다.**

```bash
scripts\rebuild-run-debug-log.bat
```

기본(`warn`)에서는 에이전트가 stdout에 실제로 무엇을 썼는지가 기록되지 않아, 「턴이 끝났는데 화면이 놓친 것」인지 「턴이 아예 안 끝난 것」인지를 나중에 가릴 수 없습니다. 이 런처는 앱과 데몬 양쪽을 `debug`로 띄웁니다.

**로그는 이 창이 아니라 파일에 있습니다.** 앱이 분리 실행되기 때문입니다. 런처가 출력하는 `LOG=<경로>`를 보세요 — 실행할 때마다 새 파일이 생깁니다. 데몬은 자기 로그를 따로 남깁니다.

| 무엇 | 어디 |
|---|---|
| 앱 로그 | 런처가 출력하는 `LOG=` 경로 |
| 데몬 로그 (개발) | 저장소 루트 `.engram-data\logs\` |
| 데몬 로그 (릴리즈) | 실행파일 옆 `data\logs\` |

`debug`는 출력이 많습니다. 한 가지를 재현하는 동안에만 쓰고, 평소에는 `rebuild-run-debug.bat`을 쓰세요.

**앱이 창도 없이 즉시 종료되고 로그가 0바이트라면** 디버그 빌드와 릴리즈 빌드의 구분이 깨진 경우입니다. 두 빌드는 서로 다른 식별자를 달고 뜨는데, 이 표식이 어긋나면 릴리즈 앱이 떠 있는 상태에서 디버그 앱이 조용히 죽습니다. `cargo build`로 셸을 직접 굽지 말고 위 런처를 쓰세요 — 런처가 표식을 심고 실제로 박혔는지까지 확인합니다. 급하면 릴리즈 앱을 트레이에서 완전히 종료한 뒤 다시 띄워도 됩니다.

## 현재 한계

- **화면 배치가 저장되지 않습니다.** 다시 켜면 창·탭·분할이 초기화됩니다(명부와 프리셋은 남습니다)
- **데몬을 재시작하면 에이전트가 자동으로 돌아오지 않습니다.** 직접 깨우면 대화는 이어받습니다
- **주고받던 우편이 데몬 재시작에 살아남지 않습니다.** 메모리에만 있습니다
- **메시징 화면이 없습니다.** 우편은 에이전트끼리만 오갑니다
- **단축키는 두 개뿐이고 바꿀 수 없습니다** — `Ctrl+Shift+T`(테마), `Ctrl+Tab`(다음 탭)

## 문서

- [아키텍처 개요](docs/reference/architecture-overview.md) — 전체 구조·crate 구성
- [문서 인덱스](docs/README.md)
- [설계 결정 기록](docs/decisions/)
- [개발 진행 기록](docs/process/step-log.md)

## 라이선스

Copyright © 2026 kimsunzun. All rights reserved.

오픈소스 라이선스는 정식 공개 시점에 부여할 예정입니다. 그 전까지 저장소 내용에는 기본 저작권 규칙이 적용되며, 외부 기여(PR)는 받지 않습니다.
