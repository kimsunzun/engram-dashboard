# codex 에 지시서를 싣는 경로 — 전수 실측 (2026-09-20)

- 대상: **codex-cli 0.155.0**(npm `@openai/codex`, Windows)
- 목적: 「에이전트마다 다른 지시서를, 명령줄 한계에 안 걸리게, 가능하면 시스템 프롬프트 강도로」가 가능한가
- 소비처: **ADR-0215**(결정) · CLAUDE.md 「백엔드 확장」
- ★**이 문서가 그 ADR 의 근거 정본이다 — 수치를 ADR 에 베끼지 않는다.**★

## 방법 — 가짜 모델 서버 + 센티넬

`127.0.0.1` 에 HTTP 서버를 띄우고 codex 설정에 **모델 제공자**로 박았다(`[model_providers.<이름>] base_url=http://127.0.0.1:<포트>/v1`). codex 는 그것을 실 API 로 알고 **자기가 조립한 요청 전문**을 보내고, 그 본문을 파일로 받아 열어 본다. 판정은 파일마다 심은 **센티넬 문자열**의 등장 횟수로 한다 — 잘림을 보려면 **머리와 꼬리에 서로 다른 센티넬**을 박고 꼬리 생존만 본다.

- **실 모델을 부르지 않는다** — 비용 0, 외부 전송 0.
- **`CODEX_HOME` 을 스크래치 폴더로 돌린다** — 실 `~/.codex` 의 세션 수가 전·후 **963개로 불변**임을 매 라운드 확인했다.
- ★**스크래치 폴더 이름으로 `codexhome` 을 쓰지 말 것**★ — codex Claude Code 플러그인 companion 이 그 이름을 자기 `CODEX_HOME` 으로 쓰며 `config.toml` 을 덮어쓴다. 실제로 그 충돌로 프로브 한 턴이 실 `api.openai.com` 으로 나갔다(자격증명 부재로 401, 페이로드 미전송).
- ★**임시 폴더를 `CODEX_HOME` 으로 주면 codex 가 헬퍼 배치 파일을 실 홈(`~/.codex/tmp/arg0/`)에 만든다**★ — 매 실행 경고가 뜬다. 세션 기록·자격증명은 아니다.
- 마른 확인용 보조 채널 = `codex debug prompt-input "<텍스트>"`(네트워크 없이 조립 결과를 렌더). 실 와이어 본문과 구조가 일치함을 확인했다.

## 결과 — 지시서가 도달하는 칸

| 수단 | 착지 | 덧붙임/대체 | 경로 입력 | 실측 상한 |
|---|---|---|---|---|
| **`model_instructions_file`** | **`instructions`**(시스템 프롬프트) | **전체 대체** | **O 절대경로** | **1,049,629자 무손실** |
| `instructions`(설정 키) | 같음 | 전체 대체 | X 문자열 | 미측정 |
| **`developer_instructions`** | `input[0].content[0]` · 역할 `developer` · 종류 `generic.developer_instructions` | **덧붙임** | X 문자열 | **502,530자**(설정 파일 경유) |
| `AGENTS.md`·`project_doc_fallback_filenames` | `input[1]` · 역할 **`user`** · 종류 `agents_md.instructions` | 덧붙임 | **O 절대경로** | `project_doc_max_bytes` 기본 **32,768B** |
| `$스킬이름` 멘션 | 역할 `user` · `<skill><name><path>` 감쌈 | 덧붙임·멘션마다 별개 | 파일마다 | **30,368B 무손실**(잘림 표식 없음) |
| `localImage` | `input_image`(base64 data URL) | — | O | 이미지 전용 |

