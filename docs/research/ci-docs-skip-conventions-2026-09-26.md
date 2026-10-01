# 문서만 바뀐 변경에 CI 를 건너뛰는 관례 — 성숙 OSS 서베이 (2026-09-26)

- **상태:** 확정 — 적대 리뷰 2회(Codex 보고서 리뷰 FIX · doc-aware 문서 리뷰 FIX) 반영. 결정 = ADR-0232 유지(설정 그대로), 근거 문구만 이 보고서에 맞춰 줄였다. **사용자 결정 대기 하나** — 아래 「거부 후보」 둘(서드파티 감지 액션 · `[skip ci]`)을 ADR-0232 거부 대안에 더할지.
- **방법:** `/research` medium · 설계-결정 모드. 조사 수집자 3갈래 병렬(① 실제 저장소 워크플로 파일 ② GitHub 동작·함정 ③ 포함/제외 목록 선택 근거·문서가 테스트 입력인 경우) → 메인이 load-bearing 인용을 원본 파일로 대조(grounding) → cross-family(Codex) 적대 리뷰.
- **계기:** ADR-0232(문서 경로만 바뀐 push·PR 은 CI 를 띄우지 않는다)를 정한 뒤 사용자가 「보통 그렇게 하나」를 물었다. 그 ADR 을 정할 때 지운 할 일(`docs/todo/ci-scope.md`)이 계획했던 서베이이기도 하다.
- **확신도 범례:** **확실** = 원본 파일·공식 문서로 대조했고 독립 출처가 둘 이상 · **가능성 높음** = 대조했으나 출처 하나, 또는 문서의 필연적 독해 · **불확실** = 대조 못 함 / 추론.
- **대조 기준 시점:** 각 저장소 기본 브랜치, 2026-09-26 열람. 표의 SHA 는 수집자가 읽은 HEAD 앞 12자리다.

## 결론 (우리 선택과 대조)

