# 바깥 스킬 폴더를 claude · codex 에 네이티브 스킬로 적재할 수 있나 (2026-10-10)

- **상태:** 조사 완료 · 결정 전(스킬 프리셋 구상 단계 — PRD 이전)
- **방법:** `/research` medium · 갈래 2(claude · codex — 각 주계열 수집자 1, 공식 문서 + 설치본 실측) · 메인 grounding(실측 기록 · 생성 스키마 대조) · cross-family 적대 리뷰 1회(codex, effort high · 레벨 2~3 — 판정 FIX 6건, 아래 반영)
- **대상판:** Claude Code 2.1.284 · codex-cli 0.156.1(소스 대조 = openai/codex main `d3f9b22`, 2026-10-09) · Windows 11
- **확신도 범례:** 확실 = 실측 + 문서/소스 교차 · 가능성 높음 = 한 출처 지지, 반증 없음 · 불확실 = 미검

## 결론

**claude 는 두 모드 모두, codex 는 JSON 모드만 프로세스(또는 서버) 단위로 바깥 폴더를 네이티브 스킬로 붙일 수 있다.** codex 터미널 모드에는 실행 인자 · `-c` · 환경변수로 스킬 폴더를 더하는 길이 없고, 남는 길은 파일 시스템 배치(전역 · 작업 폴더)나 로컬 플러그인 설치(전역 설정을 바꾼다)다.

| | 길 | 단위 | 확신도 |
|---|---|---|---|
| claude | `--plugin-dir <dir>` → `<dir>/skills/<이름>/SKILL.md` (이름이 `<폴더>:<스킬>` 로 붙는다 · manifest 불요) | 프로세스 | 확실 |
| claude | `--add-dir <dir>` → `<dir>/.claude/skills/<이름>/SKILL.md` (이름 접두 없음 · 그 폴더 읽기 권한도 준다) | 프로세스 | 확실 |
| codex JSON | app-server 메서드 `skills/extraRoots/set {extraRoots:[절대경로]}` | app-server 프로세스 전체(스레드별 아님) | 확실(스키마 · `skills/list` 실측) · 실제 턴 적재는 가능성 높음 |
| codex 터미널 | `~/.agents/skills` · `$CODEX_HOME/skills` · 작업 폴더~프로젝트 루트의 `.agents/skills` (디렉터리 정션 · 심볼릭 링크를 따라간다) | 전역 또는 작업 폴더 | 확실 |
| codex 터미널 | 로컬 플러그인: `codex plugin marketplace add <로컬경로>` → `codex plugin add <PLUGIN@MARKETPLACE>` | 전역 설치(설정 파일을 바꾼다) | 가능성 높음(명령 존재만 확인 · 적재 미측) |

**SKILL.md 형식은 양쪽이 같다**(frontmatter `name` · `description` + 본문 · 목록만 시스템 프롬프트에 오르고 본문은 쓸 때 읽는다). 이 PC 에선 `~/.agents/skills` 가 이미 `I:\Engram\core\claude-global-shared\skills` 로 정션돼 있어(2026-07-21) codex 전역에 공유 스킬이 이미 실려 있다(확실 — `dir` 실측).

## 발견

### claude

1. **`--plugin-dir` · `--add-dir` 로 바깥 스킬이 네이티브로 실린다** — 확실. 실측: 가짜 스킬 셋을 만들어 `claude -p … --output-format stream-json --plugin-dir plug --plugin-dir nomanifest --add-dir adddir` → init 이벤트 `skills` 에 `zebra-adddir` · `zprobe:zebra-plug` · `nomanifest:zebra-nomani` 가 실렸고, 스킬을 쓰라 하자 Skill 도구 호출로 본문의 비밀어를 돌려줬다. 출처: https://code.claude.com/docs/en/skills · https://code.claude.com/docs/en/cli-reference
   - **조건(리뷰 반영):** `--add-dir` 스킬 발견은 설정 출처 `project` 가 켜져 있어야 한다(기본 켬). `--safe-mode` · `strictPluginOnlyCustomization` 이면 안 실린다. `--bare` 는 자동 발견을 끄지만 `--add-dir` 스킬은 실린다(문서).