- **덧붙임 판정 근거** = `developer_instructions` 를 켜도 `instructions` 가 **17,174자 불변**. 대체 판정 근거 = `model_instructions_file` 을 켜면 codex 기본 문구(`You are a coding agent running in the Codex CLI`)가 **사라진다**.
- **`developer_instructions` 는 codex 자기 블록보다 앞**이다 — 켜기 전 `input[0]` 은 `<skills_instructions>`(3,700B) + `<permissions instructions>`(342B) 였고, 켜면 우리 텍스트가 그 **앞**에 끼어든다.
- **재조립 경로 확인** — 캡처한 기본 프롬프트(17,174자 / 17,314바이트) 뒤에 센티넬을 붙여 파일로 만들고 `model_instructions_file` 로 가리키니 `instructions` 가 기본+센티넬로 도착했다. 즉 「기본 프롬프트를 우리가 앞에 붙인다」가 성립한다. **다만 버전마다 다시 떠 와야 하고 그 방법은 미측정이다**(바이너리 정적 추출 · 기동 시 1회 수확 · 사본 박기).
- **존재하지 않는 키**(`--strict-config` 로 확인, 각각 `unknown configuration field`): `developer_instructions_file` · `instructions_file` · `experimental_instructions_file` · `base_instructions` · `baseInstructions` · `include` · `import` · `extends` · `agents_md_path` · `project_doc_path`.
  - ★**`experimental_instructions_file` 이 없는 것은 기능 부재가 아니라 `model_instructions_file` 로 개명됐기 때문이다**★ — 공식 문서에 마이그레이션 안내가 있다.
  - ★**`--strict-config` 없이는 모르는 `-c` 키가 거절이 아니라 조용히 무시된다**★ — 키 존재 확인은 반드시 그 플래그를 붙인다.
- `projects."<경로>"` 테이블은 **`trust_level` 하나뿐**이라 지시서를 못 싣는다.

## 터미널 모드의 벽 — 전부 `cmd` 껍데기 탓

우리는 codex 를 `cmd.exe /c codex.cmd` 로 띄운다(사유 = `codex` 를 이름으로 띄우는 사용자 결정, PATH 의 `codex` 가 `.cmd` 라 `CreateProcessW` 가 직접 못 띄운다).

1. **명령줄 8,191자 한계**(cmd). 껍데기를 걷으면 32,767자가 된다.
2. ★**줄바꿈이 오류 없이 명령줄을 자른다**★ — 첫 LF 에서 잘리고 아무 신호도 없다.
3. **작은따옴표가 `'…'` 인용을 깬다.**
4. ★**`%NAME%` 이 따옴표 안에서도 환경변수로 펴진다**★ — 알려진 미수정 한계.

**현재 실측 여유:** 실 지시서 1,331자를 넣은 상태에서 명령줄 전체가 **8,191 중 1,686**. 약 6.5k 여유.

★**app-server 모드엔 이 벽이 하나도 없다**★ — 값이 JSON 본문으로 가고 작업 폴더도 명령줄을 안 탄다.

## 재개·fork — 시작할 때 한 번만 먹는다

- ★**`developerInstructions` 는 재개 후에도 살아남는다**★ — app-server 프로세스를 **죽이고 새로 띄워** 스레드를 재개해도 `input[0]` 역할 `developer` 에 **바이트 수(69)까지 동일**하게 남았다. 위치도 안 밀리고 중복도 안 생긴다.
- ★**재개·fork 에 다른 값을 실어 보내면 오류도 경고도 없이 무시된다**★ — 원래 값이 유지되고, fork 는 부모 것을 물려받는다. **살아 있는 에이전트의 지시서를 바꾸려면 새 스레드를 열어야 한다.**
- `ThreadResumeParams`·`ThreadForkParams` **둘 다 `developerInstructions` 필드를 갖는다**(스키마 실측) — 없어서 못 넣는 것이 아니다.
- **`AGENTS.md`·프로젝트 문서도 세션 시작에 한 번만 읽는다** — 턴 1 뒤 파일을 고쳐도 지워도 턴 2 는 처음 읽은 내용을 유지한다(턴당 추가 비용 0). **단 `resume` 은 다시 읽어 블록을 하나 더 붙인다.**

## 스킬 멘션 — 유일하게 조립이 필요 없는 수단