1. **우리 방식은 관례 안에 있다 — 단, 「필수 체크가 없는 저장소」의 관례다.** 트리거 단 경로 필터(`paths`/`paths-ignore`)는 흔하다(Node · Playwright · React · Tauri · Polars · WezTerm · Biome). 반면 **필수 체크(branch protection·merge queue)를 거는 것으로 보이는 대형 프로젝트 표본에서는 「잡 단위 변경 감지 + 늘 도는 집계 잡」이 많았다**(CPython · Next.js · Electron · Zed · Ruff · uv · Vite · Servo — 모수를 세지 않았으므로 비율 주장은 아니다). 다른 모양도 있다 — Ruby 는 경로 필터를 **push 에만** 두고 PR 은 늘 돌린다("Do not use paths-ignore for required status checks"). 이유는 GitHub 이 문서로 못 박은 비대칭 하나다 — 경로 필터로 건너뛴 워크플로의 체크는 「Pending」으로 남아 머지를 막고, 조건(`if:`)으로 건너뛴 잡은 「Success」로 보고된다. 우리는 branch protection 을 걸지 않기로 했으므로(`docs/tracking.md` T-18) 트리거 단이 맞고, 걸게 되면 ADR-0232 재론 트리거 ①대로 잡 단위로 옮기거나(표본 다수) PR 쪽 필터를 걷는다(Ruby). **확신도 = 확실**(Pending/Success 비대칭 — 문서) · **가능성 높음**(어느 쪽이 많은가 — 표본).
2. **「문서만이면 건너뛴다」는 목적에는 제외 목록이 관례다.** 포함 목록도 흔하지만 주로 쓰임이 다르다 — **영역마다 워크플로를 따로 두고 각자 자기 영역만 보는 분할**(Tauri = crate 별 · Polars = 언어별 · WezTerm = 플랫폼별)에서 쓴다. 반례도 있다 — Biome 은 lint·test·e2e·문서 잡이 든 넓은 PR 워크플로에 포함 목록을 건다(적대 리뷰 적출). 그러니 「포함 목록은 분할 워크플로 전용」이 아니라 「분할에서 많이 보인다」까지다. 「문서 전용이면 건너뛴다」를 게이트 하나로 두는 곳은 제외 모양이다(Node · Ruby · Playwright · Ruff 의 `code` 판정 · Deno · Next.js 의 docs 그룹 · Pants · Prow 문서의 예시). 어느 쪽을 권하는 공식 지침은 찾지 못했다 — 이 결론은 관행에서 끌어낸 것이다. **확신도 = 가능성 높음.**
3. **문서가 테스트 입력이면 그 경로를 빼지 않거나 좁게 빼는 것이 관례다 — 목적은 우리 `qa.md` 되살리기와 같고 모양은 다르다.** Node 는 `doc/api/addons.md` 에서 addon 테스트를 만들기 때문에 Linux 워크플로에서 `doc/` 를 빼지 않았고, Ruff 는 「ty 의 테스트가 Markdown 으로 쓰여 있으니 Markdown 전체를 빼지 말고 `docs/**` 처럼 구체적으로 빼라」를 워크플로 주석에 적었다. Node·Vite 는 자기 워크플로 파일을 무시 목록에서 `!` 로 되살린다(문서가 아니라 워크플로 파일이지만 「빠진 경로 안의 한 파일을 되살린다」는 같은 기법이다). 작은 저장소지만 README·`docs/**` 를 `include_str!` 로 읽으면서 경로 필터에서 뺐다가 doctest 깨짐을 놓친 실사례도 있다(delta-arrow-reader #135 → 필터에 그 경로를 추가). **확신도 = 확실.**
4. **아예 안 건너뛰는 것도 관례다.** tokio · Bevy · cargo · TypeScript · VS Code(PR 워크플로)는 경로로 거르지 않는다. 비용을 치르고 단순함을 산 선택이다. 우리 사용자의 요구(「기다림이 거슬린다」)와는 반대편이다. **확신도 = 확실.**
5. **`[skip ci]` 커밋 메시지를 쓰는지는 이 조사로 알 수 없다.** 수집자가 약 500개 워크플로 파일에서 그 문자열 0건을 셌지만, 그 기능은 GitHub 내장이라 워크플로 파일에 흔적이 남지 않는다(적대 리뷰 적출). 워크플로에 자기 규칙으로 박은 곳은 없었다는 것까지만 말할 수 있다. **확신도 = 불확실.**

### 우리 문서에서 고칠 것 — 이 조사가 드러낸 과장

- ★**「`paths-ignore` 로는 되살리기를 표현할 수 없다(`!` 는 `paths` 에서만 먹는다)」는 문서가 말하는 것보다 세다**★ — ADR-0232 결정 1·거부한 대안·근거 ⑤와 `ci.yml` 주석이 이렇게 적었다. 문서가 실제로 말하는 것은 둘이다: ① 「제외만 할 때 `paths-ignore`, 포함과 제외를 섞을 때 `!` 를 붙인 `paths`」를 **권한다** ② 필터 패턴 규칙 전반에 "`!`: At the start of a pattern makes it negate previous positive patterns." 가 있다. `paths-ignore` 안의 `!` 가 **먹는다고도 안 먹는다고도** 적지 않는다.
  - 쓰는 곳은 있다 — Node 의 `test-linux.yml` 은 `paths-ignore` 안에서 `.github/**` 를 무시한 뒤 `'!.github/workflows/test-linux.yml'` 로 되살리고, `.github/` 아래 파일만 바꾼 main 커밋 4건(`abc66cbda1` · `735a09f999` · `ffa62401d7` · `9c7c88c52c`)에 「Test Linux」가 push 로 돌았다(`gh api …/actions/runs?head_sha=`, 2026-09-26).
  - ★**그 실행 기록을 증명으로 쓰지 않는다**★ — 그 push 들이 커밋 하나씩이었는지 확인하지 않았다(여러 커밋이 한 번에 밀렸으면 diff 에 다른 파일이 섞였을 수 있다). 적대 리뷰도 이 추론을 「증명 아님」으로 쳤다(아래). **확신도 = 불확실.**
  - **처분:** 우리 `paths` 모양은 문서가 권하는 모양이고 우리 저장소에서 되살리기까지 실측했으므로 **설정은 그대로 둔다.** ADR-0232·`ci.yml` 주석의 근거 문장만 「문서가 권한다」 수준으로 줄인다.
- **이름 바꾸기(rename)는 옛 경로와 새 경로를 모두 평가하는 것으로 보인다** — GitHub 문서에는 없고, 제3자 이슈 하나가 실행 기록과 함께 보고한다(https://github.com/thekevinscott/willfire/issues/237 — `packages/component/**` 로 거른 워크플로가 그 밖으로 파일을 옮긴 변경에 돌았다. ★**push 가 아니라 PR 이벤트 사례다**(실행 35991758803)★ · 그 PR 의 현재 파일 목록이 이슈 서술과 달라져 지금은 다시 대조할 수 없다) · 커뮤니티 답변 하나가 같은 말을 한다(https://github.com/orgs/community/discussions/164673). ADR-0232 의 「미확인 — 이름 바꾸기」는 **가능성 높음: 양쪽 평가**로 낮출 수 있다(그렇다면 그 위험은 거의 없다). **확신도 = 가능성 높음**(출처 둘 다 비공식).

## 발견 상세

### A. 실제 저장소 — 누가 무엇을 쓰나

| 프로젝트 | 방식 | 필수 체크 처리(보이는 범위) | 근거 | 확신도 |
|---|---|---|---|---|
| python/cpython | 잡 단위 — `compute-changes` 가 `run-docs`·`run-tests` 등을 내고 잡마다 `if:` | `all-required-green` — "only used for the branch protection", `re-actors/alls-green` 에 **조건부 `allowed-skips`** | `.github/workflows/build.yml` @ ee1bbf037ff7 | 확실(메인 대조) |
| vercel/next.js | 잡 단위 — `scripts/run-for-change.mjs --not --type docs`, docs 그룹에 `docs`·`errors`·`examples`·`.claude` 등 | `tests-pass`(`if: always()`) · 감지 실패 시 전부 돈다("if we fail to detect the changes run the command") | `build_and_test.yml` @ abac2089cd97 | 확실(메인 대조) |
| electron/electron | 잡 단위 — 자체 paths-filter 액션 · 문서 전용이면 별도 TS 컴파일 파이프라인만 | `gha-done` 집계 · 「조건부 skip 은 success 로 친다」를 주석에 적었다 | `apply-patches.yml`·`build.yml` @ 0a6c0bb09bfe | 확실(메인 대조) |
| zed-industries/zed | 잡 단위 — `git diff --name-only` + grep · main push 는 전부 | `tests_pass` · `merge_group` | `run_tests.yml` @ 933d8d93819c | 가능성 높음 |
| astral-sh/ruff | 잡 단위 — `git diff --quiet MERGE_BASE...HEAD -- ':!docs/**' ':!assets/**'` → `code` | `required-checks-passed` | `ci.yaml` @ f1ac2e7986b1 | 확실(메인 대조 — Markdown 주석) |
| astral-sh/uv | 잡 단위 — `plan.yml` 이 파일을 분류 | `required-checks-passed` | `ci.yml`·`plan.yml` @ 716f320609fe | 가능성 높음 |
| denoland/deno | 잡 단위 — `doc/` 아래만이면 docs_only · 빌드·테스트는 건너뛰고 lint 는 돈다 · diff 실패 시 전부 | 집계 잡 못 봄 | `ci.generated.yml` @ b157cd27e153 | 가능성 높음 |
| vitejs/vite | 잡 단위 — `tj-actions/changed-files` · `.github/**` 에서 `ci.yml` 은 제외 | `test-passed`/`test-failed` | `ci.yml` @ bc598a6a8a6b | 가능성 높음 |
| rust-lang/rust-analyzer | Rust CI 는 늘 돈다 · TS 잡만 `dorny/paths-filter` | `conclusion` 잡 · `merge_group` | `ci.yaml` @ 1ad44dc58e65 | 확실(메인 대조) |
| servo/servo | 경로 아님 — 이벤트 종류로 PR 을 줄이고 merge queue·main 에서 전부 | `build-result` 집계 | `main.yml` @ d05154e2b4de | 가능성 높음 |
| nodejs/node | 트리거 단 `paths-ignore`(PR·push) · `.github/**` 무시 후 자기 파일 `!` 되살림 · `doc/` 는 무시하지 않음 | 안 보임 | `test-linux.yml` @ 4889fb0a4373 | 확실(메인 대조 + 실행 기록) |
| ruby/ruby | 트리거 단 `paths-ignore` 를 **push 에만** — PR 쪽은 필수 체크가 멈춰서 걷었다 | 주석 "Do not use paths-ignore for required status checks" | `ubuntu.yml` · 커밋 4303a02f46 · 8d4ba9d443 | 확실(메인 대조) |
| microsoft/playwright | 트리거 단 `paths-ignore`(PR, `docs/**` 등) | 안 보임 | `tests_primary.yml` @ b9a34ac7783a | 가능성 높음 |
| facebook/react | 트리거 단 — 런타임 CI 는 `compiler/**` 무시, 컴파일러 워크플로는 `compiler/**` 만 | 안 보임 | `runtime_build_and_test.yml` @ d083ec1da1e5 | 가능성 높음 |
| tauri-apps/tauri | 트리거 단 **포함 목록**(PR 만 — `crates/**`·Cargo 파일·자기 워크플로 + `!` 제외) · `dev` push 는 늘 돈다 | 안 보임 | `test-core.yml` @ 15468de79772 | 확실(메인 대조) |
| wezterm/wezterm | 트리거 단 **포함 목록**(`**/*.rs`·Cargo·자산·자기 워크플로) — 플랫폼별 워크플로 | 안 보임 | `gen_windows.yml` @ b09b56c29c1e | 확실(메인 대조) |
| pola-rs/polars | 트리거 단 **포함 목록**(PR·push) — 영역별 워크플로 | 안 보임 | `test-rust.yml` @ 5fdcc2ffe33a | 확실(메인 대조) |
| biomejs/biome | 트리거 단 포함 목록(PR) + `merge_group` 은 거르지 않음 | 불명 | `pull_request.yml` @ 6964bfc8a13c | 가능성 높음 |
| tokio · Bevy · cargo · TypeScript · VS Code(PR) | 경로로 거르지 않는다 | Bevy·TypeScript = `merge_group` | 각 `ci.yml` 등 | 가능성 높음 |
| rust-lang/rust | YAML 에 문서 경로 건너뛰기 없음 · 잡 집합은 `citool` 이 계산(안 읽음) | bors | `ci.yml` @ 5ceaf6608eb3 | 불확실 |

- **Electron 의 금지 조항** — "Never add .github/workflows/** or .github/actions/** here - CI changes can affect builds" (`build.yml`). 우리 목록도 `.github/` 를 빼지 않는다.
- **실패 시 전부 돈다(fail-open)** — Deno("An empty diff defaults to running the full CI to be safe.")·Next.js 는 감지가 실패하면 CI 전체를 돌린다. 트리거 단 필터는 이 선택권이 없다 — GitHub 이 커밋 1,000개 초과·diff 시간 초과에선 돌리고(fail-open), **3,000 파일 초과에선 안 돌릴 수 있다(fail-closed, 알림 없음)**. 우리는 이것을 받아들인 예외로 적었다(ADR-0232).

### B. GitHub 동작과 함정 (공식 문서)

출처 = https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax 의 `paths`/`paths-ignore` 절·필터 패턴 절, https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks, https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs. 아래는 모두 **확실**(문구 대조)이며 ADR-0232 근거 ①~⑤와 겹치는 것은 다시 적지 않는다.

- **merge queue(`merge_group`)는 `branches` 만 받고 `paths` 를 못 받는다** — merge queue 를 쓰면 트리거 단 경로 건너뛰기는 성립하지 않는다(잡 단위 감지가 필요하다).
- **집계 잡의 함정** — `needs:` 로 매단 잡이 실패하거나 건너뛰면 매단 잡도 건너뛰고, 건너뛴 잡은 성공으로 친다. 그래서 집계 잡에 `if: always()` 를 빼먹으면 **실패가 초록으로 둔갑한다**. Next.js 주석도 같은 경고를 적었다.
- **`[skip ci]` 류** — `push`·`pull_request` 에만 먹고 `pull_request_target` 에는 안 먹으며, 필수 체크를 Pending 으로 남긴다. 여러 커밋 push 에서 머리 커밋 말고 다른 커밋의 표식도 먹는지는 문서와 2021 changelog 가 서로 다르게 말한다(**불확실**).
- **`concurrency`** — 취소는 「새 실행이 시작될 때」 일어난다. 경로 필터로 실행이 안 생긴 push 는 앞 실행을 취소하지 않는다(문구의 필연적 독해 — **가능성 높음**). 우리 브랜치의 문서만 담은 push(`60c1249`)는 실행 0건이었지만 ★**그때 앞 실행은 이미 끝나 있어 취소 여부는 시험되지 않았다**★.
- **`'**'` 와 점으로 시작하는 경로** — GitHub 은 트리거 필터의 매칭 엔진·점 처리 규칙을 문서화하지 않는다. `actions/labeler` 의 「minimatch 가 점 파일을 안 잡는다」 이야기는 **다른 도구의 것**이라 옮겨 오면 안 된다. 직접 증거는 우리 브랜치 실측 하나(`.github/workflows/ci.yml` 만 문서 밖인 `be71106` push 에 실행이 생겼다)다 — ★그 push 는 새 브랜치 생성 push 라 새 브랜치 diff 규칙을 탔다★(기존 브랜치 push 로는 재지 않았다 · 다음 `e9abf55` 는 `qa.md` 를 글자 그대로 되살리는 양성 패턴이라 이 질문의 시험이 아니다). **가능성 높음(n=1).**

### C. 서드파티 감지 액션의 보안

- **`tj-actions/changed-files` 공급망 사고 — CVE-2025-30066 / GHSA-mrrh-fwg8-r2c3**(2025-03-14~15). 버전 태그들이 악성 커밋으로 소급 이동돼 러너 메모리의 비밀을 워크플로 로그에 찍었고, 공개 저장소에서는 그 로그가 공개였다. 23,000개 넘는 저장소가 영향권이었고 v46.0.1 에서 고쳤다(https://github.com/advisories/GHSA-mrrh-fwg8-r2c3 · CISA 경보). **확실.**
- GitHub 지침 — "Pinning an action to a full-length commit SHA is currently the only way to use an action as an immutable release."(https://docs.github.com/en/actions/reference/security/secure-use) 우리 워크플로는 이미 액션을 SHA 로 핀한다(ADR-0131). 잡 단위로 옮길 때는 **직접 쓴 `git diff` 스텝**이 서드파티 신뢰를 아예 없앤다(Ruff·Zed·Deno 가 그렇게 한다) — 대가는 fetch 깊이·새 브랜치의 `before` 가 0 인 경우·강제 push 뒤 도달 불가 커밋을 직접 다뤄야 한다는 것(일반 지식 — **가능성 높음**).

### D. 포함 목록 vs 제외 목록 — 근거와 사고

- **공식 지침은 없다.** 어느 쪽을 권한다고 이름 붙인 GitHub·대형 조직의 문서를 찾지 못했다.
- Kubernetes Prow 는 두 모양을 서로 배타적인 필드로 제공하고(`run_if_changed` · `skip_if_only_changed`), 「문서만 바꾼 PR 에서 컴파일 잡을 건너뛰기」 예시를 **제외 쪽 필드**에 든다(https://docs.prow.k8s.io/docs/jobs/). **확실**(문서 대조는 수집자).
- **큰 저장소에서 경로 필터가 조용히 CI 를 건너뛰어 깨진 코드가 main 에 들어간 사후 분석은 찾지 못했다.** 기록된 것은 리뷰에서 잡힌 아슬아슬한 사례(Node #37999), 필수 체크가 멈춘 사례(Ruby), 글롭 실수(Node 의 `*.nix` → `**.nix` — 이건 불필요하게 더 돈 쪽이다), 작은 저장소의 doctest 누락(delta-arrow-reader #135)이다.

## 제약 적합도 (설계-결정 모드)

우리 제약 = public 저장소 · Windows 단일 러너(검증 약 7분) · branch protection 없음(T-18) · merge queue 없음 · 워크플로 하나가 전체를 검사 · 액션은 SHA 핀 · 문서 파일 하나가 테스트 입력.

| 후보 | 문서 push 대기 제거 | 필수 체크 양립 | 서드파티 신뢰 | 복잡도 | 우리에게 |
|---|---|---|---|---|---|
| 트리거 단 제외 목록(현행 — `paths` + `!`) | ○ 러너 0 | ✕ (Pending) — 지금은 필수 체크 없음 | 없음 | 최소 | **맞다** |
| 트리거 단 `paths-ignore` + `!` 되살리기(Node 모양) | ○ | ✕ | 없음 | 최소 | 되살리기를 문서가 보장하지 않는다 — 바꿀 이득 없음 |
| 트리거 단 포함 목록 | ○ | ✕ | 없음 | 목록 관리 | 영역별 분할이 아니라 전체 검사 워크플로라 안 맞는다 |
| 잡 단위 + 직접 쓴 `git diff` + 집계 잡 | △ 러너는 뜨고 무거운 잡만 건너뜀 | ○ | 없음 | 중간 | branch protection 을 걸게 되면 첫 후보(ADR-0232 재론 ①) |
| 잡 단위 + 서드파티 감지 액션 | △ | ○ | 있음(CVE-2025-30066 전례) | 중간 | 굳이 들일 이유 없음 |
| 거르지 않음 | ✕ | ○ | 없음 | 최소 | 사용자 요구와 반대 |
| `[skip ci]` | 사람이 붙여야 함 | ✕ | 없음 | 최소 | 관행 없음 · 잊기 쉽다 |

**거부 후보(ADR 거부 대안 후보):** 서드파티 감지 액션(보안 전례 + 직접 쓴 diff 로 대체 가능) · `[skip ci]`(관행 없음 · 사람 기억에 기댐). 둘은 이번 결정에 새로 더할 가치가 있으나, ADR-0232 는 이미 확정됐으므로 **덧붙일지는 사용자 결정**이다.

## 적대 리뷰 결과

Codex(cross-family) 1회 · 판정 **FIX** · 스팟체크 8건 중 7건 유지(GitHub Pending/Success 비대칭 · CPython 집계 잡 · Next.js 감지·집계 · Ruff Markdown 주석 · Ruby push 전용 필터 · Vite · 3,000 파일 한도).

| 적출 | 처분 |
|---|---|
| `paths-ignore` 안 `!` 되살리기를 Node 실행 기록으로 「고쳐야 할 틀린 서술」이라 단정했다 | **수용(확신도 강등)** — 위 「고칠 것」을 과장 정정으로 바꾸고 불확실로 내렸다. 단 리뷰의 반박 논리(「그 커밋들이 `test-linux.yml` 자체를 바꿨으니 분리가 안 된다」)는 반증이 아니다 — 그 파일이 바뀌는 것이 되살리기 시험의 전제다. 불확실로 둔 근거는 우리 쪽 결함(커밋별 push 미확인)이다 |
| 「필수 체크를 거는 대형 프로젝트는 거의 전부 잡 단위」 — 모수 없는 비율 주장 | **수용** — 관찰 표본 서술로 바꾸고 Ruby 의 다른 모양을 더했다 |
| `[skip ci]` 0건 — 워크플로 파일 검색으로는 커밋 메시지 사용을 못 잰다 | **수용** — 불확실로 내렸다 |
| 「포함 목록은 분할 워크플로용」 — Biome 반례 | **수용** — 「분할에서 많이 보인다」로 줄였다 |

## 쟁점 · 한계

- **`paths-ignore` 안의 `!` 가 되살리는가 — 판정 불가(불확실)** — 메인은 Node 실행 기록으로 「된다」 쪽, 적대 리뷰는 「증명 아님」 쪽이었다. 결론(설정 유지)에 영향이 없는 마이너 쟁점이라 판결하지 않고 불확실로 남긴다. 풀려면 커밋 하나만 밀어 `.github/` 아래 자기 워크플로 파일만 바꾼 push 한 건을 찾으면 된다.
- **수집자 간 상충 하나를 메인이 원본으로 판정했다** — 한 갈래는 「대형 저장소 중 워크플로 단 포함 목록을 쓰는 곳은 없다」고 했고, 다른 갈래는 Tauri·WezTerm·Polars·Biome 을 포함 목록으로 분류했다. 세 곳의 원본 파일을 열어 포함 목록임을 확인했다 — 앞 갈래의 일반화가 틀렸다.
- **「필수 체크를 거는가」는 공개 파일로 알 수 없다** — 표의 필수 체크 칸은 집계 잡·주석에서 추론했다. 명시한 곳은 CPython·Electron 뿐이다.
- **안 읽은 것** — CPython `compute-changes.py` · rust-lang/rust `citool` · VS Code Azure Pipelines · Bun Buildkite · Bevy/TypeScript/rust-analyzer/Deno/Biome 의 집계 잡.
- **GitHub Actions 밖(GitLab 등)은 범위 밖이다** — 한 갈래가 GitLab `rules:changes` 가 새 브랜치 push 에서 늘 참이 되는 함정을 덧붙였으나 우리와 무관하다.
