# claude · codex 는 SKILL.md 를 어떻게 알아보고 이름 붙이나 (2026-10-20)

| | |
|---|---|
| **상태** | 조사 완료 · 결정 전(S25 프리셋 라이브러리의 스킬 인식 · 이름 · 충돌 규칙 입력) |
| **방법** | 공식 문서 2쪽(claude skills · plugins-reference) · 소스(codex 태그 `rust-v0.156.1` = `b412ff32` · claude 는 설치본 `claude.exe` 에 든 번들 JS 를 문자열로 정독) · 실측(같은 가짜 스킬 26종을 두 CLI 에 물림 — 아래 「방법」). **실 모델 호출 0**(claude = 로컬 가짜 API 서버 · codex = `skills/list` · `debug prompt-input`) |
| **대상판** | Claude Code **2.1.284** · codex-cli **0.156.1** · Windows 11(26200) |
| **질문** | ① 파일 이름 대소문자 ② 깊이(묶음 폴더 · 스킬 안 스킬) ③ 앞머리 필수 칸 · 깨졌을 때 ④ 정체가 되는 이름 · 제약 ⑤ 이름 겹침 ⑥ 정션 · 링크 ⑦ claude 플러그인 경로의 최소 모양 · 이름 ⑧ 스킬 폴더의 다른 특별 파일 |
| **앞선 보고서** | [`external-skill-dir-loading-claude-codex-2026-10-10.md`](external-skill-dir-loading-claude-codex-2026-10-10.md)(발견 위치 · `--plugin-dir` · codex 폴더) · [`codex-instruction-channels-2026-10-20.md`](codex-instruction-channels-2026-10-20.md)(가짜 서버 · 덫 프록시 방법) |
| **확신도 범례** | 확실 = 실측(또는 실측 + 소스 교차) · 가능성 높음 = 소스나 문서만 · 불확실 = 미검 |

## 결론

1. **두 CLI 는 「스킬」 을 다르게 알아본다 — 너그러운 claude, 엄격하고 깊게 훑는 codex.**
   - **claude** 는 `<뿌리>/<폴더>/SKILL.md` **한 층만** 경로를 붙여 열어 본다. 그래서 대소문자는 **OS 규칙**이고(Windows 에선 `skill.md` 도 실림), 앞머리가 없거나 깨져도 싣는다.
   - **codex** 는 뿌리 아래를 **6층까지 재귀**로 걸으며 파일 이름이 **정확히 `SKILL.md`** 인 것만 줍는다(Windows 에서도). 앞머리 · `description` 이 없거나 YAML 이 깨지거나 BOM 이 붙으면 **그 스킬을 빼고** 오류 목록에만 올린다.
2. **이름의 정체도 다르다.**
   - **claude:** 정체는 **폴더 이름**이다. 플러그인이면 `<플러그인>:<폴더 이름의 [^A-Za-z0-9_-] 를 - 로>` 꼴이다. 앞머리 `name` 은 표시 이름이자 별칭으로만 붙는다.
   - **codex:** 정체는 **앞머리 `name`**(공백 접기)이고, 없으면 **정션을 따라간 실제 폴더 이름**이다. 64자 상한이 있다. 실제 폴더의 조상에 플러그인 manifest 가 있으면 `<플러그인>:` 접두가 붙는다.
3. **이름 겹침은 어느 쪽도 막지 않는다.**
   - **claude:** 같은 **실제 파일**만 하나로 합친다. 같은 표시 이름 둘을 이름으로 부르면 **오류 없이 정렬상 첫째**가 돈다.
   - **codex:** 같은 **정규 경로**만 합친다. 이름이 겹치면 **둘 다 모델에게 보인다** — `$이름` 멘션만 모호해서 고르지 않는다.
