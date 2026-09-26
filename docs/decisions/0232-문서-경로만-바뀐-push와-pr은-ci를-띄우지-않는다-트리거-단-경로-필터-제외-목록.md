# ADR-0232: 문서 경로만 바뀐 push와 PR은 CI를 띄우지 않는다 — 트리거 단 경로 필터 제외 목록

- 상태: 확정 (2026-09-26, 근거: 사용자 결정 + 적대 리뷰 1라운드)
- 관련: `docs/research/ci-docs-skip-conventions-2026-09-26.md`(성숙 OSS 서베이 — 결정 뒤에 했다) · `.github/workflows/ci.yml` 의 `on.push.paths`(목록 정본) · 같은 파일 `on.pull_request.paths`(같은 목록) · `src/util/launcherWiring.test.ts`(되살린 파일을 읽는 테스트) · `docs/tracking.md` T-18(branch protection — 도입 안 함) · CLAUDE.md 「CI — push하면 자동으로 돈다」·「브랜치·커밋」(머지 규칙 예외) · `.claude/skill-bindings/qa.md` 「CI와의 분담」 · Amends ADR-0131 (결정 2 매 push 범위에서 문서 경로만 바뀐 push와 PR 제외)

## 맥락

워크플로가 **어느 브랜치든 매 push 마다** Windows 러너에서 검증 3잡 전체를 띄운다 — CLAUDE.md 나 `docs/` 만 고친 push 도 예외가 없다. 실측 = 문서 11파일 커밋이 6분 44초 걸려 초록(2026-08-23 — 이 결정 전까지 `.claude/skill-bindings/adr.md` 의 채번 선점 항목에 적혀 있던 기록).

- **사용자 제기(2026-08-26 — `docs/todo.md` 의 「CI 실행 범위」 항목으로 올라 있던 것):** 문서만 바꿔도 검증 3잡이 통째로 돈다. 러너 비용은 0이지만(public repo) 기다림이 거슬린다.
  - 그 항목이 함께 적어 둔 관측 — 그날 문서 작업으로 CI 가 다섯 번 돌았고, 그중 넷은 **커밋을 하나씩 따로 밀어서** 생긴 것이었다(밀 때마다 `concurrency` 가 앞 실행을 취소하고 새로 띄운다). 모아서 한 번에 밀면 한 번이다. 이 결정 뒤에도 문서 밖 파일이 섞인 push 에는 그대로 해당한다. 그 항목(`docs/todo/ci-scope.md`)은 이 결정으로 풀려 지웠다.
- **사용자 요구(2026-09-26):** CI 에 영향을 주는 파일이 바뀔 때만 CI 를 돌린다.

## 결정

1. **`on.push` 와 `on.pull_request` 에 트리거 단 `paths` 를 제외 목록 모양으로 둔다.** 첫 항목 `'**'` 가 모든 경로를 넣고, 네 경로를 뺀 뒤(`docs/**` · `CLAUDE.md` · `README.md` · `.claude/**`), **맨 끝에서** `.claude/skill-bindings/qa.md` 하나를 되살린다. 바뀐 파일이 **전부** 빠진 경로 안이면 워크플로가 뜨지 않고, 하나라도 밖이면(되살린 파일 포함) 전과 똑같이 전부 돈다. 초안은 `paths-ignore` 모양이었고 적대 리뷰 뒤 이 모양으로 바꿨다(아래 거부한 대안 첫 항목).
   - **되살린 이유** — 그 파일은 CI 테스트 입력이다. frontend 잡의 `npm test` 가 도는 `src/util/launcherWiring.test.ts` 가 그 파일의 펜스 안 명령을 읽어 단언한다(`describe('the /qa GUI gate procedure …')`). 빼 두면 그 파일만 바꾼 push 에서 그 파일을 검사하는 테스트가 돌지 않는다.
   - **`paths-ignore` 가 아니라 `paths` 인 이유** — 문서가 「제외만 할 때 `paths-ignore`, 포함과 제외를 섞을 때 `!` 를 붙인 `paths`」를 권하고, 한 이벤트에 둘을 함께 둘 수 없다(근거 ⑤). `paths-ignore` 안에서 `!` 로 되살리는 곳도 있지만(Node) 문서가 그 동작을 보장하지 않는다 — 판정 불가로 남겼다(`docs/research/ci-docs-skip-conventions-2026-09-26.md`). `'**'` 는 경로 필터가 없을 때의 기본 동작과 같아서(근거 ⑤) 이 모양은 여전히 제외 목록이다 — 모르는 경로는 전부 「돈다」로 떨어진다.
