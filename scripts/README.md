# scripts

앱과 빌드·테스트를 띄우는 스크립트 모음입니다(Windows).

앱은 셸에서 직접 띄우지 않습니다 — 셸에서 띄우면 그 호출이 앱 수명에 매달리고 앱 출력이 셸로 계속 거슬러 올라옵니다. 런처는 WMI(`Win32_Process.Create`)로 앱을 프로세스 트리 밖에 띄우고 출력을 파일로만 보내므로, 로그에서 필요한 줄만 읽으면 됩니다.

## 런처

| 런처 | 하는 일 |
|---|---|
| `scripts\run-debug.bat` | 클라이언트만 빌드 + dev 서버 확인 + 실행 |
| `scripts\rebuild-run-debug.bat` | 데몬까지 재빌드(백엔드 수정 후) + 실행 |
| `scripts\rebuild-run-debug-log.bat` | 위와 같되 앱·데몬을 `debug` 로그로 실행 |
| `scripts\run-release.bat` | 이미 빌드된 릴리즈 실행 |
| `scripts\rebuild-run-release.bat` | 릴리즈 새로 빌드 + 실행 |

## 빌드·테스트

빌드·테스트도 같은 방식으로 [`run-detached.ps1`](run-detached.ps1)을 거쳐 프로세스 트리 밖에서 돌리고, 출력은 파일로만 받습니다 — 빌드 로그 전체를 읽지 않고 필요한 줄만 보기 위해서입니다. 사용법은 그 스크립트 헤더에 있고, 어떤 명령을 어느 강도로 돌리는지는 `/qa` 바인딩([`.claude/skill-bindings/qa.md`](../.claude/skill-bindings/qa.md))이 정합니다.

## 그 밖의 파일

나머지 `*.mjs`·`*.ps1`은 런처·빌드가 부르는 보조 스크립트와 개발용 도구입니다. 용도는 각 파일 헤더에 있습니다.