4. **Engram 휘발 플러그인 경로(TRD §1-5)는 그대로 된다 — 실측 확실.** 지켜야 할 조건 셋:
   - **manifest 불요** — `--plugin-dir <휘발>\engram` 에 `skills\` 만 있으면 한 플러그인이다.
   - **이름** — 이름은 `engram:<정션 이름>` 이다. TRD 의 「정션 따라가기 미측」 은 이번 실측으로 확인됐다.
   - **두지 말 것** — `skills\SKILL.md` 를 두면 안 된다. 그 하나만 스킬이 되고 자식은 무시된다.
5. **Engram 규칙과 어긋나는 곳은 다섯이다**(아래 「Engram 규칙과 어긋나는 곳」).
   - `skill.md` 인식 — codex 가 조용히 버린다.
   - 붙인 스킬 안의 스킬 — codex 에만 새어 들어간다.
   - 앞머리 형식 무검사 — codex 가 조용히 빠뜨린다.
   - 이름 문자 · 길이 무검사 — claude 가 이름을 바꾸고, Windows 폴더 이름이 못 되는 이름이 있다.
   - 충돌 키의 대소문자 · 치환 — claude 쪽 내부 이름이 부딪힌다.

## 질문별 답

| # | claude 2.1.284 | 근거 · 확신도 | codex 0.156.1 | 근거 · 확신도 |
|---|---|---|---|---|
| **1 파일 이름** | 폴더 경로에 `SKILL.md` 를 붙여 연다 → **OS 파일 시스템 규칙**. Windows 실측: `skill.md` · `Skill.md` 모두 실림(플러그인 `engram:c-lower` · `engram:c-mixed`, 프로젝트 `p-lower`). 문서는 대소문자를 적지 않는다(예시는 전부 대문자) | 실측 + 번들(`join(dir,"SKILL.md")` 뒤 stat · readFile) · **확실** | 걷기에서 얻은 파일 이름을 **`== "SKILL.md"` 로 바이트 비교**(`ext/skills/src/loader/discovery.rs:176`) → OS 무관 정확 일치. Windows 에서 `skill.md` · `Skill.md` 는 **목록에도 오류에도 없다** | 실측 + 소스 · **확실** |
| **2 깊이** | **한 층만**: `<뿌리>/<폴더>/SKILL.md`(플러그인 `skills/` · 개인 · 프로젝트 공통). 묶음 폴더 아래(`c-group/c-nested`) · 스킬 안 스킬(`c-outer/c-inner`) · 더 깊은 것 **안 실림**(디버그 로그 「Loaded 23 skills」 = 26 항목 − 셋). 예외: 플러그인 `skills/` 바로 안에 `SKILL.md` 가 있으면 **그 하나만** 스킬(소스) · 프로젝트는 하위 폴더의 `.claude/skills` 도 그 폴더 파일을 만질 때 읽는다(문서) | 실측 + 번들 · **확실** | **재귀**: 최대 6층 · 폴더 2000 · 항목 20000(`loader/mod.rs:31-32` · `discovery.rs:49`). 모든 `SKILL.md` 가 스킬 — `c-nested` · `c-inner` · `c-deep6` 실림, `c-deep7` 안 실림. **점(.) 폴더 아래는 건너뜀**(`.c-hidden` 없음). 플러그인의 Agent Plugin 루트만 직속 자식 모드 | 실측 + 소스 · **확실** |
| **3 앞머리** | 전부 선택. 앞머리 없음 → 그대로 실림 · YAML 깨짐 → 「필드 없이」 실림(경고 로그) · BOM 허용 · CRLF 허용. 설명이 없으면 본문 첫 줄을 설명으로 쓴다(프로젝트 `p-nofm` = 「Heading」). ★단 **플러그인 경로에서 설명을 직접 안 쓴 스킬**(`c-nofm` · `c-nodesc` · `c-badyaml`)은 init `slash_commands` 에만 있고 **모델에게 보내는 스킬 목록엔 없다**(이름을 대 Skill 도구로 부르면 돈다 — `engram:c-nofm` 실측). `claude plugin validate`: 깨진 YAML = error · 앞머리/설명 없음 = warning | 실측 · **확실**(모델 목록에서 빠지는 이유는 코드로 못 짚음 = 가능성 높음) | **필수**: 첫 줄 `---` · 닫는 `---` · 비지 않은 블록 · YAML 파싱(값에 `: ` 가 든 줄은 한 번 따옴표로 고쳐 다시 읽음) · `description` 비지 않음 · `name` ≤ 64자(`skills/src/parser.rs`). 어기면 **그 스킬은 빠지고** `skills/list` 의 `errors` 에 오른다(TUI = 「Skipped loading N skill(s) due to invalid SKILL.md files.」). ★**BOM 이 붙으면 「missing YAML frontmatter」 로 빠진다**(`c-bom`). `name` 은 선택 | 실측 + 소스 · **확실** |
| **4 이름** | **정체(내부 이름 · 모델 목록 이름) = 폴더 이름.** 플러그인 = `<플러그인>:` + 폴더 이름의 `[^A-Za-z0-9_-]` 를 `-` 로(`.c-hidden` → `engram:-c-hidden`). 앞머리 `name` = `/` 메뉴 표시 이름이자 별칭 — Skill 도구가 `engram:c-fm-other` · 맨 `c-fm-other` 둘 다 받는다. 맨 폴더 이름(`c-dir-mismatch`)은 거부(「Did you mean engram:c-dir-mismatch?」). 앞머리 이름이 다르면 모델 목록엔 폴더 쪽이 뜬다(디버그 「Skill prompt: showing "engram:c-dir-mismatch" (userFacingName="engram:c-fm-other")」). **길이 상한 없음**(65자 실림). 문자: 문서상 제어문자 · zero-width · bidi · `<>` · 낱칸 공백 밖의 공백을 거부. 공백 · 점 · 한글 · `:` 는 앞머리 이름으로 실림(`c-weird` · `c-colon`) | 실측 + 번들 + 문서 · **확실** | **정체 = 앞머리 `name`**(공백 접기). 없거나 빈 문자열이면 **정규화된(정션을 따라간) 실제 폴더 이름**(`loader/host.rs:394` — `c-junc-alias` → `c-jtarget` · `engram-1234abcd-c-noname` → `c-noname`). **≤ 64자**(65 = 오류). **문자 제한 없음**(공백 · 점 · 대문자 · 한글 · `:` 실측). ★**실제 폴더의 조상에 `.codex-plugin/plugin.json` · `.claude-plugin/plugin.json` · `.cursor-plugin/plugin.json` 이 있으면 `<manifest name>:<이름>`**(`myplug:c-ps` 실측 · `loader/namespace.rs`). `$이름` 멘션은 `[A-Za-z0-9_:-]` 만 이어 읽어 공백 든 이름은 멘션으로 못 부른다(`skills/src/mentions.rs:226`) | 실측 + 소스 · **확실**(멘션 = 소스만) |
| **5 이름 겹침** | **같은 실제 파일(realpath)** 은 한 번만 — 플러그인끼리(`engram2:c-exact` · `engram2:other-dirname` 건너뜀 실측), 그리고 개인 스킬 폴더와 겹치면 플러그인 쪽을 건너뛴다(소스). 다른 파일이면 다 실린다: 개인 > 프로젝트 — 같은 이름이면 개인만(`c-pers` 실측 · 문서) · 플러그인은 접두로 갈라져 공존. 한 플러그인 안에서 앞머리 이름이 같은 둘(`c-dupA` · `c-dupB`)은 폴더 이름이 달라 **둘 다 실리고**, 겹친 표시 이름 `engram:c-dup` 으로 부르면 **오류 없이 정렬상 첫째(c-dupA)** 가 돈다 | 실측 + 번들 · **확실** | **같은 정규 `SKILL.md` 경로만** 하나로 합친다 — 먼저 오는 루트가 이긴다(순서 = `$CODEX_HOME/skills` · `~/.agents/skills` · 시스템 · 플러그인 · extraRoots · 프로젝트 `.agents/skills` — `host_roots.rs` · `host_merge.rs:232`). **이름 겹침은 오류도 우선순위도 없이 전부 목록에 오르고 모델 프롬프트에 둘 다 뜬다**(`c-dup` 셋 · 프롬프트 두 줄 실측). `$c-dup` 처럼 이름으로 부르면 개수 ≠ 1 이라 **고르지 않는다**(`skills/src/selection.rs`) | 실측(목록 · 프롬프트) + 소스(멘션) · **확실** |
| **6 정션 · 링크** | 따라간다 — 폴더 항목이 디렉터리거나 링크면 받는다. 이름 = **링크 이름**, 「Base directory」 = 링크 경로, 중복 판정 = realpath | 실측 · **확실** | 사용자 · 프로젝트 · 관리자 범위는 디렉터리 링크 · 정션을 따라간다(시스템 범위만 무시) · 같은 실제 폴더는 한 번만 걷는다. 이름 · 접두 = **실제 경로 기준**, 프롬프트의 경로 표시는 링크 경로(`r2/engram-1234abcd-c-exact/SKILL.md`). **파일 심볼릭 링크인 `SKILL.md` 는 걷기에서 빠진다**(`exec-server/src/local_file_system.rs` 의 `walk`) | 정션 = 실측 · **확실** / 파일 링크 = 소스만 · **가능성 높음**(만들 권한이 없어 못 잼) |
| **7 claude 플러그인** | **manifest 불요** — `--plugin-dir X` 에서 `X\skills\` 가 있으면 「is one plugin: skills at its top marks it」(디버그 로그). 플러그인 이름 = **X 폴더 이름**(init `plugins` = `engram` · source `engram@inline`). 최소 모양 = `X\skills\<이름>\SKILL.md`. 함정: ① `X\skills\SKILL.md` 가 있으면 그 하나만 스킬(번들) ② `X` 에 `skills\` 같은 표식이 없으면 「플러그인 묶음」 으로 보고 자식마다 플러그인(`--help`) ③ **`claude plugin validate` 는 manifest 가 없으면 실패**(「No manifest found in directory」) — 런타임은 멀쩡하다 ④ 링크를 안 따라간다(「components are read without following symlinks」) — 실제 경로를 따로 검사해야 한다 ⑤ 플러그인 이름 규칙(공백 · `@` · `:` · 경로 구분자 금지 · `claude-` · `anthropic-` 접두 예약 — 문서). `engram` 은 안전 | 실측 + 문서 + 번들 · **확실** | 해당 없음(JSON = `skills/extraRoots/set` 호스트 루트 · 터미널 = 작업 폴더 `.agents/skills` — 앞선 보고서). 단 **codex 도 `.claude-plugin/plugin.json` 을 플러그인 표식으로 읽는다**(4) | — |
| **8 특별 파일** | 개인 · 프로젝트 스킬 폴더 안의 `.claude-plugin/plugin.json` → `<이름>@skills-dir` 플러그인으로 실림(문서) · 예약 폴더 이름 `synced` · `anthropic-skills`(문서) · 본문의 `${CLAUDE_SKILL_DIR}` 등 치환 · 플러그인 뿌리의 `CLAUDE.md` 는 안 읽힘(문서). 나머지는 모델이 필요할 때 읽는 일반 파일 | 문서 + 번들 · **가능성 높음**(이번에 안 잼) | `agents/openai.yaml` — 표시 정보 · 의존 도구 · 정책(`allow_implicit_invocation: false` = 암묵 호출 대상에서 뺌 · `products` 제한 — `skills/src/model.rs`) · 조상의 플러그인 manifest 셋(4의 접두) · 점 폴더 무시. 나머지는 일반 파일 | 소스 · **가능성 높음** |

### 픽스처별 결과 (실측)

claude = `--plugin-dir <휘발>\engram`(항목마다 라이브러리로 가는 정션 — TRD §1-5 모양 그대로) · codex = `skills/extraRoots/set [lib]` 뒤 `skills/list`.

| 픽스처 | 내용 | claude | codex |
|---|---|---|---|
| `c-exact` | 기준 | `engram:c-exact` | `c-exact` |
| `c-lower` · `c-mixed` | `skill.md` · `Skill.md` | 실림 | **없음**(오류도 없음) |
| `c-group/c-nested` | 묶음 폴더 아래 | **없음** | `c-nested` |
| `c-outer` + `c-outer/c-inner` | 스킬 안 스킬 | `c-outer` 만 | **둘 다** |
| `c-deep6` · `c-deep7` | 6층 · 7층 | 없음 · 없음 | 있음 · 없음 |
| `.c-hidden` | 점 폴더 | `engram:-c-hidden` | 없음 |
| `c-noname` · `c-emptyname` | `name` 없음 · `name: ""` | 폴더 이름 | 폴더 이름 |
| `c-nodesc` · `c-nofm` · `c-badyaml` | 설명 없음 · 앞머리 없음 · YAML 깨짐 | 실림 · **모델 목록엔 없음** | **오류로 빠짐** |
| `c-bom` · `c-crlf` | BOM · CRLF | 정상 · 정상 | **오류로 빠짐** · 정상 |
| `c-dir-mismatch` | `name: c-fm-other` | `engram:c-dir-mismatch`(표시 `engram:c-fm-other`) | `c-fm-other` |
| `c-weird` · `c-colon` | `Weird Name.v2 한글` · `ns:c-colon` | `engram:c-weird` · `engram:c-colon`(표시는 앞머리 이름) | 앞머리 그대로 |
| `c-long64` · `c-long` | 64자 · 65자 | 둘 다 실림 | 실림 · **오류** |
| `c-dupA` · `c-dupB` | 같은 `name: c-dup` | 둘 다(폴더 이름으로 갈림) | 둘 다 `c-dup` |
| `c-caseA` · `c-caseB` | `C-Case` · `c-case` | 둘 다 | 둘 다 |
| `c-junc-alias` | 정션 → 이름 없는 `c-jtarget` | `engram:c-junc-alias` | **`c-jtarget`** |
| `c-junc-plug` | 정션 → `myplug\.claude-plugin` 아래 `c-ps` | `engram:c-junc-plug`(표시 `engram:c-ps`) | **`myplug:c-ps`** |

곁 실측 — claude 프로젝트 `.claude/skills`:

- `skill.md` 실림 · 묶음 폴더 아래 안 실림.
- `p-dirname` 의 앞머리 `name: p-fmname` 은 별칭으로만 돈다.
- 개인 `c-pers` 가 프로젝트 `c-pers` 를 가린다.

곁 실측 — codex 작업 폴더 `.agents/skills`:

- Engram 꼴 정션 `engram-1234abcd-c-exact` 는 extraRoots 쪽과 같은 실제 경로라 **한 줄로 합쳐졌다**.
- 같은 이름 둘(`w-samename` · `w-samename2`)은 프롬프트에 두 줄로 떴다.
- ★실 `~/.agents/skills`(이 PC 는 공유 스킬 폴더 정션) 아래의 claude 동기화 폴더 `synced/<uuid>/…` 까지 재귀로 주워 **`docs` · `pdf` · `pptx` 등이 둘씩** 프롬프트에 오른다. claude 는 그 폴더 이름을 예약해 건너뛴다.

## Engram 규칙과 어긋나는 곳

비교 대상 = TRD §1-2 「종류 판정」 의 스킬 행 · §1-3 5 충돌 · §1-5 스킬 번역 행. 지금 규칙의 요점:

- 바로 안에 `SKILL.md` 가 든 폴더 = 스킬(OS 대소문자 규칙 · Windows 는 `skill.md` 도).
- 참조 = 폴더 경로.
- 이름 = 앞머리 `name`, 없으면 폴더 이름.
- 같은 이름 다른 경로 = 충돌(빨강).

| # | Engram 규칙 | claude | codex | 생기는 일 | 권고 |
|---|---|---|---|---|---|
| **A** | `SKILL.md` 찾기 = OS 규칙(Windows 는 `skill.md` 도 — 사용자 2026-10-17) | 맞음 | **어긋남** — OS 무관 정확히 `SKILL.md` | Windows 라이브러리의 `skill.md` 스킬이 화면에 보이고 claude 는 싣는데 **codex 는 오류 한 줄 없이 빠뜨린다** | **인식을 정확히 `SKILL.md` 로 좁힌다** — 두 CLI 가 모든 OS 에서 싣는 유일한 이름이다. 2026-10-17 사용자 결정을 뒤집으므로 **사용자 확인 필요**. 대안 = 인식은 두고 codex 백엔드 에이전트에 붙으면 문제 줄(「codex 는 `SKILL.md` 만 읽는다」) |
| **B** | 스킬 안 스킬 = 따로 스킬(붙이면 그 폴더 하나) | 맞음 — 정션 하나 = 스킬 하나(안쪽은 안 훑음) | **어긋남** — 붙인 폴더 아래 6층까지 `SKILL.md` 를 전부 함께 싣는다(점 폴더 제외) | **붙이지 않은 안쪽 스킬이 codex 에만 실린다.** 이름이 겹쳐도 Engram 충돌 검사가 못 본다 | codex 백엔드의 「실제로 받는 것」(`library::resolve`)에 **안쪽 스킬까지 펼쳐 넣고** 충돌 검사도 그 목록으로 한다(실제로 실리니까). 안쪽 스킬을 따로 붙이면 같은 실제 경로라 codex 가 하나로 합친다 — Engram 의 「같은 경로 = 처음 한 번」 과 맞다 |
| **C** | 앞머리 형식을 안 본다(이름만 읽음) | 너그러움 — 단 플러그인 경로에서 설명을 안 쓴 스킬은 **모델 목록에 안 뜬다** | **엄격** — 앞머리 블록 · YAML · `description` · BOM 없음 · `name` ≤ 64 를 어기면 **빠진다**(오류 목록 · TUI 경고 한 줄) | 라이브러리에선 멀쩡한 스킬이 codex 에선 없고, claude 에선 모델이 모른다 | 훑기에서 `SKILL.md` 를 **codex 규칙으로 검사**해 그 스킬 줄을 「오류」 상태로(이유 + 줄). 막힘 ② 오류로 둘지(두 백엔드 공통 · 또는 codex 만)는 **사용자 결정** — claude 도 모델이 못 보니 공통이 일관된다 |
| **D** | 이름 = 앞머리 `name`, 없으면 폴더 이름 | 정체는 **Engram 이 만든 정션 폴더 이름**이 `[^A-Za-z0-9_-]`→`-` 로 바뀐 것. 앞머리 `name` 은 표시 이름 · 별칭 | 같음 — 단 ① 공백을 접고 빈 문자열은 없음으로 ② 폴더 이름은 **정션을 따라간 실제 폴더** ③ manifest 조상이 있으면 `<ns>:` 접두 | `Weird Name.v2 한글` 같은 이름은 claude 에서 `engram:Weird-Name-v2---` 로 바뀐다. `ns:x` 처럼 Windows 폴더 이름이 못 되는 이름(`:` `?` `*` `"` `<` `>` `|` `\` `/`)은 **정션 자체를 못 만든다**. 라이브러리가 남의 플러그인 폴더를 품으면 codex 이름만 `ns:이름` 이 된다 | 이름 = 앞머리 `name` 을 trim · 공백 접기, 비면 **실제 폴더 이름**(정션이면 대상). **이름 검사 추가**: 1–64자 · `[a-z0-9-]`(Anthropic 권장 꼴 — 소문자 · 숫자 · 하이픈) 밖이면 「오류」. 실제 폴더 조상에 플러그인 manifest 셋 중 하나가 있으면 「codex 에서는 `<ns>:<이름>`」 문제 줄(또는 그 이름으로 충돌 검사) |
| **E** | 충돌 = 같은 이름(정확히 같음) · 다른 경로 | 정션 이름이 Windows 에서 **대소문자 무시**로 부딪히고(`C-Case` · `c-case` 를 한 폴더에 못 둔다), 치환 뒤 같아지는 이름(`a b` · `a-b` · 한글 두 글자끼리)은 **같은 내부 이름**이 된다 | 이름 겹침을 허용(둘 다 실림 · 멘션만 모호). 멘션 개수는 소문자로도 센다 | 지금 키로는 `Foo` · `foo` 가 통과하는데 휘발 폴더를 못 깐다 | **충돌 키 = `lower(이름)` 을 `[^a-z0-9_-]`→`-` 로 바꾼 것.** D 의 이름 검사를 넣으면 키는 「소문자 이름」 으로 줄어든다 |