2. **목록의 정본은 `ci.yml` 의 `on.push.paths` 다.** 다른 문서는 그 자리를 가리키기만 하고 항목을 베끼지 않는다. `on.pull_request.paths` 는 순서까지 같게 둔다.
3. **`v*` 태그 push 와 `workflow_dispatch` 는 영향이 없다** — 태그 push 에는 경로 필터가 평가되지 않고(아래 근거 ①), `workflow_dispatch` 에는 경로 필터가 없다. 릴리즈 트리거와 그 앞의 검증 3잡은 그대로 돈다.

## 거부한 대안

- **`paths-ignore` 단독**(초안의 모양 — 네 경로를 그대로 나열) — 빠진 경로 안의 한 파일을 되살리는 것을 문서가 보장하는 모양으로 쓸 수 없다(근거 ⑤). 적대 리뷰가 `qa.md` 를 CI 테스트 입력으로 잡은 뒤(아래 근거) 바꿨다. 기각 근거 강도 = 문서(GitHub 이 권하는 모양 — 금지 규정은 아니다).
- **포함 목록(allowlist — `paths:` 에 CI 에 닿는 경로만 양성으로 나열. 결정 1 의 `'**'` + `!` 모양과 다르다)** — 사용자 기각(2026-09-26). 아래 근거는 세션이 그날 제시했고, 사용자가 제외 목록을 고르며 받아들였다(「그래 그럼 제외 목록으로 가자. 저렇게 4개면 된다는거잖아. 진행해」). 사용자가 따로 댄 근거는 없다.
  - **틀렸을 때의 방향이 반대다.** 포함 목록에서 한 항목을 빠뜨리면 그 경로의 변경에서 게이트가 **조용히 사라진다.** 제외 목록에서 빠뜨리면 **한 번 더 돌 뿐**이다.
  - **CI 에 닿는 경로가 흩어져 있고 이름만으로는 안 보인다.** 루트의 추적 항목 23개 중 약 15개가 CI 에 닿는다(2026-09-26 조사). 그중엔 `prompts/*.md`(아래 둘째 대안) · `rust-toolchain.toml`(backend 잡이 채널을 읽어 단언한다) · `vitest.config.ts` · `.gitattributes`(줄끝 처리가 fmt·바인딩 sync 게이트에 닿는다 — ★이 항목은 판단이고 실측하지 않았다★) 같은 비자명한 것이 섞여 있다.
  - **루트에 새 파일이 생길 때마다 목록을 고쳐야 한다.**
  - 기각 근거 강도 = 실측(루트 항목 조사) + 코드(아래 `include_str!`).
- **`**/*.md` 일괄 제외** — 아래 근거는 세션이 2026-09-26 에 제시했고, 사용자가 네 경로 목록을 고르며 받아들였다(위와 같은 발화). 확장자로는 문서와 배송물이 갈리지 않는다. `prompts/engram-help.md` 는 `include_str!` 로 바이너리에 구워지고(`crates/engram-dashboard-daemon/src/bin/engram.rs:231`), `prompts/agent-priming.md` 와 함께 배포판 manifest 에 든다(`scripts/build-release.ps1:50`). 이걸 빼면 도움말 본문·프라이밍을 바꾼 push 에 게이트가 없다. 기각 근거 강도 = 코드.
- **잡 단위 건너뛰기**(워크플로는 늘 띄우고 첫 스텝에서 바뀐 경로를 검사해 잡을 건너뛴다) — **사용자에게 제시했고 고르지 않았다. 사용자가 따로 준 기각 사유는 없다.** 사실로 적어 둘 수 있는 대가만 남긴다.
  - 얻는 것: 실행 기록이 「skipped」로 남는다 · 조건으로 건너뛴 잡은 「Success」로 보고되므로(근거 ④) 필수 체크를 거는 branch protection 과 양립한다 · 아래 「영향」의 머지 규칙 예외가 필요 없다.
  - 치르는 것: push 마다 러너 기동 1회 + 변경 감지 로직(직접 짜거나 서드파티 action).
  - ★기각 근거 강도 = 약함★(정량·실측·코드 어느 것에도 걸리지 않는다). 아래 재론 트리거 ①이 오면 이 대안이 첫 후보다.

## 근거

