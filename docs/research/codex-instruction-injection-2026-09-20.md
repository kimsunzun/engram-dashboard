# The paths for loading instructions into codex — exhaustive measurement (2026-09-20)

- Target: **codex-cli 0.155.0** (npm `@openai/codex`, Windows)
- Purpose: is 「에이전트마다 다른 지시서를, 명령줄 한계에 안 걸리게, 가능하면 시스템 프롬프트 강도로」 achievable
- Consumers: **ADR-0215** (decision) · CLAUDE.md 「백엔드 확장」
- ★**This document is the canonical grounds for that ADR — do not copy the figures into the ADR.**★

## Method — a fake model server + sentinels

We started an HTTP server on `127.0.0.1` and set it in the codex config as a **model provider** (`[model_providers.<이름>] base_url=http://127.0.0.1:<포트>/v1`). codex takes it for the real API and sends **the full request it assembled itself**; we receive that body as a file and open it. The decision is made by the number of times a **sentinel string** planted per file appears — to see truncation, plant **different sentinels at the head and the tail** and check only that the tail survives.

- **No real model is called** — zero cost, zero external transmission.
- **`CODEX_HOME` is redirected to a scratch folder** — we confirmed every round that the session count in the real `~/.codex` was **unchanged at 963** before and after.
- ★**Do not use `codexhome` as the scratch folder name**★ — the codex Claude Code plugin companion uses that name as its own `CODEX_HOME` and overwrites `config.toml`. That collision actually sent one probe turn to the real `api.openai.com` (401 for lack of credentials, payload not sent).
- ★**If a temporary folder is given as `CODEX_HOME`, codex creates helper batch files in the real home (`~/.codex/tmp/arg0/`)**★ — a warning appears on every run. They are not session records or credentials.
- The auxiliary channel for dry checks = `codex debug prompt-input "<텍스트>"` (renders the assembled result without a network). We confirmed its structure matches the real wire body.

## Result — where the instructions arrive

| Means | Landing | Append/replace | Path input | Measured limit |
|---|---|---|---|---|
| **`model_instructions_file`** | **`instructions`** (system prompt) | **Full replace** | **O absolute path** | **1,049,629 chars lossless** |
| `instructions` (config key) | Same | Full replace | X string | Not measured |
| **`developer_instructions`** | `input[0].content[0]` · role `developer` · kind `generic.developer_instructions` | **Append** | X string | **502,530 chars** (via the config file) |
| `AGENTS.md` · `project_doc_fallback_filenames` | `input[1]` · role **`user`** · kind `agents_md.instructions` | Append | **O absolute path** | `project_doc_max_bytes` default **32,768B** |
| `$스킬이름` mention | role `user` · wrapped in `<skill><name><path>` | Append · separate per mention | Per file | **30,368B lossless** (no truncation marker) |
| `localImage` | `input_image` (base64 data URL) | — | O | Image only |

- **Grounds for deciding append** = with `developer_instructions` on, `instructions` stays **unchanged at 17,174 chars**. Grounds for deciding replace = with `model_instructions_file` on, codex's default sentence (`You are a coding agent running in the Codex CLI`) **disappears**.
- **`developer_instructions` comes before codex's own block** — before turning it on, `input[0]` was `<skills_instructions>` (3,700B) + `<permissions instructions>` (342B), and with it on our text is inserted **before** them.
- **Reassembly path confirmed** — we appended a sentinel to the captured default prompt (17,174 chars / 17,314 bytes), made it a file, and pointed `model_instructions_file` at it; `instructions` arrived as default + sentinel. So 「기본 프롬프트를 우리가 앞에 붙인다」 holds. **But it must be fetched again for every version, and how to do that is not measured** (static extraction from the binary · harvesting once at startup · planting a copy).
- **Keys that do not exist** (confirmed with `--strict-config`, each `unknown configuration field`): `developer_instructions_file` · `instructions_file` · `experimental_instructions_file` · `base_instructions` · `baseInstructions` · `include` · `import` · `extends` · `agents_md_path` · `project_doc_path`.
  - ★**`experimental_instructions_file` is missing not because the feature is absent but because it was renamed to `model_instructions_file`**★ — the official docs have a migration note.
  - ★**Without `--strict-config`, an unknown `-c` key is silently ignored rather than rejected**★ — always attach that flag to check that a key exists.
- The `projects."<경로>"` table has **only `trust_level`**, so it cannot carry instructions.

## The wall of terminal mode — all because of the `cmd` shell