곁 사항 (규칙 변경 아님 · 알아 둘 것)

- **휘발 플러그인 모양은 확인됐다** — manifest 없이 `engram:<정션 이름>` · 정션 따라감 · Base directory = 정션 경로.
  - `skills\SKILL.md` 를 두지 말 것.
  - `claude plugin validate` 를 관문으로 쓰지 말 것 — manifest 가 없으면 실패하고 링크를 안 따라간다. 쓰려면 `.claude-plugin\plugin.json` `{"name":"engram"}` 한 줄이 필요하다.
- **같은 실제 스킬을 사용자 전역 폴더가 이미 가리키면 「붙임」 이 효과가 없다.**
  - claude 는 플러그인 쪽을 건너뛰고 맨 이름(개인 스킬)으로만 보인다.
  - codex 는 경로가 같아 하나로 합친다.
  - 이 PC 는 `~/.claude/skills` · `~/.agents/skills` 가 둘 다 공유 스킬 폴더로 이어져 있다. 라이브러리가 그 폴더를 품으면 그 스킬은 늘 켜져 있고, 떼도 안 빠진다.
- **codex 는 이름이 겹쳐도 막지 않으므로 Engram 충돌 검사는 codex 에 대해선 「Engram 이 깔 수 없다」 가 아니라 「모델이 둘을 본다」 의 문제다.** claude 는 Engram 휘발 폴더가 이름별이라 깔 수 없다(TRD §1-3 5 의 이유 그대로).