2. **스킬 폴더를 더하는 설정 키 · 환경변수는 없다** — 가능성 높음(문서 요약 + `--help` 부재 · 설정 레퍼런스 전수 grep 은 안 했다). `permissions.additionalDirectories` 는 파일 접근만 준다. 간접 길로 개인 스킬 폴더 안의 심볼릭 링크, `CLAUDE_CONFIG_DIR`(설정 홈 전체를 옮긴다 — 인증 · 세션까지 딸려 가 프리셋 스위치로는 부적합 · 미측)가 있다.
3. **작업 폴더 밖 파일 읽기는 기본 거부** — 확실(실측 `permission_denials:[Read …]`). `--add-dir` 또는 `--settings '{"permissions":{"additionalDirectories":[…]}}'` 로 풀린다(실측). **플러그인 스킬 본문은 읽기 권한 없이 실린다** — 단 SKILL.md 가 옆 파일 · 스크립트를 읽게 하면 그 폴더 권한이 따로 필요하다.
4. **이어받기(`--resume`)** — 스킬 목록은 프로세스마다 새로 만든다(확실 — 플러그인 A 로 만든 세션을 B 만 주고 이어받자 init 목록에 B 만 있었다). **그러나 시스템 프롬프트(`--append-system-prompt-file` 포함)는 첫 요청 때 떠 둔 사본을 이어받기 뒤에도 쓴다** — 확실(CLI 레퍼런스 원문 · 압축되거나 새 대화일 때 바뀐다 · `--system-prompt-snapshot off` 로 끈다). **리뷰 반영:** 목록이 바뀌어도 이미 호출된 스킬 본문은 대화 맥락에 남고 압축도 예산 안에서 넘긴다(skills 문서). CLAUDE.md 가 그 사본 안인지는 불확실.
5. **대체 주입 길** — `--append-system-prompt-file`(아무 절대경로 · 지금 쓰는 길) · CLAUDE.md `@경로` import(절대경로 허용 · 프로젝트 메모리에서 바깥을 가리키면 첫 회 승인 창 — 승인 안 했으면 조용히 안 실린다) · `.claude/rules/*.md`(`paths:` 로 범위 한정 가능) · `~/.claude/rules` 와 사용자 CLAUDE.md 는 승인 없이 바깥 import. 출처: https://code.claude.com/docs/en/memory
6. **미검(리뷰 반영):** 실측은 전부 `-p` 였다 — 대화형 모드의 작업 폴더 신뢰 창과 `--add-dir` 스킬의 상호작용은 재지 않았다(permissions 문서상 `-p` 는 신뢰 창을 건너뛴다).

### codex