We launch codex as `cmd.exe /c codex.cmd` (reason = the user's decision to launch `codex` by name; `codex` on PATH is a `.cmd`, so `CreateProcessW` cannot launch it directly).

1. **The command-line limit of 8,191 chars** (cmd). Removing the shell makes it 32,767 chars.
2. ★**A newline cuts the command line without an error**★ — it is cut at the first LF with no signal at all.
3. **A single quote breaks the `'…'` quoting.**
4. ★**`%NAME%` is expanded as an environment variable even inside quotes**★ — a known unfixed limit.

**Current measured headroom:** with the real instructions (1,331 chars) in, the whole command line is **1,686 of 8,191**. About 6.5k of headroom.

★**app-server mode has none of these walls**★ — the value goes in the JSON body and the working folder does not travel on the command line either.

## Resume · fork — consumed only once, at start

- ★**`developerInstructions` survives a resume**★ — even when we **killed the app-server process and started a new one** and resumed the thread, it remained in `input[0]` role `developer`, **identical down to the byte count (69)**. The position does not shift and no duplicate appears.
- ★**If a different value is sent with a resume or fork, it is ignored without an error or warning**★ — the original value is kept, and a fork inherits the parent's. **To change the instructions of a live agent, a new thread must be opened.**
- `ThreadResumeParams` and `ThreadForkParams` **both have a `developerInstructions` field** (schema measured) — it is not that it cannot be passed because the field is missing.
- **`AGENTS.md` and the project docs are also read only once, at session start** — even if the file is edited or deleted after turn 1, turn 2 keeps the content it first read (zero extra cost per turn). **But `resume` reads it again and attaches one more block.**

## Skill mention — the only means that needs no assembly

- If `$스킬이름` is written in the prompt body, codex reads that `SKILL.md` **whole, frontmatter included**, and inserts it. If several are called, they go in as **separate blocks**.
- ★**The `$` is required**★ — a bare skill name is not picked up. A name that does not exist is **silently dropped**.
- **The catalog (name + description) is always included separately, and the description is silently truncated at about 400 chars.**
- ★**Adding a root is possible only through the app-server RPC, and that is process-global**★ — threads attached to one app-server **overwrite each other's roots** (we confirmed thread 1 inheriting thread 2's root on the next turn). Terminal mode has no means to add a root at all.
- Documented search paths = `$CWD/.agents/skills` · parent · `$REPO_ROOT/.agents/skills` · `$HOME/.agents/skills` · `/etc/codex/skills`.
  - ★**It is not that codex reads the Claude skills folder automatically**★ — on this PC `~/.agents/skills` is a junction to `I:\Engram\core\claude-global-shared\skills`, so it **only read a documented path**.

## Prompt syntax — only `$스킬이름`

We sent seven forms directly and confirmed that **all passed through literally**: `@절대경로` · `@상대경로` · `#` · `/init` · a skill name without a $ prefix · a structured `mention` input item · a nonexistent `$이름`. The binary also has **only one collector** that scans the text — the **skill mention** — and a slash-command file convention such as `$CODEX_HOME/prompts` **does not exist**.

## Authority level — checked against the official docs

- `learn.chatgpt.com/docs` (the old `developers.openai.com/codex` has moved here).
- **The `instructions` parameter of the Responses API = 「`developer` 역할 텍스트 입력과 동등」** (the Returns section of the API reference). The guide docs also place the two side by side and describe them as nearly the same.
- **The model-defined authority ladder** = Root > System > Developer > User > Guideline > No Authority. ★**The System layer is 「OpenAI 만 넣을 수 있다」**★ — the ceiling reachable through the API is Developer.
- **At the same level, the later message beats the earlier one** — do not put contradictory text in two places at once. GPT-5 burns reasoning to reconcile the contradiction (a doc warning).
- **Role-token wrapping is identical for everything up to the user message**, only the role name differs — based on the published Harmony format. ★**Harmony is for `gpt-oss`, and the token format of hosted models is not public — this application is an inference.**★
- **Documentation status:** `model_instructions_file` · `developer_instructions` · `project_doc_fallback_filenames` · `project_doc_max_bytes` · `project_root_markers` · the `AGENTS.md` scheme · `SKILL.md` · the `$` mention = **in the docs**. ★**Putting an absolute path into `project_doc_fallback_filenames` is not in the docs**★ (the docs speak only of file names). `skills/extraRoots/set` is an experimental API that the docs themselves say **「예고 없이 바뀔 수 있다」**. **codex has no version policy for config keys.**

## Not measured

- How differently a real model follows `instructions` and `developerInstructions` (conflict test — it needs a real model call, so it was not run).
- Which of the three ways of fetching codex's default prompt per version works.
- The ceiling of `project_doc_max_bytes`.
- The upper limit of a skill body (confirmed only up to passing 30KB) · the total limit of several simultaneous mentions.
- Types other than `file` for `thread/attachment/add` · the models that support audio for `localAudio`.
- In what order the hosted model renders `instructions` · `tools` · `input` — ★**not observable from the client side**★ (the assembly is inside the server).