- `$스킬이름` 을 프롬프트 본문에 쓰면 codex 가 그 `SKILL.md` 를 **frontmatter 까지 통째로** 읽어 넣는다. 여러 개 부르면 **각각 별도 블록**으로 들어간다.
- ★**`$` 가 필수다**★ — 스킬 이름만 쓰면 안 잡힌다. 없는 이름은 **조용히 버려진다**.
- **카탈로그(이름+설명)는 따로 늘 들어가며 설명이 약 400자에서 조용히 잘린다.**
- ★**루트 추가는 app-server RPC 뿐이고 그것은 프로세스 전역이다**★ — 한 app-server 에 붙은 스레드들이 **서로의 루트를 덮어쓴다**(스레드1 이 다음 턴에 스레드2 의 루트를 물려받는 것을 확인). 터미널 모드엔 루트 추가 수단이 아예 없다.
- 문서화된 탐색 경로 = `$CWD/.agents/skills` · 상위 · `$REPO_ROOT/.agents/skills` · `$HOME/.agents/skills` · `/etc/codex/skills`.
  - ★**codex 가 Claude 스킬 폴더를 자동으로 읽는 것이 아니다**★ — 이 PC 는 `~/.agents/skills` 가 `I:\Engram\core\claude-global-shared\skills` 로 가는 junction 이라 **문서화된 경로를 읽었을 뿐**이다.

## 프롬프트 문법 — `$스킬이름` 하나뿐

일곱 형태를 직접 보내 확인했고 **전부 글자 그대로 지나갔다**: `@절대경로` · `@상대경로` · `#` · `/init` · 사글자 없는 스킬 이름 · 구조화된 `mention` 입력 아이템 · 없는 `$이름`. 바이너리에도 텍스트를 훑는 수집기가 **스킬 멘션 하나뿐**이고, `$CODEX_HOME/prompts` 같은 슬래시 커맨드 파일 규약은 **존재하지 않는다**.

## 권한 등급 — 공식 문서 대조

- `learn.chatgpt.com/docs`(구 `developers.openai.com/codex` 가 이쪽으로 옮겨졌다).
- **Responses API 의 `instructions` 파라미터 = 「`developer` 역할 텍스트 입력과 동등」**(API 레퍼런스 Returns 절). 안내 문서도 둘을 나란히 놓고 "거의 같다"고 쓴다.
- **모델 규정 권한 사다리** = Root > System > Developer > User > Guideline > No Authority. ★**System 층은 「OpenAI 만 넣을 수 있다」**★ — API 로 닿는 천장은 Developer 다.
- **같은 등급에선 나중 메시지가 앞을 이긴다** — 두 칸에 모순되는 글을 동시에 넣지 말 것. GPT-5 는 모순을 화해시키느라 추론을 태운다(문서 경고).
- **역할 토큰 감싸기는 user 메시지까지 전부 동일**하고 역할 이름만 다르다 — 공개된 Harmony 형식 기준. ★**Harmony 는 `gpt-oss` 용이고 호스팅 모델의 토큰 형식은 비공개다 — 이 적용은 추론이다.**★
- **문서화 상태:** `model_instructions_file`·`developer_instructions`·`project_doc_fallback_filenames`·`project_doc_max_bytes`·`project_root_markers`·`AGENTS.md` 체계·`SKILL.md`·`$` 멘션 = **문서에 있다**. ★**`project_doc_fallback_filenames` 에 절대경로를 넣는 사용법은 문서에 없다**★(문서는 "파일명"만 말한다). `skills/extraRoots/set` 은 문서가 스스로 **「예고 없이 바뀔 수 있다」**고 적은 실험 API. **codex 는 설정 키에 대한 버전 정책이 없다.**

## 미측정

- `instructions`·`developerInstructions` 를 실제 모델이 얼마나 다르게 따르는지(충돌 테스트 — 실 모델 호출이 필요해 안 돌렸다).
- codex 기본 프롬프트를 버전마다 떠 오는 방법 셋 중 무엇이 되는지.
- `project_doc_max_bytes` 의 천장.
- 스킬 본문의 상한(30KB 통과까지만 확인) · 동시 다중 멘션의 총량 상한.
- `thread/attachment/add` 의 `file` 외 타입 · `localAudio` 의 오디오 지원 모델.
- 호스팅 모델이 `instructions`·`tools`·`input` 을 어느 순서로 렌더하는지 — ★**클라이언트 쪽에서 관측 불가**★(조립이 서버 안쪽).