7. **SKILL.md 스킬은 정식 기능이다(기능 플래그 불요)** — 확실. 발견 위치: `$CODEX_HOME/skills`(호환용 · deprecated) · `<OS 홈>/.agents/skills` · 시스템 폴더 · 프로젝트 `.codex/skills` · 프로젝트 루트(`.git`)~작업 폴더 각 층의 `.agents/skills` · 플러그인 스킬 · `extra_skill_roots`. 소스 `codex-rs/ext/skills/src/host_roots.rs:58-185`.
8. **스킬 폴더를 더하는 CLI 인자 · `-c` 키 · 환경변수는 없다** — 확실(소스). `[skills]` 설정(`bundled.enabled` · `include_instructions` · `max_context_tokens` · `config=[{path|name, enabled}]`)은 이미 발견된 스킬을 켜고 끌 뿐이다. **그 켜고 끄기 규칙은 사용자 층과 세션 플래그 층에서만 읽는다**(`config/src/skills_config.rs:158-180` — 프로젝트 설정의 `[skills.config]` 는 안 먹는다 · `-c` 는 먹는다). 터미널 모드에선 값에 TOML 따옴표가 필요해 우리 명령줄 문자 가드에 걸린다(추론).
9. **app-server `skills/extraRoots/set`** — 확실(0.156.1 생성 스키마 `v2/SkillsExtraRootsSetParams.json` 실재 · 실측: 설정 후 맨 작업 폴더에서 `skills/list` 가 바깥 스킬을 냈고 `skills/changed` 알림이 왔다). 실행 중 프로세스 전역이고 다음 턴의 개발자 메시지 목록에 반영된다(상류 시험 `app-server/tests/suite/v2/host_skills.rs` · 실제 모델 턴으로는 미측).
10. **app-server `thread/resume` 도 `developerInstructions` · `config` 를 받는다** — 확실(0.156.1 생성 스키마 `v2/ThreadResumeParams.json` 에 칸 실재). ★우리 코드(`backend/codex/protocol.rs` 의 `ThreadResumeParams` 주석)는 「그 칸의 존재를 재 보지 않아 비워 둔다 — 실측으로 확인되면 더한다」고 적혀 있다 — **그 실측이 이것이다.** 단 이미 올라가 도는 스레드면 이어받기의 덮어쓰기가 무시되고 불일치만 보고된다(`thread_processor.rs:217-229`). ★**정정(같은 날 실측): 새 프로세스로 이어받아도 새 값은 오류 없이 버려진다**★ — 칸이 스키마에 있다는 것과 먹는다는 것은 다르다(아래 「codex 프리셋 주입, 실제 턴」).
11. **`CODEX_HOME`** 은 설정 · 스킬 · 전역 AGENTS.md · 세션 · 인증을 통째로 옮긴다 — 에이전트별로 바꾸면 이어받기 조회와 인증이 깨진다(소스 확실 · 부작용은 가능성 높음). `~/.agents/skills` 는 OS 홈을 따라 같이 안 움직인다.
12. **AGENTS.md 대체 길** — 층마다 `AGENTS.override.md` > `AGENTS.md` > `project_doc_fallback_filenames`(경로 구분자 · 점이 든 이름은 거절 — 절대경로 불가) · 프로젝트 루트~작업 폴더 · 상한 32 KiB · import 문법 없음. 바깥 파일을 절대경로로 가리킬 길이 없다(확실 — `core/src/agents_md.rs`).
13. **샌드박스 `workspace-write` 는 디스크 전체 읽기를 준다**(쓰기만 제한) — 가능성 높음(소스 `protocol/src/permissions.rs:819-870` · Windows 실행 미측).
14. **이어받기** — 스킬 목록은 턴마다 현재 디스크 · 설정으로 다시 만든다(가능성 높음 · 소스). developer_instructions 는 이어받기 메타데이터에 영속되지 않고 이어받기 인자 · 설정에서 온다.
15. **로컬 플러그인 길(리뷰가 잡은 누락)** — `codex plugin marketplace add` 가 로컬 경로를 받고 `codex plugin add` 가 설치한다(설치본 `--help` 확인). 플러그인 스킬 폴더는 발견 위치에 든다(소스 `host_roots.rs:58-62`). 설치는 전역 설정을 바꾸므로 에이전트별 스위치가 아니다. 실행 단위로 `-c plugins."<id>".enabled=true` 를 쓸 수 있는지는 미검이고, 터미널 모드에선 큰따옴표가 가드에 걸린다(추론).

### 추가 실측 — claude 이어받기의 시스템 프롬프트 사본 (2026-10-10)

발견 4 의 문서 근거를 실측으로 굳혔다(haiku · `-p` · 같은 세션 id 로 이어받기 — `-p` 의 `--resume` 은 세션을 갈라 새로 만들지 않는다). **확실.**

| 실행 | 인자 | 답(비밀어) |
|---|---|---|
| 1 | 새 대화 + a.md(ALPHA-7) | — |
| 2 | 이어받기 + b.md(BRAVO-9) | **ALPHA-7** — 새 파일 무시, 첫 요청 사본 사용 |
| 3 | 이어받기 + b.md + `--system-prompt-snapshot off` | **BRAVO-9** — 새 파일 반영 |
| 4 | 새 대화 + b.md(대조) | BRAVO-9 |
| 5 | 이어받기, 파일 인자 없음 | ALPHA-7 — off 실행이 사본을 덮어쓰지 않았다(대화 기록에 두 답이 다 있어 단독으로는 약한 증거) |

`claude --help`(2.1.284) 원문: `--system-prompt-snapshot <on|off>` — 기본 on 이면 첫 요청의 프롬프트(`--append-system-prompt` 포함)를 기록해 그 뒤 모든 요청 · 이어받기에 그대로 보내고(다른 글을 줘도) 압축 때까지 유지, off 면 매 요청 새로 만든다(「프롬프트 글을 다듬는 용도」라고 적혀 있다). 미검: off 가 프롬프트 캐시 적중에 주는 영향.

