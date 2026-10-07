# 비정상 종료 뒤 세션 복원 스냅숏 — 보관·회전 선례 (2026-10-02)

> 강도: light(수집자 1 · 메인 스팟 그라운딩). 계기: S21-storage TRD §10 F17 「최근 3 개 회전 보관(Firefox 류)」을 출처로 확인. 연결: `docs/process/S21-storage/trd.md` §6-7 · §10 F17.
> **결론: 「크래시 스냅숏 N 개 회전」 선례는 없다.** 성숙한 앱은 살아 있는 파일 + 한 세대 전 사본(+ 마지막 정상 종료본)만 둔다. Firefox 의 「3 개」는 **업그레이드 백업**에만 걸린다.
> **결정(2026-10-02 사용자 결정 D1–D3):** F17 「최근 3 개 회전 보관」 철회 · 크래시 사본 = `shell\state\state.crash.json` 한 개 · 크래시 뒤엔 늘 묻는다(Chromium 식 — 거부 = Firefox 식 첫 크래시 조용히 복원). 반영 = TRD §6-5 · §6-7 · §14-9. **D8**(2026-10-02 — 쓰기 실패는 따로 다루지 않는다)은 아래 「덧붙임」에 기댄다 — TRD §6-4 · §14-10.

## 비교

| | Firefox | Chromium | VS Code(창 상태) | Windows Terminal |
|---|---|---|---|---|
| 파일 | 정상 종료본 `sessionstore` · 실행 중 `recovery` · 한 세대 전 `recovery.baklz4` · 직전 정상본 `previous` · 업그레이드 백업 ≤3 | `Sessions/Session_<ts>` 추가 전용 로그 — 현재 + 직전 유효본 1 | `storage.json` 의 `windowsState` 키 하나(회전 없음) · SQLite 는 `.backup` 1 | `state.json` 하나(회전 없음) |
| 비정상 종료 판별 | CrashMonitor 체크포인트(없으면 `state == "running"`) | `profile.exit_type` 을 시작 때 "Crashed", 종료 때 "Normal" | 없음 | 없음 |
| 깨졌을 때 | 순서대로 다음 후보: clean → recovery → recovery.bak → previous(+upgrade) | 최신부터 「초기 상태 표지」가 읽히는 첫 파일 | sqlite: `.backup` 으로 갈아끼움 → 빈 DB | 빈 상태로 시작 |
| 원자성 | 임시 + rename · 백업은 move · fsync 없음 | 추가 전용 + 표지로 유효성 | 임시(`.vsctmp`) + rename | 임시 + rename · fsync 미확인 |
| 묻나 | 첫 크래시는 조용히 복원, 연속 크래시(`max_resumed_crashes=1` 초과) · 6시간 넘은 세션이면 묻는 화면 | 크래시 뒤 **자동 복원 안 함**(크래시 루프 방지) — 「페이지 복원?」 | 설정대로 조용히 | 설정대로 조용히(1초 디바운스 · 레이아웃은 5분 주기 + 종료 시) |

## 우리 설계에 주는 것

- **N 개 회전은 근거가 없다** — 「Firefox 류 3 개」 인용은 틀렸다(3 은 업그레이드 백업). 크래시 스냅숏은 **한 세대**면 선례와 같다.
- **「뻗었나」와 「어느 파일이 멀쩡한가」를 가른다** — 판별은 별도 표지(Chromium 은 시작 때 Crashed 로 세우고 정상 종료 때 내린다), 파일은 순서 고정 후보 + 검증.
- **멀쩡하다고 확인된 것만 백업으로 올린다** — 무작정 회전하면 크래시 루프에서 마지막 멀쩡한 스냅숏이 밀려난다(Firefox · VS Code 공통).
- **크래시 루프 방지** — 연속 크래시면 자동 복원하지 말고 묻는다(Firefox 1 회 · Chromium 은 늘 묻는다).
- 원자 쓰기는 임시 + rename, 세 앱 모두 fsync 를 확인하지 못했다 — 전원 차단은 한 세대 사본이 덮는다.

## 출처 (메인 스팟 확인 = ✔)

- Firefox `browser/components/sessionstore/SessionFile.sys.mjs` — 파일 역할 L82–150 · 읽기 순서 L165 ✔ — https://github.com/mozilla-firefox/firefox/blob/main/browser/components/sessionstore/SessionFile.sys.mjs
- Firefox `SessionWriter.sys.mjs` — 백업은 recovery 가 멀쩡할 때만 L171–228 · 업그레이드 백업 정리 L245–300
- Firefox `browser/app/profile/firefox.js` — `max_resumed_crashes = 1` ✔ · `upgradeBackup.maxUpgradeBackups = 3` ✔
- Firefox `SessionStartup.sys.mjs` L276–320(크래시 판별) · `SessionStore.sys.mjs` `#needsRestorePage`
- Chromium `components/sessions/core/command_storage_backend.cc` · `chrome/browser/sessions/exit_type_service.cc` L194–205 · `startup_browser_creator_impl.cc`
- VS Code `src/vs/platform/windows/electron-main/windowsStateHandler.ts` · `src/vs/platform/state/node/stateService.ts` L141 · `src/vs/base/parts/storage/node/storage.ts` L183–315
- Windows Terminal `src/cascadia/TerminalSettingsModel/ApplicationState.cpp` L145–148 ✔ · `src/inc/til/io.h` · `src/cascadia/WindowsTerminal/WindowEmperor.cpp`

확신도: 표의 각 칸은 수집자가 소스로 읽은 것(확실)이고, Chromium 「실행 중 ≤3 파일」 · fsync 부재는 추론(가능성 높음)이다. 수집자 원문은 이 세션에만 있다.

## 덧붙임 — 상태 파일을 못 쓸 때 (같은 날 · light)

**결론: 네 앱 모두 로그만 남기고 계속 돈다.** 시작을 거부하거나 그 실행의 세션 복원을 끄는 곳이 없고, 사용자에게 보이는 알림도 없다. 재시도는 다음 저장 주기가 통째로 다시 쓰는 것으로 대신한다(Chromium 만 Windows 파일 잠금용 ReplaceFile 5회 재시도). 「실행 중」 표지 쓰기 실패를 따로 다루는 곳도 없다 — Firefox 는 마지막 쓰기가 실패해도 정상 종료로 기록한다. 사용자에게 창을 띄우는 것은 **읽기 쪽**(Chromium 의 깨진·못 읽는 프로필 대화상자)뿐이다.

- Windows Terminal `ApplicationState.cpp` L195 「Errors are only logged」 ✔
- VS Code `stateService.ts` `doSave` 의 `logService.error` L144 ✔
- Firefox `SessionFile.sys.mjs` 쓰기 오류 → 「Could not write session state file」 + 실패 수 집계(gecko-dev L450–492)
- Chromium `base/files/important_file_writer.cc`(재시도 5 · 100 ms · 경고 로그) · `chrome_pref_service_factory.cc`(프로필 오류 대화상자 = 읽기 오류 전용)
