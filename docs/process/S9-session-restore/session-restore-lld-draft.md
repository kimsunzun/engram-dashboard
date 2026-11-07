# Session save/restore + agent profile + auto-restart — design draft (for review)

Status: **draft.** Waiting for 3-party verification by fable / web Gemini / web GPT. Premise: `wezterm-failure-analysis.md`.
Scope: ① session save/restore + making spawn configurable (agent profile) + ② process auto-restart. (The messaging system is excluded)

## 0. Core decision (review point)

**AgentId (uuid v4) = unified with claude `--session-id`.** We generate the UUID and, at spawn, specify it as `claude --session-id <AgentId>` → we control the claude session ID. Restore is `claude --resume <AgentId>`. This avoids the root of wezterm's non-determinism/position mapping failures.

## 1. Data model

```rust
// 에이전트 프로필 — spawn 단위이자 persist 단위
struct AgentProfile {
    id: AgentId,            // = claude --session-id (uuid v4, 우리 생성)
    name: String,           // 표시명 (사용자 지정)
    command: AgentCommand,  // 무엇을 띄우나
    cwd: PathBuf,
    env: Vec<(String,String)>,
    created_at, last_active, // 메타
}

enum AgentCommand {
    Claude { resume: bool },          // claude (--session-id) or claude --resume
    Shell  { program: String, args: Vec<String> },  // cmd/pwsh/커스텀
}
```
- Configurable spawn (③) = "what" through `AgentCommand`. "Where" through cwd. (Replaces the current hardcoded claude)

## 2. Persist (save)

- **What:** only the `AgentProfile` list. (The claude conversation itself is saved by claude in `~/.claude/projects/` — we do not touch it)
- **Format:** a JSON file (`<app_data>/agents.json`). Simple · easy to inspect. SQLite is overkill (the scale is a few dozen agents).
- **When:** immediately on a profile change (spawn/kill/rename) + at app exit. (Periodic saving is unnecessary — profiles change at low frequency)
- **Layout separation:** the slot↔AgentId mapping is the **frontend (slotStore) responsibility**. The backend holds only profiles. (Avoids the wezterm position-coupling trap)

## 3. spawn (profile-based)

```
spawn_agent(profile):
  id = profile.id (없으면 uuid v4 생성)
  args = match profile.command {
    Claude{resume:false} => ["claude", "--session-id", id]
    Claude{resume:true}  => ["claude", "--resume", id]
    Shell{program,args}  => [program, ...args]
  }
  PTY spawn (cwd, env) → 기존 PtyManager.spawn_agent 흐름
```
- Extend the existing `spawn_agent(cwd)` to `spawn_agent(profile)`. The PtySession/drain/manager core is unchanged (interface only).

## 4. Restore (app start)

```
restore_all():
  profiles = load(agents.json)
  for p in profiles:
    p.command = Claude{resume:true}   // 복원 모드
    spawn_agent(p)
    // claude --resume <id> 실패 시 → fallback (아래)
```
- **fallback (defense — we fully distrust claude restore):** if `--resume` fails (session missing/damaged), automatically re-spawn with `--session-id <id>` (fresh). Our profile metadata is preserved, so the agent survives and only the conversation starts anew.
- **Failure detection signal:** must be settled by a spike — the exit code/stderr/whether it hangs for `--resume <없는id>` (uncertain ②).

## 5. Auto-restart (②)

```
정책: AgentProfile.restart_policy = { Never | OnCrash | Always }, max_retries
drain thread가 terminal 전이 감지 (Exited{code≠0} or 비정상) →
  manager가 정책 확인 → OnCrash & code≠0 & retries<max →
    claude --resume <id> 로 재spawn (대화 이어가기)
```
- A normal exit (code=0, user kill) is not restarted.
- A restart also uses `--resume`, so the conversation is kept. On failure, the fresh fallback.
- Preventing infinite restarts: max_retries + backoff.

## 6. Spike needed (the wezterm lesson — measurement first)

Windows measurement before the real implementation:
1. `claude --session-id <uuid>` spawn → works normally? When re-spawning the same uuid, append/error? (uncertain ①)
2. `claude --resume <없는/손상 uuid>` → exit code/stderr (settles the fallback trigger, uncertain ②)
3. `claude --resume <id>` on a PTY of a different size → does the TUI redraw normally? (uncertain ③)

## 7. Defensive principles (applying wezterm-failure-analysis §3)

1. Session = AgentId (independent of position) ✅ — not bound to a slot index
2. claude tracking = our spawned child PID/AgentId ✅ — no process-name (node) detection
3. Session ID = controlled by us through `--session-id` ✅ — removes non-determinism
4. Layout↔session separation ✅ — frontend slots / backend profiles
5. Graceful restore failure ✅ — fresh fallback, profile metadata preserved

## 8. Review questions (to the 3 parties)

- Is unifying AgentId=session-id valid? (What are the holes in the assumption 1 AgentId ↔ 1 claude session?)
- Is the JSON persist + save-on-change policy sufficient? (Concurrency / save corruption during a crash?)
- Does auto-restart `--resume` risk a restart loop / conversation contamination?
- On the fallback (resume failed → fresh), how does the user notice "the conversation was lost"? (UX signal)
- Besides the 3 spike items, what claude CLI traps did we miss?