### 추가 실측 — codex 프리셋 주입, 실제 턴 (2026-10-10)

codex-cli 0.156.1 · 모델 gpt-6-sol · effort low · 실제 턴 약 15회. 터미널 모드는 같은 `-c` 를 받는 `codex exec` 로 대신 쟀다(대화형 TUI 미측). **확실(이 판 · 대조 실행으로 확인).**

| 모드 | 부품 | 새 대화 | 이어받기 |
|---|---|---|---|
| JSON | 지시문(`developerInstructions`) | **됨** | **안 됨** — 새 값을 오류 · 경고 없이 버린다(대조: 지시문 없이 시작한 스레드를 새 값으로 이어받자 「NONE」 · 기록에 새 값 0회). 지시문 없이 이어받으면 첫 지시문이 기록에서 그대로 살아 있다 |
| JSON | 네이티브 스킬(`skills/extraRoots/set` → `thread/start`) | **됨** — 모델이 SKILL.md 를 읽고 ZULU-42 | **됨 — 새 프로세스에서 다시 set 해야** 한다. 스킬 목록은 턴마다 지금 프로세스 기준으로 다시 만들어 새 블록을 덧붙인다(set 안 하면 목록에서 빠지고, set 하면 돌아온다) |
| 터미널(`codex exec`) | 지시문(`-c developer_instructions=…`) | **됨** | **안 됨** — `codex exec resume` 의 `-c developer_instructions` 를 말없이 버린다(대조 「NONE」) |
| 터미널 | 네이티브 스킬 | **경로 없음** — `--help` · 설정 스키마 · 바이너리 문자열 모두 스킬 폴더를 더하는 키 · 인자 · 환경변수가 없다. `-c skills.config=[{path=…}]` 는 이미 발견된 스킬만 켜고 끈다(실측 NOSKILL). 작업 폴더 `.agents/skills` 정션만 됨(ZULU-42 · 작업 폴더를 건드린다) | 미측 |

- 미측 갈래: `thread/resume` 의 `config`(자유 형식 객체) · `baseInstructions` 가 이어받기에 먹는지. 대화형 TUI `codex resume`.

### 추가 실측 · 웹 — codex 이어받기에서 지시문을 바꾸는 길 (2026-10-10)

대조 설계: 지시문 없이 시작한 스레드 → 새 프로세스로 이어받으며 후보로 「preset word X」 전달 → 「없으면 NONE」. 각 1회 · 판정 근거는 답 + 롤아웃(`~/.codex/sessions`)에 X 가 적혔나.

| 후보 | 모드 | 결과 |
|---|---|---|
| `thread/resume.developerInstructions` · `config.developer_instructions` | JSON | **무시**(오류 · 경고 없음) |
| `codex exec resume -c developer_instructions=…` | 터미널 | **무시** |
| `collaborationMode.settings.developer_instructions`(default 모드 · `turn/start` 와 `thread/settings/update` 둘 다 · 실험 API) | JSON | **무시** — 설정엔 적히나 모델 입력에 안 들어간다 |
| 같은 것, plan 모드 | JSON | 먹음 — 단 에이전트가 plan 모드로 바뀌어 일반 작업에 못 쓴다 |
| `thread/resume.baseInstructions` | JSON | 먹음 — 단 codex 기본 지시문 **전체를 대체**하고 영속되지 않는다(매 이어받기 재전송 필요) · 프리셋 통로로 부적합 |
| **`turn/start.additionalContext` `{키:{value, kind:"application"}}`(실험 API)** | JSON | **먹음** — developer 역할 메시지로 기록에 한 번 적히고 이어받기 뒤에도 남는다. `kind:"untrusted"` 는 user 역할 인용문으로 들어간다 |
| **`thread/inject_items` developer 메시지(실험 API)** | JSON | **먹음** — 위와 같고 턴에 묶이지 않는다 |
| **작업 폴더 AGENTS.md 변경 · 신설** | JSON · 터미널 | **먹음** — 이어받을 때(스레드를 올릴 때) 다시 읽어 「이전 AGENTS.md 지시를 모두 대체한다」 메시지를 덧붙인다. 같은 프로세스 안 턴 사이에는 다시 읽지 않는다 |
| `project_doc_fallback_filenames=["ENGRAM.md"]`(이어받기 `config` · 터미널 `-c`) | JSON · 터미널 | 먹음 — **단 작업 폴더에 AGENTS.md 가 없을 때만**(있으면 AGENTS.md 만 실린다 · `AGENTS.override.md` 는 AGENTS.md 를 가린다) |

