# Common coder brief — mid-turn input queue (read fully before coding)

You are a coder subagent. Implement ONLY your assigned chunk. Working directory: I:\Engram\apps\engram-dashboard-wt1 (git worktree, branch v0.3.2/feat/json-midturn-queue). Do not cd elsewhere.

## Read first
1. `.claude/handoff/attachments/20260926-midturn-impl/plan.md` — your chunk's row, §0 rules, §3 contact points, §6 orchestrator decisions (authoritative when they differ from the TRD).
2. The TRD lines your chunk cites: `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` (Korean; 3–5 KB single lines, ~255k chars — NEVER read it whole: use python/`sed -n` on the cited line numbers).
3. Comment rules (inject into your own work): `C:\Users\kimsunzun\.claude\skills\code-conventions\references\comments.md` + `.claude/skill-bindings/code-conventions.md`. Match the surrounding code's comment density and idiom; load-bearing code gets a `// ADR-0231` anchor line.
4. CLAUDE.md sections 「핵심 불변식」, 「코어 격리」, 「백엔드 확장」, 「빌드·검증 명령」 (only what your chunk touches).

## Rules
- Build must stand after your chunk: `cargo build`, the touched crates' tests, `cargo fmt --check`, `npx tsc --noEmit` / `npm test` if you touched `src/`.
- ★Run every build/test through `scripts/run-detached.ps1` (usage: `.claude/skill-bindings/qa.md` 「분리 실행」 lines 70–93) and judge by the `__EXIT` marker; read only the lines you need from the log. Crates that spawn real processes (agent, daemon, base) need `-- --test-threads=4`. Never bare `cargo test` at the root.
- Keep `OutputCore::new` and `AgentSession::new` signatures (add builders). `transport/{pty,stdio,input_queue}.rs`, `src/components/slot/TerminalSlot.tsx`, `src/lab/terminal/` must have diff 0 vs master.
- TDD: write the tests your chunk's plan row lists (TRD §7-1 line refs), with fixtures only — no real agents in unit tests.
- Do not change behaviour beyond your chunk's "no worse than today" statement.
- Forbidden: spawning subagents · committing · touching task lists · writing any skill feedback.md · editing docs/process/** or docs/decisions/** (report doc mismatches instead) · launching the app from the shell.

## Before returning — adversarial self-check
Re-read your own diff with an attacker's eye (lock order, lost/duplicated events, seq order, state never cleared, panics in handlers, missed exhaustive matches, generated bindings not regenerated). Report what you checked and anything suspicious — do not resolve doubts silently.

## Return (English, compact — no tool-output dumps)
- Summary of the change (≤10 lines) · files touched · new/changed public types & fns (exact signatures — the next chunk consumes them)
- Tests added (names) · commands run + `__EXIT` results + pass/fail counts
- Adversarial self-check: surfaces checked + findings
- Deviations from plan/TRD and why · stale comments/docs you noticed
