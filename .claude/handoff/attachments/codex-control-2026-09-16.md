# codex 제어 수단 실측 — 2026-09-16

세션 재개(`exec resume`)가 **실재한다**는 것을 확인한 기록. 직전까지의 「codex 는 이어 붙일 수 없다」는
결론을 **뒤집는다**. codex-cli **0.154.0** 기준.

## 결론

| 물음 | 답 |
|---|---|
| 호출 간 맥락이 이어지나 | **YES** — `codex exec resume <UUID>` |
| 토큰이 줄어드나 | **raw 는 오히려 는다.** 이득은 캐시 적중 + repo 재탐색 소멸 |
| 무인 반복 호출이 되나 | **YES** — TTY 불요, 3연속 성공 |
| 도는 도중 조향이 되나 | **거의 아니다** — 쏘면 끝날 때까지 관여 못 한다 |

## 실측 — resume

빈 스크래치패드에서 토큰 한 개를 심고 다음 호출에서 되물었다. `ZYGOTE-7` 을 그대로 답했다.

| 호출 | 형태 | input | cached | output | 벽시계 |
|---|---|---|---|---|---|
| 1 | fresh (심기) | 16,536※ | — | — | 5.0s |
| 2 | `resume --last` | 18,677※ | — | — | 21.7s |
| 3 | `resume <UUID> --json` | 55,440 | 34,688 | 23 | 4.6s |
| 4 | fresh `--json` (대조군) | 16,521 | 0 | 5 | 5.2s |

※ 사람용 출력의 "tokens used" 표시값. `--json` 의 `input_tokens` 와 다른 척도라 섞어 읽지 말 것.

**resume 은 raw input 을 줄이지 않는다** — 이력을 매 턴 전부 재전송하므로 오히려 는다(55,440 vs 16,521).
실이득 둘:

1. **이력의 63% 가 캐시 적중**(34,688 / 55,440).
2. ★**repo 를 다시 읽는 tool call 이 통째로 사라진다**★ — 호출당 17만의 정체가 그 재탐색이었다.
   빈 디렉터리 fresh 가 16.5k 인 것이 방증이다(17만 중 약 15.4만이 repo 읽기).

## 있는 제어 수단 (`--help` 실측)

**`codex exec` 서브커맨드:** `resume` · **`fork`**(세션을 갈라 새 세션으로) · `review`.

| 축 | 플래그 |
|---|---|
| 세션 | `exec resume <UUID>` · `exec fork` · `--ephemeral`(★resume 쓸 거면 금지★) |
| 회수 | `-o <FILE>`(최종 답변만) · `--json`(JSONL 턴별 usage) · `--output-schema <FILE>`(응답 JSON Schema 강제) |
| 모델 | `-m <MODEL>` · `-c key=value`(예 `model_reasoning_effort`) · `-p <PROFILE>` |
| 범위 | `-C <DIR>` 작업 루트 · `--add-dir` · `--worktree`(관리 워크트리) · `-s read-only\|workspace-write\|danger-full-access` |

**최상위:** `queue`(도는 세션에 메시지 투입 — 미실측) · `agents`(로컬 데몬 세션 목록) · `archive`/`unarchive`/`delete` · `doctor` · `update`.

## 물리는 함정

1. ★**`--last` 를 쓰지 말 것**★ — cwd 필터 + 최신순이라 같은 cwd 에 다른 세션이 하나만 생겨도 그쪽을 집는다.
   실측으로 **엉뚱한 세션을 물어 UNKNOWN 을 답했다**. 1회차에서 UUID 를 잡아 명시 resume 한다.
2. **UUID 회수 = `--json` 첫 줄** `{"type":"thread.started","thread_id":"<uuid>"}`. 사람용 출력에선 `session id:` 줄.
3. **`< /dev/null` 필수** — 없으면 stdin 을 읽어 `<stdin>` 블록을 프롬프트에 덧붙이고, 파이프가 열린 채면 행 위험.
4. **`--skip-git-repo-check`** — git repo 밖에서 필수(사전 §비주도 CLI 기본형에 이미 있다).
5. **cwd 가 세션에 기록된다** — repo 를 안 읽히려면 `-C` 로 지정하거나 중립 디렉터리에서 기동.
6. **이력이 턴마다 누적돼 input 이 계속 커진다** — 장수 세션은 언젠가 fresh 보다 비싸진다. **손익분기 미측정.**

## 플러그인 — 전제 둘이 사실과 달랐다

- **codex 플러그인은 설치돼 있지 않다.** `claude plugin list` = clangd-lsp · nmfc-origin-dev-harness ·
  nmfc-origin-wiki · nmfc-ue5-bridge. `known_marketplaces.json` 에 `openai-codex` 항목 자체가 없다.
- **플러그인 전역 활성은 이미 돼 있다.** `~/.claude/settings.json` 의 `enabledPlugins` 에 셋이 `true`.
  비활성은 clangd-lsp 하나뿐이다.
- **기억하던 것의 정체 = MCP 서버이고 그것은 죽어 있다.** `~/.claude.json` 에 `codex mcp-server` 설정이
  project 스코프로 남아 있으나 **0.154.0 에 `mcp-server` 서브커맨드가 없다**(`Error: stdin is not a terminal`
  로 죽는다). `codex mcp` 는 codex 가 외부 MCP 를 *소비*하는 반대 방향이다.
  → **살아 있는 재사용 경로는 `codex exec resume` 하나뿐.**

## ★검증 안 된 것★

- ★**캐시된 토큰이 ChatGPT 플랜 usage limit 에 full 로 잡히는지**★ — 과금상 할인돼도 rate limit 엔
  그대로 셀 수 있다. **이번 결론의 비용 이득 전체가 여기 걸려 있는데 확인 수단이 없었다.**
  (직전 세션의 2회 사망은 과금이 아니라 한도였다.)
- **17만 재현** — repo 에 codex 를 붙이지 말라는 제약 때문에 미측정. 16.5k 대조군에서 역산한 추정이다.
- **`fork` 의 실제 비용·동작** — 맥락 1회 적재 후 렌즈별 분기가 되는지. 존재만 확인했다.
- **한도 사망의 겉모습** — 종료코드·문구를 모른다. 이걸 모르면 재시도도 폴백도 못 짠다.
- `--output-schema` · `queue` · `archive` 계열 — 존재만 확인.
- 재부팅·세션 아카이빙 뒤에도 resume 이 살아남는지.

## 쓰려면 우리가 만들어야 하는 것

1. **thread_id 를 어디에 두나** — ★핵심 갭★. Claude Code 서브에이전트는 매번 새로 떠서 UUID 를 스스로
   기억하지 못한다. 파일에 적고 워커마다 넘겨야 한다.
2. **호출 껍데기** — 기동 → UUID 회수 → 명시 resume → `-o` 회수 → `--json` usage 파싱.
   현행 `/codex-cross-review` 의 `codex-review.py` 는 **매 호출 fresh** 라 이 축이 통째로 없다.
3. **예산·중단 정책** — 몇 턴/몇 토큰에서 끊고 새로 여나. 위 손익분기 미측정이 선결.

## 정본 관계

- 호출 형태의 정본 = `~/.claude/references/dictionary.md` 「주도/비주도 family」 표의 **비주도 CLI 기본형**.
  거기엔 아직 세션 축(resume·fork·`--json` usage)이 없다 — **사전 개정은 사용자 결정**이다.
- 이 파일은 실측 기록이고 정본이 아니다. 수치를 다른 문서로 베끼지 말 것.