## 방법 — 재현 요점

스크립트는 세션 스크래치(`…\scratchpad\skill-recog\`)라 남지 않는다. 요점만 남긴다.

- **픽스처:** 라이브러리 흉내 폴더 하나에 위 표의 26 항목을 만들었다. 정션 대상은 그 폴더 밖에 두었다.
  - 파일 심볼릭 링크는 `mklink` 권한이 없어 만들지 못했다.
- **codex 소스:** 태그 `rust-v0.156.1` sparse 클론을 정독했다.
  - `ext/skills/src/loader/{mod,discovery,host,namespace,host_merge}.rs` · `ext/skills/src/host_roots.rs`
  - `skills/src/{parser,selection,name_counts,mentions,model}.rs`
  - `exec-server/src/local_file_system.rs`(walk) · `utils/plugins/src/plugin_namespace.rs` · `exec-server-protocol/src/protocol.rs`(manifest 경로 셋)
- **codex 실측:**
  - 스크래치 `CODEX_HOME`(이름 `hc`)에서 `codex app-server` 를 띄웠다.
  - `skills/list` 를 extraRoots 설정 전 · 후 한 번씩 불렀다. 응답의 `skills` · `errors` 가 판정 근거다.
  - 작업 폴더 `.agents/skills` 의 렌더는 `codex debug prompt-input` 으로 봤다 — 모델 입력을 오프라인으로 낸다.
  - 덫 프록시(`HTTP(S)_PROXY` → 기록 후 403)가 `chatgpt.com` · `github.com` 접속 시도 각 1회를 막았다. 모델 요청은 없었다.
- **claude 문서:** <https://code.claude.com/docs/en/skills>(「How a skill gets its command name」 · 이름 거부 문자 · YAML 깨짐 = 필드 없이 실림 · 겹침 표) · <https://code.claude.com/docs/en/plugins-reference>(manifest 선택 · 플러그인 이름 = `--plugin-dir` 폴더 이름 · `skills/` = `<name>/SKILL.md` 하나씩 · 이름 규칙).
- **claude 번들:** `claude.exe`(246 MB · bun 단일 실행 파일)에서 `"SKILL.md"` 주변 문자열을 떠 읽었다.
  - 개인 · 프로젝트 로더 = `readdir` 뒤 `join(dir, 이름, "SKILL.md")` 를 stat · readFile.
  - 플러그인 로더 = 같은 꼴, 내부 이름은 `${플러그인}:${폴더 이름.replace(/[^a-zA-Z0-9_-]/g,"-")}`.
  - 표시 이름 = 앞머리 `name`(플러그인 접두 붙임) · 별칭 = `:` 없는 앞머리 이름 · realpath 중복 건너뛰기.
  - 난독화된 함수 이름은 판본마다 바뀌므로 인용하지 않는다.
- **claude 실측:**
  - 실행 환경:
    - `CLAUDE_CONFIG_DIR` = 스크래치 · `ANTHROPIC_BASE_URL` = 로컬 가짜 Messages API(요청 **본문만** 기록 · 헤더 미기록 · 가짜 키) · `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`
    - 부모 세션 환경변수(`CLAUDECODE` · 메시징 소켓 등)는 뺐다.
    - `-p --output-format stream-json --verbose --no-session-persistence --debug-file` · `--plugin-dir` 둘
  - 1회차 = init 이벤트(`skills` · `slash_commands` · `plugins`) + 요청 본문의 스킬 목록 + 디버그 로그.
  - 2회차 = 가짜 서버가 첫 답으로 **Skill 도구 호출 10개**(이름 후보)를 돌려보냈다. 다음 요청에 실린 `tool_result`(성공/「Unknown skill」)와 주입된 「Base directory」 로 어느 이름이 풀리는지 쟀다.
  - `claude plugin validate --json` 4회(전부 오프라인).
  - **실 모델 호출 0.**
- **실 설정 불변 확인:**
  - `~/.codex` — 세션 파일 1589 · `history.jsonl` 150줄 · `session_index.jsonl` 513줄 · `config.toml` 해시 · `skills` 목록, 전 · 후 같음.
  - `~/.claude` — `settings.json` · `skills` · `plugins` 해시가 같다.
  - `~/.claude.json` · `~/.claude/projects` · `~/.codex/*.sqlite-wal` 은 다른 세션들이 그 사이에 썼다. 시험 이후 바뀐 파일 전부에서 스크래치 경로 문자열 0건이다.

## 한계

- **대소문자 구분 파일 시스템(Linux · macOS 구분 볼륨) 미측.**
  - 소스상 claude 는 그 OS 의 열기 규칙을 따르므로 거기선 `SKILL.md` 만 실릴 것이다(가능성 높음).
  - codex 는 OS 무관 정확 일치다(소스 확실).
- **파일 심볼릭 링크 `SKILL.md` 미측**(권한). codex 는 소스상 빠지고, claude 는 stat 이 링크를 따라가므로 실릴 것이다(불확실).
- **실 모델이 어느 스킬을 고르는지는 안 쟀다.** claude 의 이름 풀이는 실제 CLI 코드 경로를 탔지만 호출은 가짜 서버가 지어냈다. codex `$이름` 멘션의 모호 처리는 소스만 봤다.
- **claude 플러그인 경로에서 설명 없는 스킬이 모델 목록에서 빠지는 규칙**은 세 경우를 실측했을 뿐 코드 조건을 못 짚었다.
- **claude 개인 · 프로젝트 경로의 정션**, 스킬 폴더 안 `.claude-plugin`(`@skills-dir`), 하위 폴더 `.claude/skills` 동적 발견, `agents/openai.yaml` 효과 — 미측(문서 · 소스).
- **codex 대화형 TUI 미측** — `skills/list` 와 `debug prompt-input` 은 같은 로더를 쓴다(소스).
- 판본 한정: Claude Code 2.1.284 · codex-cli 0.156.1. claude 는 번들 문자열 판독이라 다음 판에서 쉽게 바뀔 수 있다.