- **사용자 결정(2026-09-26)** — 제외 목록 방식 · 네 제외 경로 · 포함 목록 기각.
- **적대 리뷰(2026-09-26 — Claude doc-aware + Codex blind, 둘 다 FIX)** — 결정 1 의 `paths` 모양과 `qa.md` 되살리기는 이 리뷰가 잡은 결함을 고친 것이다.
- ★**빠진 경로를 읽는 것 — 초안은 하나를 놓쳤다**★. 초안은 `.github/workflows/ci.yml` · `scripts/build-release.ps1` · `scripts/*.mjs` · `src-tauri/tauri.conf.json` · `src-tauri/build.rs` · Rust/TS 소스를 grep 해 「네 경로를 읽는 것이 없다」고 적었는데 틀렸다 — 적대 리뷰가 `src/util/launcherWiring.test.ts:76` 이 `.claude/skill-bindings/qa.md` 를 읽는 것을 잡았다. 리뷰 뒤 코드·스크립트·워크플로(`*.ts`·`*.tsx`·`*.rs`·`*.mjs`·`*.js`·`*.ps1`·`*.bat`·`*.yml`)에서 빠진 경로를 가리키는 문자열 리터럴을 다시 훑었고, 파일을 읽는 것은 그 하나였다(`crates/engram-dashboard-daemon/tests/engram_cli.rs` 의 `"README.md"` 는 CLI 인자 문자열이다). ★런타임에 조립한 경로는 이 방식으로 못 본다★.
- **저장소 상태(`gh api`, 2026-09-26)** — public · master 에 branch protection 없음 · ruleset 0개.
- **GitHub 동작 — 공식 문서로 확인한 것**(2026-09-26 열람):
  - ① **태그 push 에는 경로 필터가 평가되지 않는다** — "Path filters are not evaluated for pushes of tags." (https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#onpushpull_requestpull_request_targetpathspaths-ignore) 같은 절: `branches` 와 경로 필터를 함께 두면 **둘 다** 만족할 때만 돈다.
  - ② **바뀐 파일 목록을 만드는 법** — push 는 two-dot diff, PR 은 three-dot diff. 기존 브랜치 push = head 와 base SHA 를 직접 비교 · 새 브랜치 push = "a two-dot diff against the parent of the ancestor of the deepest commit pushed" · PR = 토픽 브랜치 최신본과, 그 브랜치가 base 와 마지막으로 맞춰진 커밋의 비교. 바뀐 파일이 없으면 돌지 않는다. (https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#git-diff-comparisons)
  - ③ **한도** — push 가 커밋 1,000 개를 넘으면 **항상 돈다** · diff 생성이 시간 초과되면 **항상 돈다** · diff 가 3,000 파일을 넘고 필터가 맞추는 파일이 필터가 돌려준 앞 3,000 개 안에 없으면 **돌지 않는다**(같은 URL). 셋째는 `paths` 모양이라 문서 문장이 그대로 적용된다 — 받아들인 예외다(아래 「영향」).
  - ④ **경로 필터로 건너뛴 워크플로의 체크는 「Pending」으로 남고, 그 체크를 요구하는 PR 은 머지가 막힌다.** 문서의 처방 = "Avoid requiring workflows that can be skipped." 대조로 **조건(`if`)으로 건너뛴 잡은 「Success」로 보고된다.** (https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks#handling-skipped-but-required-checks · 같은 문장이 위 workflow-syntax 의 `paths` 절에도 있다)
  - ⑤ **`paths` 의 제외와 순서** — `paths` 와 `paths-ignore` 는 한 이벤트에 함께 쓸 수 없다 · 제외만 할 때는 `paths-ignore`, 포함과 제외를 섞을 때는 `!` 를 붙인 `paths` 를 쓰라고 권한다 · 필터 패턴 규칙 전반: "`!`: At the start of a pattern makes it negate previous positive patterns." · 순서가 뜻이다 — 양성으로 맞은 뒤의 `!` 패턴은 빼고, `!` 로 빠진 뒤의 양성 패턴은 되살린다 · `'**'` 는 "the default behavior when you don't use a path filter" 다. (①과 같은 페이지 — `paths` 절과 필터 패턴 절)
- **문서에서 끌어낸 해석(확신도 = 가능성 높음)** — 기존 브랜치 push 는 그 push 가 옮긴 범위만 본다(②의 head 와 base 를 push 전후 SHA 로 읽은 것 — 문서가 base 를 push 전 SHA 라고 못 박지는 않는다). 그래서 코드 push 뒤에 문서만 push 하면 새 실행이 생기지 않고, 코드에 대한 판정은 앞 실행이 쥔다. 새 실행이 없으니 `concurrency` 의 앞 실행 취소도 일어나지 않는다. 실측하지 않았다.
- ★**성숙 프로젝트 서베이 — 결정 뒤에 했다**★ — 지운 할 일(`docs/todo/ci-scope.md`)은 `/research` 로 성숙 프로젝트가 경로 필터와 잡 단위 건너뛰기 중 어느 쪽을 왜 고르는지 조사하는 것을 절차로 적어 두었다. 결정 때는 하지 않았고(사용자가 생략을 명시한 적은 없다), 사용자가 「보통 그렇게 하나」를 물어 같은 날 했다 = `docs/research/ci-docs-skip-conventions-2026-09-26.md`. 결론: 필수 체크가 없는 저장소의 관례 안이다 · 필수 체크를 거는 표본은 잡 단위 감지 + 집계 잡이 많았다(재론 ①의 후보와 같다) · 문서가 테스트 입력이면 그 경로를 빼지 않거나 좁게 빼는 것이 관례다(Node `doc/` · Ruff) — 우리 `qa.md` 되살리기와 목적이 같다. 설정은 그대로 두고 이 ADR 의 과장 문구만 줄였다. 그 할 일이 착수 전에 풀라고 적은 두 미지(태그 push 에 경로 필터가 걸리나 · 필수 체크가 「Pending」에 묶이는 함정)는 서베이 대신 GitHub 공식 문서로 답했다(①·④).

## 영향 / 불변식

- **머지 규칙 예외(CLAUDE.md 「브랜치·커밋」)** — 빠진 경로만 바뀐 변경에는 CI 실행이 없으므로 초록 없이 머지할 수 있다. 단 그것을 기계로 확인했을 때만이고, 확인 명령과 「빨강·취소 뒤 문서만 push」의 처리(`workflow_dispatch` 로 손으로 띄우기)의 정본은 그 절이다.
- **빠진 경로만 바뀐 변경은 어떤 게이트도 보지 않는다 — 이 결정은 빠진 경로를 어떤 게이트도 읽지 않는다는 전제 위에 선다.** 게이트가 읽는 파일이 빠진 경로 안에 있으면 목록 끝에서 되살린다(지금 = `qa.md` 하나). 제외를 더할 때도 먼저 그 아래를 읽는 것이 있는지 본다. `.claude/**` 는 문서가 아닌 설정(`.claude/settings*.json` 등)도 덮는다 — 그것을 읽는 게이트는 위 재확인에서 나오지 않았다.
- **되살리는 줄은 목록 맨 끝에 둔다** — 그 뒤에 `!` 줄을 더하면 도로 빠진다(근거 ⑤).
- **push 쪽과 PR 쪽 목록은 순서까지 같게 둔다** — 한쪽만 고치면 push 와 PR 이 서로 다른 변경에서 돈다.
- **받아들인 예외 — 3,000 파일 한도(근거 ③)** — diff 가 3,000 파일을 넘고 필터에 맞는 파일이 앞 3,000 개 안에 없으면, 코드가 섞인 push 라도 돌지 않는다. 그런 push 는 `gh workflow run ci.yml --ref <브랜치>` 로 손으로 띄운다.
- **이름 바꾸기 — 가능성 높음: 옛 경로와 새 경로를 모두 평가한다** — 문서는 말하지 않고, 비공식 출처 둘(실행 기록이 붙은 제3자 이슈 — PR 이벤트 사례 · 커뮤니티 답변)이 양쪽 평가를 보고한다(`docs/research/ci-docs-skip-conventions-2026-09-26.md` 「결론 — 우리 문서에서 고칠 것」). 그렇다면 아래 걱정은 성립하지 않는다. 만약 새 이름으로만 평가한다면 그런 이동만 담은 push 에는 CI 가 뜨지 않고, 옮겨 간 파일에 기대던 빌드·테스트가 깨진 것은 다음 코드 push 에서야 드러난다.
- **재론 트리거**
  - ① **이 워크플로의 체크를 필수로 거는 branch protection·ruleset 을 도입할 때** — 현황 = `docs/tracking.md` T-18(사용자 결정: 도입 안 함 · 출처 = ADR-0131 미결 ②). 도입하면 문서만 바꾼 브랜치의 PR 이 「Pending」에 묶여 머지가 막힌다(근거 ④). 그때 이 결정을 다시 연다. 후보 = 잡 단위 건너뛰기.
  - ② **문서 경로를 검사하는 CI 잡이 생길 때**(예: `docs/handbook/documentation-system.md:40` 이 「나중」으로 적어 둔 문서 lint/CI) — 그 잡은 문서만 바꾼 push 에서 영영 돌지 않는다. 그때 이 결정을 다시 연다.
  - ③ **빠진 경로 아래 파일이 게이트 입력이 될 때** — 목록 끝에서 그 파일을 되살린다(위 불변식).
  - ④ **문서 전용 디렉터리가 새로 생길 때** — 목록에 더한다. 더하기 전에 그 경로 아래를 아무것도 읽지 않는지 먼저 확인한다(위 불변식).