- **덧붙이기만 된다** — `additionalContext` · `inject_items` 는 옛 프리셋을 지우지 못한다. 바뀐 것은 「바뀌었다」고 명시해 덧붙여야 하고(그렇게 했을 때 새것을 따랐다 · n=1), 충돌 시 새것 우선은 보장이 아니다.
- **웹(원문 · `gh api` 로 이슈 상태 확인):** openai/codex #19045(open) — 이어받기 · 포크의 `developerInstructions` 무시는 **알려진 버그**(이미 올라간 스레드의 무시는 의도 — `thread_processor.rs` 의 mismatch 문자열). #43295 · #43309(open) — AGENTS.md 가 바뀌면 묶음 전체를 덧붙여 맥락이 넘칠 수 있다 · 「얼리기」 정책 요청. #16607(open) — 이어받은 스레드가 옛 스킬 목록을 유지한다는 보고이나, **우리 0.156.1 실측(`extraRoots` 재설정)에서는 새 스킬 블록이 덧붙었다.** 최신 릴리스는 rust-v0.162.1(설치본 0.156.1).
- **웹(t3code 소스 주석 · 미검):** codex 는 압축 때 클라이언트가 넣은 developer 메시지를 버린다 — t3code 는 압축 뒤 `thread/inject_items` 로 다시 넣는다. paseo · t3code 는 `turn/start` 마다 `collaborationMode` 지시문을 싣는다(위 실측상 default 모드에서는 안 먹었다 — 모델 카탈로그 덮어쓰기가 가릴 수 있다는 소스 주석이 있다).
- 미측: 대화형 TUI · 압축 뒤 `additionalContext` 생존 · 「바뀌었다」 문구 없는 충돌.

### 추가 실측 — codex JSON 프리셋 수명 전체 (2026-10-10)

codex-cli 0.156.1 · gpt-6-sol · effort low · 단계마다 새 app-server 프로세스 · 각 1회. 결론 = **이어받으며 프리셋(지시서 + 네이티브 스킬)을 바꿀 수 있다 — 단 처리 셋이 더 필요하다.** 가능성 높음(n=1).

| 단계 | 결과 |
|---|---|
| 시작: `extraRoots`=A + `developerInstructions`=A | 지시 A · 스킬 alpha 동작 |
| 이어받기: `extraRoots`=B + `additionalContext` B | 지시 B 적용 · 스킬 목록이 새로 덧붙어 alpha 빠지고 bravo 들어옴. **단 alpha 로 앞서 읽은 내용은 대화 기록에 남아 모델이 기억한다** |
| 이어받기, 아무것도 안 보냄 | B 유지(기록) |
| 같은 키 · 같은 값 재전송 | 프로세스 안에서는 한 번만 적힌다(키+값 중복 제거, 압축 뒤에도 유지). 새 프로세스의 첫 턴은 다시 적는다 |
| **압축(`thread/compact/start`)** | **developer 메시지가 전부 빠진다 — 프리셋 B 와 시작 때 `developerInstructions` A 까지.** B 는 암호화된 요약을 통해서만 답에 살아남았고 A 는 사라졌다. 같은 값 재전송은 중복 제거로 안 적히고, 값을 바꾸거나(`[rev 2]`) `thread/inject_items` 로 다시 넣으면 적힌다 |
| 「바뀌었다」 문구 없는 충돌(시작 CHERRY → 이어받기 DATE) | DATE 를 따랐다. 물으면 둘 다 본다고 답한다 |

