# 실행 때 붙은 구성 요소를 못 읽으면 — 막기 vs 알리고 띄우기 (관행 조사)

- **상태:** light 조사 · 2026-10-18 · 수집자 1(문서형) + 메인 grounding(핵심 인용 둘 원문 대조) · 적대 리뷰 없음
- **질문:** 에이전트를 띄울 때 붙은 프리셋의 라이브러리 폴더가 없거나 codex 작업 폴더에 지침·스킬 파일을 놓을 수 없으면 실행을 막는가, 빠진 채로 띄우고 알리는가.
- **결정:** 사용자 2026-10-18 — 막는다(누가 실행하든 · 사유를 보인다). 이 조사가 그 결정을 받친다.

## 결론

- **사용자가 명시해서 붙인 구성 요소는 기본이 「필수」이고, 없으면 막고 오류를 낸다.** Docker Compose · systemd · Claude Code 자신의 `--agents` · `--add-dir` 이 그렇다. — 가능성 높음
- **빠진 채로 띄우기는 구성 요소마다 작성자가 명시로 켜는 「선택」 표시**다(Compose `required: false` · systemd `EnvironmentFile=-경로` · Unreal 플러그인 `Optional`). 전역 「관대 모드」로 기본 동작하는 피어는 찾지 못했다. — 가능성 높음
- **선택 표시로 빠진 채 띄울 때 피어들은 대개 알리지도 않는다**(Compose「silently ignores」 · Unreal「silently ignore」 · systemd `-` 형식). 「알리고 띄우기」는 피어 선례가 약하다. — 가능성 높음
- **사람/헤드리스로 갈리는 피어는 찾지 못했다.** — 불확실(못 찾음 ≠ 없음)

## 피어별

| 피어 | 동작 | 장치 | 확신도 | 출처 |
|---|---|---|---|---|
| Docker Compose `env_file` | 기본 막힘 · 선택 가능 | `required` 기본 `true` · `false` 면 「silently ignores the entry」 | 확실(원문 대조) | https://docs.docker.com/reference/compose-file/services/ |
| systemd | 기본 막힘 · 항목별 선택 | `EnvironmentFile=-path`「not considered an error if the file does not exist」 · `Requires=` 대 `Wants=` | 확실(수집자 인용) | https://man7.org/linux/man-pages/man5/systemd.exec.5.html · https://man7.org/linux/man-pages/man5/systemd.unit.5.html |
| Unreal 플러그인 | 선택이면 조용히 빠짐 · 필수일 때 동작은 미확정 | `FPluginReferenceDescriptor.bOptional`「silently ignore it not being present」 | 플래그 확실 · 필수 동작 불확실(포럼만) | https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Projects/FPluginReferenceDescriptor |
| Claude Code CLI | 명시 인자는 시작 때 검증 후 종료 | `--agents`「validates the value at startup and exits on an invalid one」 · `--add-dir`「Validates that each path exists as a directory」 · 전역 우회 = `--safe-mode` · `--bare`(명시 선택) | 확실(원문 대조) | https://code.claude.com/docs/en/cli-reference |
| Claude Code `--settings` · `--plugin-dir` · `--mcp-config` 없는 파일 | 모름 | — | 기권 | 문서에 없음 |
| VS Code `launch.json` `envFile` 없음 | 막힘(ENOENT) | 언어 확장 동작 | 가능성 높음(이슈 스니펫만) | dotnet/vscode-csharp · microsoft/vscode-python 이슈 |
| JetBrains 실행 구성 오류 | 빨간 표시까지만 확인 · 막는지 모름 | — | 기권 | https://www.jetbrains.com/help/idea/run-debug-configuration.html |
| codex CLI · tmuxinator/tmuxp | 모름 | — | 기권 | — |

## 이 결정에 대한 함의(추론)

- Engram 의 프리셋은 사용자가 명시로 붙인 것이고 「선택」 표시가 없다 → 관행대로면 **필수 → 막고 사유를 보인다**.
- 나중에 「빠져도 띄우기」가 필요해지면 전역 스위치가 아니라 **붙임마다 「선택」 표시**가 피어 꼴이다.
- 실측 참고: claude 는 `--settings` 에 타입 틀린 값이 하나라도 있으면 그 전체를 말없이 버린다(`settings-asset-validation-spike-2026-10-17.md`) — CLI 쪽 「조용한 빠짐」이 실제로 있어서 Engram 이 앞에서 막는 값어치가 있다.

## 공백

- Unreal 필수 플러그인 누락의 기본 동작 · JetBrains 막힘 여부 · claude `--settings`/`--plugin-dir`/`--mcp-config` 없는 경로 · codex — 확인 못 함.
