# 작업자 규칙 — engram-dashboard

명령을 돌리는 작업자가 이 저장소에서 지킨다.

- 저장소 루트에서 bare `cargo test`를 돌리지 않는다 — 테스트 실행 명령(플래그·분리 실행)은 `.claude/skill-bindings/qa.md`를 따른다.
- 셸 패키지(`-p engram-dashboard`, `--workspace` 포함)에 `--lib`·`--all-targets`를 쓰지 않는다 — 내장 lib 테스트 타깃이 manifest 없이 지어져 `0xc0000139`로 죽는다. 셸 단위 테스트는 `cargo test -p engram-dashboard --test lib_unit`로 돌린다.
- `cargo build`로 지은 `engram-dashboard.exe`를 띄우지 않는다 — release identifier가 찍혀 릴리즈 앱이 떠 있으면 창 없이 죽는다. 띄울 셸은 `node scripts/build-client-shell.mjs`로 짓는다.
- 앱을 셸에서 직접 띄우지 않는다 — `.claude/skill-bindings/qa.md` 「GUI 실측」 절차(빌드 · 점검 · `scripts/launch-detached.ps1`)로 띄운다.
- 에이전트 프로필 env에 자격증명을 넣지 않는다 — `agents.json`에 평문으로 저장된다.
- Windows에서 `python`은 Store 스텁(exit 49)이다 — `py`를 쓴다.
- 작업 산출물은 영어로 쓴다 — 코드 주석 · 문서 · 메인에게 돌려주는 보고. 지시서가 한국어여도 같다. 지시서가 출력 언어를 정하면 그것을 따른다. 한국어로 쓰는 것과 고치는 문단의 범위 = `CLAUDE.md` 「언어」.
