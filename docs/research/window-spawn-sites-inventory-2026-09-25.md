# 창을 띄울 수 있는 프로세스 기동 지점 전수 (2026-09-25)

- **상태:** 조사 완료 · **공용화는 보류**(사용자 결정 2026-09-25 — 「일단 스크립트에서 각각 하는걸로 만족하자. 나중에 어찌어찌」)
- **방법:** 경량 조사 1회(코드 독해) + 메인 스팟 대조 3건(`wmi_create_raw` 의 시작 정보 생략 · agent 두 통로의 `CREATE_NO_WINDOW` · 데몬·셸의 `windows_subsystem`) — 적대 리뷰 없음
- **계기:** 분리 실행 스크립트의 콘솔 창이 포커스를 뺏는 문제(브랜치 `v0.3.2/fix/hide-detached-console`)를 고치다가 「앱이 직접 띄우는 것도 전부 최소화 기본으로 — 한 곳에서 공용화는 안 되나」라는 요청이 나왔다.

## 결론

- **운영 코드에서 창을 띄울 수 있는 Rust 지점은 둘이고, 포커스를 뺏을 수 있는 것은 하나다.** 둘 다 `crates/engram-dashboard-discovery/src/lib.rs` 에 있다.
  1. **데몬 WMI 기동**(`wmi_spawn` → `wmi_create_raw`, ~l.1110-1275) — debug 데몬은 콘솔 서브시스템이라(`daemon/src/main.rs:9` 가 release 에서만 `windows_subsystem`) 보통 콘솔 창이 뜬다. `console=false` 면 시작 정보를 **아예 안 넘겨** 창 표시 값이 없다(l.1127·1152). 주석상 의도(「로그용」 — 사용자 결정 2026-06-19). 포커스 탈취는 스크립트 쪽 실측(`scripts/run-detached.ps1` 주석)에서 유추 — 데몬 경로 자체는 미실측. — 가능성 높음
  2. **release 앱의 `taskkill`**(`TaskKiller` ~l.911, `daemon_stop`) — release 셸은 콘솔이 없어(`src-tauri/src/main.rs:2`) 콘솔 도구가 새 콘솔을 잠깐 연다. 미관측 추론. — 불확실
- **나머지는 이미 창이 없다** — PTY 통로(ConPTY) · stdio 통로와 codex app-server 통로(`CREATE_NO_WINDOW` — `agent/src/transport/stdio.rs:95-96` · `agent/src/backend/codex/transport.rs:611`, 같은 상수를 각자 인라인으로 둔다). — 가능성 높음
- **스크립트:** 분리 실행 둘(`run-detached.ps1`·`launch-detached.ps1`)의 래퍼 콘솔은 이 브랜치에서 최소화·비활성 기동으로 바뀐다(앱 창은 보통으로 뜬다 — 아래 「공용화하려면」 절). vite 는 `start /MIN`(`run-debug.bat`·`rebuild-run-debug.bat`). 나머지 스크립트는 호출자 콘솔을 물려받아 창이 없다.
- **테스트:** 약 25곳이 실 프로세스를 띄우지만 cargo 콘솔을 물려받아 창이 안 뜬다. 예외 = `#[ignore]` 수동 WMI 시험(`real_wmi_spawn`·`real_wmi_spawn_flag_matrix`).

## 공용화하려면 (보류된 안 — 다시 열 때 여기서 시작)

- **완전한 한 곳은 없다** — PowerShell 과 Rust 두 언어라 코드를 공유할 수 없다. 언어별 한 곳씩은 된다. OS 전역 설정(「새 콘솔은 늘 최소화」)은 없는 것으로 안다. — 가능성 높음
- **Rust 데몬 기동:** `wmi_create_raw` 가 이미 유일한 병목이다 — 늘 `Win32_ProcessStartup` 을 만들어 `ShowWindow = 7`(SW_SHOWMINNOACTIVE)을 싣고 `CreateFlags` 는 `console=true` 일 때만. 호출부 변경 0. WMI 소비자가 하나라 `base` 입주 조건 ①에 안 맞으므로 discovery 에 둔다.
- **Rust std `Command` 지점(stdio · codex app-server · taskkill):** `base::platform` 에 `background_command()` 류 헬퍼(Windows = `CREATE_NO_WINDOW`, 그 밖 = 무동작 — ADR-0230). 소비자 둘(agent·discovery)이라 입주 조건을 만족하고 `platform` 안이라 새 입주자가 아니다. ★**stable std 는 `STARTUPINFO.wShowWindow` 를 못 준다**★ — 「최소화·비활성」은 `CreateProcessW` 직접 호출이 필요하고, 헤드리스 프로세스엔 숨김(`CREATE_NO_WINDOW`)이 맞는 의미다(추론).
- **스크립트:** 두 분리 실행 스크립트의 WMI 기동 블록을 `scripts/lib/` 의 dot-source 헬퍼 하나로. Engram 앱 창의 첫 표시 상태는 런처가 정할 수 없다(실측 — `scripts/launch-detached.ps1` 주석).

## 모르는 것

- WMI 로 뜬 debug 데몬 콘솔이 실제로 전경을 가져가는지(데몬 경로 실측 없음) · release 앱 `taskkill` 깜빡임의 실제 관측 · Windows Terminal 이 기본 터미널일 때 `start /MIN` 이 위임·활성화될 수 있는지 · ConPTY 자식이 창을 띄우는 경우가 있는지.
