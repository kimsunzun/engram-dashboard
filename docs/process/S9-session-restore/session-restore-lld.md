# Session save/restore + profile + auto-restart — integrated design (3-party verification applied)

Status: **revised draft.** fable/Gemini/GPT 3-party verification integrated. Premise: `wezterm-failure-analysis.md`.
Overall verdict: **Spike GO immediately / full backend core implementation after the spike** (Gemini NO-GO, GPT "hold the trust feature", fable "spike #4 decides whether the design holds").
Change history against the draft: `session-restore-lld-draft.md` (original) → this document. Full reviews: `s9-review-fable.md`, `session-restore-review-gemini.md`, `-gpt.md`.

## 0. Core decision — not unification but "separated control" (3 parties unanimous)

The original's **identification `AgentId = claude --session-id` is discarded.** The two have different lifetimes (AgentId is permanent / session_id can be damaged, forked, fail to resume, or be replaced). **Separate them:**

- `AgentId` (uuid v4) — our **immutable** key. The only reference for slots/profiles/tracking. Never changes.
- `claude_session_id` (uuid v4, **mutable, Optional**) — the current claude session. The initial value is newly generated, updated on fallback/replacement, with history kept.

## 1. Data model

```rust
struct AgentProfile {
    id: AgentId,                          // 불변 키
    name: String,
    command: AgentCommand,
    cwd: PathBuf,                          // spawn 전 canonicalize (spike #8)
    env: Vec<(String,String)>,
    claude_session_id: Option<Uuid>,      // 가변 — 현재 세션
    old_session_ids: Vec<Uuid>,           // fallback/교체 이력 (복구·표시용)
    restart_policy: RestartPolicy,        // Never | OnCrash | Always
    auto_restore: bool,                   // 앱 시작 시 자동 복원 여부 (spike #9)
    last_restore: Option<RestoreResult>,  // { when, outcome }
    created_at, last_active,              // last_active는 spawn/kill만 갱신(디바운스)
}
enum AgentCommand {
    Claude { extra_args: Vec<String> },   // 모델/권한 등 (resume 여부는 spawn 인자로 결정, 프로필에 안 박음)
    Shell  { program: String, args: Vec<String> },
}
// 파일 최상단: schema_version: u32  (미래 마이그레이션)
```

## 2. Persist (3 parties — atomic is mandatory)

- **atomic write:** write `agents.json.tmp` → flush → `rename` (replace). Prevents the whole profile from evaporating during a crash. **(Gemini Split-Brain / fable required)**
- **Preserve on load failure:** when corrupt, move it to `agents.json.corrupt-<ts>`, start with an empty list + notify the user. No overwriting on top of the corruption.
- **Serialize saves:** many callers (commands spawn/kill/rename + drain restart) → a single Mutex, a full snapshot inside the lock.
- **last_active debounce:** do not include it in every save-on-change (it would become high-frequency). Only spawn/kill save immediately.
- Format JSON (enough for a few dozen; the 3 parties agree to reject SQLite).

## 3. spawn (profile-based)

```
spawn_agent(profile, mode):
  guard: profile.id가 sessions 맵에 이미 있으면 거부 (이중 spawn 가드)  ← 1-e
  sid = profile.claude_session_id (없으면 uuid v4 생성 후 persist)
  args = match (profile.command, mode) {
    (Claude{extra}, Fresh)  => ["claude","--session-id", sid, ...extra]
    (Claude{extra}, Resume) => ["claude","--resume", sid, ...extra]   // spike #5: +--session-id 조합 가능하면 그걸로
    (Shell{p,a}, _)         => [p, ...a]      // Shell은 mode 무시 (원안 1-d 버그 수정)
  }
  PTY spawn (canonicalize(cwd), env) → 기존 PtyManager 흐름
```

## 4. Restore (app start) — fallback is "separation", not "loss"

```
restore_all():  // auto_restore=true 프로필만, stagger(순차+간격) — spike #9
  for p in profiles where p.auto_restore:
    if let Claude = p.command:  spawn(p, Resume)   // Shell은 Fresh
    else: spawn(p, Fresh)
```
- **fallback (when resume fails):** re-spawn Fresh with a **new uuid v4** (reusing the same id fails again with already-in-use — 1-b). The old sid → `old_session_ids`. **The claude transcript is preserved in `~/.claude/projects/` → not loss but separation.**
- **Failure detection:** settle the signal with spike #2/#6/#7 — distinguish a termination with exit/stderr from a hang on the trust·login prompt. (Never mistake a hang for success)
- **UI signal (no silent, 3 parties unanimous):** emit `agent-restore-result { id, outcome: Resumed | FreshFallback{reason} }` → a tree badge (⚠) + a terminal banner ("이전 대화 복원 실패 — 새 세션. 이전 대화는 보존됨"). **No modals** (N spam).

