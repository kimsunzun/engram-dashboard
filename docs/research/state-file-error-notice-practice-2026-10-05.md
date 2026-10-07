# 상태 · 설정 파일을 못 읽었거나 깨졌을 때 사용자에게 알리나 — 관행 (light · 2026-10-05)

- **상태:** 결정됨(사용자 2026-10-05) — ① 부팅에서 `state.json` 을 못 읽으면 **안내만 하고 그 실행은 끝까지 저장하지 않는다**(재시도를 늘리거나 실행 중 다시 읽지 않는다 — 「가장 깔끔한 구현」) · ② 깨진 파일을 떠 두고 기본 화면이면 같은 방식으로 안내(관행 · 세션 추천) · 자동 복구 단추 없음 · LLM 도 상태 조회로 안다 · 구현 = P3c2(TRD §9-2).
- **방법:** `/research` light — 수집자 1(문서형) · 약 12회 검색·열람 · 메인 grounding = Windows Terminal · VS Code 문구를 원본 소스에서 직접 대조(2026-10-05). 나머지는 2차 출처.
- **결정 질문:** 저장 관리(TRD `docs/process/S21-storage/trd.md` §6-5)의 두 경우에 화면 안내를 할까 — ① `state.json` 을 못 읽어 이번 실행은 화면 상태를 저장하지 않는다(가드 ⅰ) ② 깨진 파일을 `.corrupt` 로 떠 두고 기본 화면으로 시작한다.
- **확신도 범례:** 확실(원본 소스 직접 대조) · 가능성 높음(2차 출처 여럿) · 불확실(사용자 보고 하나 · 문구 미대조).

## 발견

| 앱 | 경우 | 사용자가 보는 것 | 문구 | 확신도 · 출처 |
|---|---|---|---|---|
| Windows Terminal | settings.json 파싱 실패 → 기본값으로 계속 | 경고(형태 미확인 — 대화상자인지 InfoBar 인지) | 제목 "Failed to load settings" · 본문 "Settings could not be loaded from file. Check for syntax errors, including trailing commas." + "Temporarily using the Windows Terminal default settings." · 다시 읽기 실패 = "Failed to reload settings" | **확실**(문구) — [Resources.resw](https://raw.githubusercontent.com/microsoft/terminal/main/src/cascadia/TerminalApp/Resources/en-US/Resources.resw) `InitialJsonParseErrorTitle/Text` · `UsingDefaultSettingsText` · `ReloadJsonParseErrorTitle` |
| Windows Terminal | 쓰기 불가 | 경고 | "We could not write to your settings file. Check the permissions …" | 가능성 높음(수집자 요약 · 메인 미대조) |
| VS Code | settings.json 이 깨진 채 설정을 쓰려 할 때 | 알림 + 「Open Settings」 단추 · 쓰기 거부(덮지도 떠 두지도 않음) | "Unable to write into user settings. Please open the user settings to correct errors/warnings in it and try again." | **확실** — [configurationEditing.ts](https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/services/configuration/common/configurationEditing.ts) `errorInvalidConfiguration` · `open` |
| Chrome | 설정이 다른 프로그램에 바뀐 것을 감지(HMAC) → 기본값으로 되돌림 | 알림 + 되돌리기 선택 | "Some settings were reset …"(2차 인용 · 원문 미대조) | 불확실 — [chalmers 논문](https://odr.chalmers.se/items/412c8886-a931-4f3f-8219-6561e2f82c65) · [jamf 포럼](https://community.jamf.com:443/t5/jamf-pro/google-chrome-settings-from-user-template-quot-some-settings-were/td-p/180177) |
| Chrome | Preferences 를 못 읽거나 깨짐 | 모달 오류 대화상자 · 복구는 사용자가 손으로 | "Your preferences file is corrupt or invalid. Google Chrome is unable to recover your settings." | 불확실 — [사용자 보고](https://www.wintips.org/?p=16845) |
| JetBrains | 설정 파일을 못 읽음(파싱 오류 · 크기 한도) | 알림 | "Cannot load settings from file '<path>': …" | 불확실 — [지원 포럼](https://intellij-support.jetbrains.com/hc/en-us/community/posts/115000008604/comments/115000035464) |
| Firefox | prefs.js · sessionstore 손상 | 자동 안내 근거 없음 — 사용자가 손으로 백업에서 복구 | — | 불확실 — [SUMO](https://support.mozilla.org/questions/1245632) · [discourse](https://discourse.mozilla.org/t/session-file-corruption/110112) |

## 해석 (사실 아님)

- **조용히 넘어가는 곳은 근거로 확인된 범위에 없다.** 확인된 둘(Windows Terminal · VS Code)은 모두 화면에 알리고, 원인(파일 · 구문)과 다음 행동(파일 열기 · 기본값 「임시」)을 문구에 싣는다.
- **「이번엔 기본값으로 임시 동작」을 명시하는 선례 = Windows Terminal** — 우리 ①(이번 실행은 저장 안 함)과 가장 가깝다.
- **자동 복구 단추는 드물다**(Chrome 변조 되돌림만). 나머지는 안내 + 사용자가 손으로.
- **읽기 실패(잠김 · 권한)와 손상을 갈라 다르게 알리는 선례는 못 찾았다.**

## 공백

- 읽기 실패(잠김 · 권한)만의 문구 선례 — 대부분 손상 · 쓰기 실패 사례였다.
- Windows Terminal 경고의 UI 형태 · Chrome/Firefox/JetBrains 원문 · VS Code `state.vscdb` · 터미널 세션 복원 도구(tmux-resurrect · zellij · wezterm · iTerm2) · Sublime · Obsidian — 근거 없음.
