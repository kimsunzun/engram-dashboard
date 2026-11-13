# ADR-0341: Preset folder entry — claude reads it natively through --add-dir, codex gets only what it lacks, a missing folder blocks launch

- 상태: 확정 (2026-11-04, basis: user decisions 2026-11-04 · research `docs/research/preset-folder-entry-backend-loading-2026-11-04.md` (cross-family review FIX applied)) · 부분 폐기 by ADR-0363 (decision 6, an unreadable folder below a folder entry)
- 관련: Amends ADR-0312 (decision 3 scope) · Amends ADR-0337 (decision 3 notice count) · ADR-0316 (claude launch materials) · ADR-0328 (one merged `--settings`) · PRD `docs/process/S25-preset-library/prd.md` R7 · E4 · TRD `docs/process/S25-preset-library/trd.md` §1-5 · ADR-0355 (same-name codex skills across skill roots, folder entries included — launch with a notice) · Amended by ADR-0363 (decision 6, an unreadable folder below a folder entry)

## 맥락
The user asked for a preset option that names a folder (2026-11-01: 「폴더 경로 지정하면 거기있는 폴더를 해당 에이전트 룰로 파싱하는거하고, 고정경로 저장하는거. 결국 Claude 나 codex 가 폴더 지정하는것과 같은 옵션임」). Clarified 2026-11-04: the folder may be anywhere (a company-rules folder, a personal-rules folder); at launch it is attached automatically as `--add-dir`; its contents are read the way the backend reads a project folder at startup (「우리 클로드 맨처음에 읽을때 폴더 훓어서 읽잖아. 그것하고 같다고 생각하면 됨」). The earlier "no outside content" rule (ADR-0312 decision 3, user 2026-10-31 「외부에 두지말고 engram 안으로」) was about per-item links that break when an outside file moves, not about a folder entry (user 2026-11-04).

Research on Claude Code 2.1.284 and codex-cli 0.163.0: claude loads an `--add-dir` folder's skills, agents and commands natively, and its `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md` and unconditional `.claude/rules` when `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` is set. codex loads nothing from an extra folder except skills, and only in JSON mode (`selectedCapabilityRoots` per thread, or `skills/extraRoots/set`); codex `--add-dir` grants sandbox write only; terminal mode has no skill or `AGENTS.md` route.

## 결정
1. **A preset can hold folder entries.** Each entry is a folder path, inside or outside the library, kept in entry order. A folder entry is not a library asset: no reference tracking, no move or rename rewrite, no trash. Nothing is ever written into the folder.
2. **claude:** each folder entry becomes `--add-dir <folder>` in entry order, and the launch sets `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` when the preset has at least one folder entry. Claude loads what it natively loads from an added folder. We compose nothing extra for claude; the folder's settings (hooks, env, permissions) are not applied.
3. **codex JSON:** we read the folder the codex way and fill what codex lacks — instructions = the folder's `AGENTS.override.md`, else its `AGENTS.md`, added to the existing instruction composition (ADR-0337 decision 1 channel); skills = the folder handed to codex as a skill root (which of the two codex routes is a TRD choice). The folder's `.codex/config.toml` is not applied.
4. **codex terminal:** folder instructions and skills are not carried, the same as library instructions and skills (ADR-0337 decision 2), and are listed in the launch reply's `notices`.
5. **Notices are shown once per agent.** The same omission is not repeated on later launches or resumes of that agent; a changed omission set is shown again. Human callers (screen) and LLM callers (`engram` command reply) receive the same notice. The daemon log records the omission on every launch. This also applies to the library omissions of decision 4 / ADR-0337 decision 3.
6. **A missing or unreadable folder blocks launch** with "cannot find folder", like a missing attached item (PRD E4 ①). It unblocks when the path is fixed or the entry is removed from the preset.
7. **Folder scan preview:** our own parser reads the folder by each backend's rules to show what will load. For claude it is a prediction of claude's own loading.

## 거부한 대안
- **We parse the folder ourselves and translate everything into our launch materials for both backends** — would give an exact preview and allow folder settings, but re-implements claude's discovery rules and drifts when claude changes them. User rejected (2026-11-04 「Claude는 스스로 읽고 codex는 빠지는것만 우리가 채우자. 나중에 우리만의 룰? 있을때는 생각해봐야겠지만 딱히 그런 의도는 아니라서」).
- **Copy the folder into the library (ADR-0312 decision 3 as written)** — the user wants the folder used in place; the outside-content concern was broken per-item links (user 2026-11-04).
- **`permissions.additionalDirectories` in `--settings` instead of `--add-dir`** — grants file access only and loads nothing, even with the env var (doc verbatim + measured).
- **claude `--plugin-dir` for the folder** — needs a plugin layout (`<dir>/skills` …) and never loads a plugin-root `CLAUDE.md`; a rules folder is laid out as a project folder (plugins reference).
- **codex `--add-dir` / `runtimeWorkspaceRoots`** — sandbox write access only; loads no `AGENTS.md` and no skills (real turn + source).
- **Missing folder: warn and launch without it** — the agent would run without the company rules unnoticed. User rejected (「폴더 없으면 막아. 프리셋에서 빼야 실행되지.」).
- **codex terminal: carry the folder `AGENTS.md` through `-c developer_instructions`** — splits the rule from library instructions (ADR-0337 decision 2) and adds a command-line length block. (rejection ground: weak · rejected directly by the user — offered 2026-11-04, user took "omit and notify".)
- **Notify on every launch** — users who already know would see it every run. User rejected (「알면서 쓰는사람도 있을텐데 그 메세지 실행하면서 계속 보면 안되잖아」).

## 근거
- `docs/research/preset-folder-entry-backend-loading-2026-11-04.md` — claude: 12 haiku `-p` runs with marker files + docs (`code.claude.com/docs/en/permissions.md`: "Directories listed in `permissions.additionalDirectories` in a settings file grant file access only…"; `memory.md`: the env var row). codex: zero-model probes, 2 real turns, source at `rust-v0.163.0`. Peers: path lists in config (opencode, Gemini, Aider, paseo). Cross-family review (codex, effort high): FIX, 10 findings applied.
- User decisions 2026-11-04 (quoted above).

## 영향 / 불변식
- The preset format gains a folder entry kind; preset editing (screen and `preset.*` commands) must add, remove and reorder it (LLM-first control).
- The claude backend owns `--add-dir` and the env var; manager and transport stay backend-agnostic (ADR-0004). The env var is process-wide for that claude process: any other `--add-dir` it gets also loads its `CLAUDE.md`.
- Never write into a folder entry (same as ADR-0337 decision 4 for the working folder and `~/.codex`).
- Folder entries are outside library reference handling: library moves, renames, deletes and the relink flow (R19) do not touch them.
- Known gaps the TRD must handle or accept: claude never loads `paths:` rules, nested `CLAUDE.md`, `AGENTS.md` or settings hooks/env from an added folder; claude collision handling differs by version (2.1.284 drops the loser, ≥ 2.1.309 prefixes); codex `selectedCapabilityRoots` is experimental.