## 5. Auto-restart — activate after the spike (3 parties: do not put it in early as a trust feature)

- **Status decision:** not an exit code heuristic — decide by the backend `transition()` status (shutdown = user kill → do not restart). Only Exited{code≠0}/abnormal.
- **Retry ladder:** resume K times (2) → fresh with a new id once → stop + notify. (Prevents the Poison Pill loop — Gemini/fable)
- **retry reset:** reset the counter after healthy uptime of N minutes (10) (prevents cumulative exhaustion).
- **Auth-expiry zombie:** spike #7 — must detect a restart that stalls at a login prompt.

## 6. ★State machine conflict — both LLDs need revision (fable's biggest omission)★

Restart = "a new PtySession under the same AgentId". In the confirmed backend LLD, Exited/Killed are **terminal**, so this conflicts:
- **Subscriber succession:** **migrate** the old PtySession's `subscribers` (Arc sink) to the new session (the frontend Channel does not drop, best UX). Or the frontend watches the status and resubscribes. **→ A decision is needed, revise backend LLD §9.**
- **replay reset:** this is a new session start, so reset the replay buffer (marking that the earlier output has ended).
- **Frontend state:** add a `Restarting` state or allow Exited→Running re-entry — **revise frontend-integration-lld**.
→ If this section is undefined, restart conflicts with two existing confirmed designs. **Revise both LLDs before the core implementation.**

## 7. Spike (3-party integration — measure before the core, fork decides whether it holds)

