# 검사 통과 기록의 캐시 키 — 내용 지문 vs 수정 시각 (관행 조사)

- **상태:** light 조사 · 2026-10-18 · 수집자 1(문서형) + 메인 grounding(Prettier · Gradle 원문 대조) · 적대 리뷰 없음
- **질문:** 「이 파일은 이미 검사를 통과했다」를 앱을 다시 켠 뒤에도 남겨 건너뛰려면 무엇을 키로 쓰고, 도구 버전을 키에 넣는가, 어디에 두는가, 도구가 꺼져 있는 동안 고친 것은 어떻게 잡는가.
- **맥락:** 설정 에셋마다 claude · codex CLI 검사(약 2초)의 통과 기록을 (내용 지문 · CLI 버전)으로 남기려는 설계.

## 결론

- **정확성이 중요하면 키는 내용 지문이다.** Prettier 는 기본이 내용이고, Gradle 은 내용 해시를 키로 쓰되 수정 시각 · 크기가 같으면 지문을 다시 계산하지 않는 지름길만 둔다. ESLint 는 기본이 수정 시각이고 내용은 선택이다. — 확실(Prettier · Gradle 원문 대조)
- **도구 버전 · 설정은 키에 들어간다** — Prettier(버전 · 옵션 · Node 버전) · Gradle(작업 코드) · ESLint(설정 · 버전). 빠뜨린 것(Prettier 의 플러그인)은 문서가 「캐시를 지우라」고 따로 경고한다. — 가능성 높음
- **도구가 꺼져 있던 동안의 편집은 다음 실행에서 저장된 지문과 비교해 잡는다** — 도구가 떠 있을 필요가 없다. 수정 시각 방식은 시각 · 크기를 보존한 편집을 놓치고, git clone · checkout 은 내용 없이 시각만 바꾼다(그래서 Prettier · ESLint 가 내용 방식을 둔다). — 가능성 높음
- **저장 = 도구 전용 파일 하나**(`.eslintcache` · `node_modules/.cache/prettier/.prettier-cache` · `.tsbuildinfo`) · 위치는 플래그로 바꿀 수 있다. — 가능성 높음
- **이 설계 (내용 지문 · CLI 버전) 는 관행과 맞는다.** 작은 YAML 해시 비용은 2초 검사에 비해 무시할 만하다(수치 출처 없음 — 추론).

## 피어별

| 피어 | 키 | 도구 버전 | 저장 | 확신도 | 출처 |
|---|---|---|---|---|---|
| Prettier `--cache` | 기본 내용(「If no strategy is specified, `content` will be used.」) · 선택 metadata | 키에 포함(「Prettier version, Options, Node.js version」) · 플러그인은 빠짐(「not used as cache keys」) | `./node_modules/.cache/prettier/.prettier-cache` | 확실(원문 대조) | https://prettier.io/docs/cli |
| Gradle 증분 빌드 | 경로 + 내용 해시 · 「If the stats of a file (i.e. lastModified and size) did not change, Gradle will reuse the file's fingerprint」 | 「Gradle also considers the code of the task as part of the inputs」 | 확인 못 함 | 확실(원문 대조) | https://docs.gradle.org/current/userguide/incremental_build.html |
| ESLint `--cache` | 기본 metadata(수정 시각) · 선택 content | 설정 · 버전 바뀌면 무효(요약 인용) | `.eslintcache`(cwd) | 가능성 높음 | https://eslint.org/docs/latest/use/command-line-interface |
| TypeScript `.tsbuildinfo` | 파일마다 version(내용 해시) + signature | 미확인 | JSON 파일 | 가능성 높음(2차 자료) | https://deepwiki.com/microsoft/TypeScript/8.3-builderprogram-and-state |
| git racy-git | 수정 시각이 색인 시각 이후면 내용 비교 | — | — | 확실 | https://www.kernel.org/pub/software/scm/git/docs/technical/racy-git.html |

## 공백

- pre-commit · Bazel · Nx · Turborepo 는 조사 안 함. Gradle 캐시 위치 · TypeScript 버전 필드 미확인. 캐시 파일이 깨지거나 동시에 쓰일 때의 처리 미조사.

## 덧붙임 — 낡은 기록 지우기 (2026-10-18 · light · file-entry-cache README 원문 대조)

- **파일마다 결과를 남기는 검사 캐시(ESLint · Prettier — 둘 다 `file-entry-cache`)는 키가 경로다.** 저장할 때 모든 키를 훑어 **디스크에 없는 파일의 기록을 지운다** — 「Saves the cache to disk and removes any files that are no longer found.」(README · 원문 대조). 내용 비교는 선택(`useCheckSum` — 「use a checksum to determine if the file has changed」). — 가능성 높음 https://github.com/jaredwray/cacheable/blob/main/packages/file-entry-cache
- **버전이 바뀌어도 통째로 지우지 않는다** — 도구 버전 · Node 버전 · 설정을 해시해 기록마다 붙이고, 어긋나면 그 기록이 빗나간 것으로 보고 덮어쓴다(ESLint `lint-result-cache.js` · Prettier `format-results-cache.js`). Prettier 는 캐시 파일 모양이 깨졌으면 파일을 지우고 다시 만든다. — 가능성 높음(소스 요약)
- **내용 지문만 키로 쓰는 저장소(ccache · Bazel 디스크 캐시)는 「안 쓰이는 지문」으로 지우지 않고 크기 상한 + 오래된 순(LRU · 나이)으로 지운다.** — 확실(수집자 인용) https://ccache.dev/manual/latest.html · https://bazel.build/remote/caching
- **「지금 어떤 파일과도 지문이 안 맞는 기록 지우기」는 선례를 못 찾았다** — 이 경우의 관행 = 위 ESLint · Prettier 꼴(경로 키 + 내용 비교 + 없는 경로 지우기 + 버전 어긋나면 덮어쓰기).