- **캐시:** 바꾸지 않은 이어받기는 약 99% 캐시 적중. 프리셋 교체는 한 번에 약 6.6k 토큰을 캐시 없이 읽는다 — 대부분 스킬 목록 전체 재전송(약 21k 글자) 몫이고 프리셋 글 자체는 작다. 스킬 목록은 원격 플러그인 캐시가 바뀌기만 해도 다시 실려 같은 비용이 교체 없이도 생긴다. 다시 실릴 때마다 맥락이 그만큼 늘어난다(다음 압축까지).
- ★**기존 동작에 대한 곁가지 발견:**★ 지금 우리 codex JSON 프라이밍(ADR-0215 · `developerInstructions`)도 압축 뒤 빠진다는 뜻이다 — 이어받은 프로세스에서 쟀고, 스레드를 연 그 프로세스 안의 압축은 미측.
- 미측: 같은 프로세스 압축 뒤 `developerInstructions` 복원 여부 · 압축 요약과 충돌하는 재주입.

### 피어 — orca · herdr 는 지시서 · 스킬을 어떻게 싣나 (소스 정독 2026-10-10)

orca = upstream main `6cbbe456`(2026-10-10 · 로컬 클론 `df7460af` 는 낡아 새로 받았다) · herdr = 로컬 `v0.9.1`(`065ef9d`) + master `2563803d` 부분 대조. **둘 다 터미널 에이전트에 시스템 프롬프트 · developer 지시를 넣지 않는다 — 이 문제의 선례가 아니다.** 확실(grep 전수 · 핵심 줄 메인 대조).

- **orca 지시:** 작업 지시(「You are a dispatched worker」 머리말)를 **터미널에 사용자 차례로 친다**(`src/main/runtime/orchestration/preamble.ts:87` · 보내기 `coordinator-task-dispatch.ts:140-146`). 문구를 일부러 순하게 쓴 이유 = claude 가 붙여 넣은 글을 프롬프트 주입으로 의심하기 때문(STA-8200 주석). `--append-system-prompt` 류는 사용자 인자 파싱 표에만 나온다. 구조화(SDK · app-server) 경로는 별도이고 거기도 지시를 덧붙이지 않는다.
- **orca 스킬:** `orca skills install`(→ `npx skills add`)로 네이티브 전역 · 프로젝트 스킬 폴더에 설치 — 실행마다 붙이지 않는다. SKILL.md 는 얇은 껍데기이고 본문은 CLI(`orca skills get …`)가 낸다.
- **orca 이어받기 · 압축:** 이어받기는 원래 명령 + 이어받기 인자뿐(`src/shared/tui-agent-resume-startup.ts:78` `followupPrompt: null`) — 다시 넣는 것 없음. 압축은 화면 상태 표시에만 쓴다(PreCompact 는 일부러 등록 안 함 — `src/shared/claude-compact-completion.ts:3-6`).
- **herdr:** 지시 주입 0 · 상태 보고 훅만 설치 · 스킬은 사용자가 한 번 전역 설치(`npx skills add … -g`) 또는 `herdr --skill` 출력 · 이어받기는 맨 인자 · 압축은 상태 표시만.
- **함의(추론 · 미측):** codex 압축은 사용자 메시지를 남기고 developer 메시지를 버렸다(위 수명 실측의 `replacement_history`). orca 처럼 지시를 **사용자 차례**로 넣으면 codex 압축에서 살아남을 수 있다 — 대신 화면에 사용자 말풍선으로 보이고, claude 는 붙여 넣은 지시를 의심할 수 있다(orca 주석).

### 추가 실측 — 이어받기와 프롬프트 캐시 (2026-10-10)

haiku · `-p` · 덧붙인 md ≈ 16.7k 토큰 · 첫 요청 총 ≈ 31.7k · 각 경우 1회(반복 없음). 수치 = 요청별 `usage`(`cache_creation_input_tokens` / `cache_read_input_tokens`). **확실(이 판 · n=1).**

| 경우 | 이어받기 결과 |
|---|---|
| 기본(on) | 적중 — 31,695 읽기 · 새로 쓰기는 새 차례 몫 ≈ 80 |
| off · md 그대로 | 적중 — on 과 같은 수치(첫 요청 바이트가 on 과 같다) |
| off · 도중에 md 교체 | 바꾼 그 한 번만 md 부터 뒤를 다시 씀(읽기 15,046 · 쓰기 16,839) → 다음 이어받기는 다시 전부 적중 |
| off · 사이에 git 커밋 | 적중 — 환경 정보의 git 상태는 off 여도 첫 대화 값으로 남아 있었다(새 커밋이 안 보였다) |
| md 없음(대조) | 적중 |