| # | Item | Source | Priority |
|---|------|------|------|
| 1 | `--session-id <uuid>` spawn normal / append vs error on reuse | original | High |
| 2 | `--resume <없는/손상 uuid>` exit·stderr (fallback signal) | original | High |
| 3 | `--resume` redraw at a different PTY size | original/GPT | Medium |
| **4** | **Two resume round trips — session id kept vs fork** | **fable** | **Top priority (decides whether it holds)** |
| 5 | Whether the `--resume <old> --session-id <new>` combination is possible | fable | High (a root fix if it works) |
| 6 | Restore hangs on the trust prompt of a new cwd | fable | High |
| 7 | N simultaneous login expirations — waiting zombies | fable/Gemini | Medium |
| 8 | Effect of cwd normalization (`I:\` vs `i:\` / trailing) on resume | fable | Medium |
| 9 | Stagger for N simultaneous spawns / contamination from 2 concurrent PTYs with the same session-id | fable/Gemini | Medium |

Also observe the reaction to server-side session expiry (Gemini) as part of the #2/#7 family.

### 7-R. Spike measurement results (completed 2026-06-11 — `spike-results.md`)
| # | Result |
|---|------|
| 1 | `--session-id <uuid>` normal, our uuid returned in json ✓. Reuse is exit 1 `already in use` |
| 2 | `--resume <없는uuid>` exit 1 `No conversation found` ✓ — the fallback signal is clear (not a hang) |
| **4** | **`--resume` does not fork** — session id kept + append ✓ (**the retreat line §8 is not needed**) |
| /clear | **Creates a new sessionId**, updated in real time in `sessions/<pid>.json` ✓ |
| Key | **`~/.claude/sessions/<PID>.json` = {pid, sessionId, status, …}** — the current sessionId can be looked up deterministically by PID |
| trust | Did not occur in this environment (trusted). Possible on a new machine — mind it in operation |
| 3·redraw | Minor (we manage cols/rows) |

## 8. Retreat line — **not needed (spike #4 confirmed it does not fork)**

Since it was measured that `--resume` does not fork, the "fresh-new-id retreat" scenario itself never occurs. (But the route of guessing the sid by finding the newest file in the transcript directory is still forbidden — sessions/<pid>.json is the deterministic replacement.)

## 9. Order of progress

1. ✅ Spike measurement done (§7-R)
2. 🔄 Fix the §6 state machine revision → revise the backend and frontend LLDs
3. ⏳ Core implementation (profile/persist/restore/restart)

## 10. Defensive principles (wezterm lesson — kept)

Position-independent AgentId / direct spawn tracking / separated control of the session ID / layout↔session separation / graceful restore failure (new-id fallback + explicit preservation).

## 11. The sid acquisition mechanism (confirmed by the spike — final)

**Core: the first one is specified by us (no capture needed); only on a change do we deterministically re-acquire it from sessions/<pid>.json.**

### 11-1. The first sid — at spawn
- **We generate both** the `AgentId` (immutable key) + the `claude_session_id` (initial uuid)
- Spawn with `claude --session-id <claude_session_id>` → **starts with the value we specified.** The wezterm-style "capture the claude-generated sid afterwards" is unnecessary (removes the non-determinism at its source)
- Obtain the child PID (held by PtyManager — a volatile handle, not saved)

### 11-2. sid tracking — while running (change detection)
- Watch the single file `~/.claude/sessions/<현재 child_pid>.json` (create/modify)
- If `sessionId` differs from the `claude_session_id` we know (= `/clear`·fork) → update `claude_session_id`, and keep the old value in the `old_session_ids` history
- **PID instability is irrelevant** — we do not save it, and PtyManager always holds the currently alive child PID → look it up each time

### 11-3. Restore — restart/app start
- `claude --resume <id>` with the saved `claude_session_id` (latest)
- Failure (exit 1 `No conversation found`) → **fresh fallback with a new uuid** + the old id history + `agent-restore-result` UI notification ("새 세션, 이전 대화 보존됨")
- After restore, resume 11-2 with the new child PID

### 11-4. Defense (best-effort)
- `sessions/<pid>.json` is an **unofficial internal file** (a version field exists → the format may vary) → even if it is absent or changed, **at least the "first specified value" is kept** → try restore → fallback on failure. **Either way, no damage.**
- PID reuse → verify it is our process with `startedAt`/`updatedAt`
- Keep the claude version up to date (an old-version bug where PTY sessions were not saved — release notes observation)

## 12. 3-party final integration (confirmed — fable/Gemini/GPT)

**Verdict: core GO / risk zones gated.** Full reviews: `s9-final-review-fable.md`, `session-restore-final-review-gemini.md`, `-gpt.md`.

### 12-1. Confirmed (unanimous)
- **AgentId / claude_session_id separation** — supported by measurement.
- **§6 = frontend resubscribe (not subscriber succession)** [fable's recommendation + GPT's "channel succession NO-GO" agree]. Grounds: minimal change to the confirmed LLD, the frontend needs to be aware of the restart anyway for the banner, C2 (`terminal.reset`) is a verified idempotent path, failure visibility. **Implementation key: `epoch:u32` on `AgentInfo` + events, and the frontend effect deps `[agentId, epoch]`.** Restart goes to a dedicated restart task, not directly from drain.

### 12-2. Implementation gates
- **Core GO (immediately):** profile / AgentId separation / spawn (`--session-id`) / atomic persist / basic restore (`--resume`) / sid watcher / fresh fallback
- **Gate (after the core, provisional, verify before publishing as a trust feature):** auto-restart / re-entry from the terminal state [GPT NO-GO zone]

### 12-3. Required fixes (found by the 3 parties)
1. **[Gemini critical] cwd normalization with `dunce::canonicalize`** — `std::fs::canonicalize` attaches the UNC prefix (`\\?\`) on Windows and distorts the claude session folder name (`projects/<cwd치환>`) → mismatch with direct terminal execution breaks the restore. Avoid the prefix with dunce.
2. **[GPT biggest risk] No silent stale restore** — if sessions tracking fails, do not restore **silently** with the old sid. When uncertain, **explicit fresh fallback + notification**.
3. **[fable] Work around PID mismatch (shim)** — if the PTY child PID is a shim (claude.cmd→node), `sessions/<child_pid>.json` may not exist. Then **scan `sessions/*.json` for `sessionId == 우리 지정값`** (our sid is a unique key, so this is deterministic). Detect early with a self-test right after spawn.
4. **[fable security] No plaintext persist of `profile.env`** — if an API key/token is in env, it leaks in plaintext to `agents.json`. State explicitly that secrets must not be saved + warn on key patterns.
5. **[Gemini] sessions watch race** → **watch at the directory level**, not the file, + atomic updates.
6. **Operations:** consider that `~/.claude` may be moved by `CLAUDE_CONFIG_DIR`, a `sessions/<pid>.json` version gate + a watcher toggle.

### 12-4. LLD revision checklist (input to the code plan)
The 7 items, backend a~d / frontend e~g, from `s9-final-review-fable.md` — reflect them in the code plan (the §6 conflict is closed).