- 결론: `--system-prompt-snapshot off` 를 늘 붙여도 md 가 그대로면 캐시 손해가 없고, md 를 바꾼 이어받기만 그 뒤를 한 번 다시 쓴다.
- 한계: 캐시가 실행 사이에 이어져 첫 요청 수치는 실행 순서의 영향을 받는다. on 상태에서 md 를 바꾼 경우는 재지 않았다. 캐시 구분점 위치는 사용량 수치로 추정했다.

### 추가 실측 — CLAUDE.md 는 이어받기에 다시 읽힌다 (2026-10-10)

같은 방식(haiku · `-p` · git init 한 빈 작업 폴더). **확실(이 판 기준).**

| 실행 | 그때 CLAUDE.md | 인자 | 답 |
|---|---|---|---|
| 1 | LIMA-3 | 새 대화 | — |
| 2 | MIKE-4 로 고침 | 이어받기 | **MIKE-4** |
| 3 | MIKE-4 | 새 대화(대조) | MIKE-4 |

- **실리는 자리가 다르다.** `--append-system-prompt-file` 은 세션 기록(`~/.claude/projects/<폴더>/<세션>.jsonl`)에 `attachment.type = "prompt_snapshot"` 으로 시스템 프롬프트 전문이 저장되고 이어받기는 그것을 다시 보낸다. CLAUDE.md 는 시스템 프롬프트가 아니라 `attachment.type = "instructions"` 첨부로 대화 줄기에 실리고, 이어받을 때 파일을 다시 읽어 **바뀐 파일만** 새 첨부(`changed: true` · `reason: "session_start"`)로 덧붙인다.
- **옛 사본은 지워지지 않는다** — LIMA-3 첨부가 대화 기록에 남아 둘 다 맥락에 있고, 모델은 나중 것을 골랐다. 지시끼리 충돌하면 최신 우선이 보장되는 것은 아니다(n=1).
- 미검: `--add-dir` 폴더의 CLAUDE.md(`CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`)도 같은 첨부 · 같은 이어받기 갱신을 타는지.

## 적대 리뷰 결과 (codex · FIX → 반영)

| 대상 | 결함 | 처리 |
|---|---|---|
| codex 터미널 「길은 파일 배치뿐」 | 로컬 플러그인 길 누락(high) | 발견 15 · 결론 표에 추가 |
| claude `--add-dir` | 조건 누락(`project` 출처 · safe-mode) | 발견 1 조건 |
| claude 이어받기 | 호출된 스킬 본문이 맥락에 남는다 | 발견 4 |
| claude 설정 키 없음 | 간접 길(심볼릭 링크 · `CLAUDE_CONFIG_DIR`) 누락 | 발견 2 |
| codex `skills.config` | 읽는 층 한정 누락 | 발견 8 |
| claude 신뢰 창 | `-p` 실측만으로 대화형을 말할 수 없다 | 발견 6 |

반박(근거 동반)으로 뒤집힌 load-bearing 주장은 없다.

## 공백

- codex `skills/extraRoots/set` 을 실제 모델 턴으로 재지 않았다(`skills/list` + 상류 시험).
- codex `-c skills.config` 끄기 필터 · 플러그인 실행 단위 켜기 · 파일 심볼릭 링크 SKILL.md 미측.
- codex Windows 샌드박스 바깥 읽기 실행 미측.
- claude 대화형 모드(신뢰 창) · `CLAUDE_CONFIG_DIR` 미측. claude 이어받기에서 CLAUDE.md 가 사본 안인지 불확실.
- codex 공식 스킬 문서(developers.openai.com/codex/skills)는 읽지 않았다 — 소스 기준.

실측 기록(이 저장소 밖 · 세션 스크래치라 남지 않는다): claude 가짜 스킬 실측 7회(haiku) · codex app-server JSON-RPC 실측 스크립트 · `codex app-server generate-json-schema` 산출.
