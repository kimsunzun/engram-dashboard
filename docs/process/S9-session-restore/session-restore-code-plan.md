# S9 detailed code plan — core GO scope

Source: `session-restore-lld.md` (§12 confirmed) + `s9-final-review-fable.md` (7 LLD revisions) + `spike-results.md`.
Scope: **core GO** (profile/separation/spawn/persist/restore/watcher/fallback). **Auto-restart is gated** — only the design remains; implementation is on hold.

## A. Data model — extend `pty/types.rs`

```rust
pub struct AgentProfile {
    pub id: AgentId,                       // 불변 키
    pub name: String,
    pub command: AgentCommand,
    pub cwd: PathBuf,                       // 저장 전 dunce::canonicalize (UNC 회피)
    pub env: Vec<(String, String)>,        // ※자격증명 금지 — persist 시 *_KEY/*_TOKEN 패턴 경고
    pub claude_session_id: Option<Uuid>,   // 가변 (초기 = 우리 생성)
    pub old_session_ids: Vec<Uuid>,        // fallback/clear 이력
    pub epoch: u32,                        // 재spawn마다 +1 (프론트 재구독 트리거)
    pub auto_restore: bool,
    pub created_at: i64, pub last_active: i64,
}
pub enum AgentCommand { Claude { extra_args: Vec<String> }, Shell { program: String, args: Vec<String> } }
pub enum SpawnMode { Fresh, Resume }
// AgentInfo + AgentStatusChanged 에 epoch: u32 추가
pub enum RestoreOutcome { Resumed, FreshFallback { reason: String } }  // agent-restore-result
```

## B. `persistence/` (new module)

```rust
pub fn save_profiles(profiles: &[AgentProfile]) -> io::Result<()>;
// agents.json.tmp 쓰기 → flush → rename(atomic). 전역 Mutex 직렬화. schema_version 포함.
pub fn load_profiles() -> io::Result<Vec<AgentProfile>>;
// 손상 시 agents.json.corrupt-<ts> 보존 후 빈 목록. schema_version 게이트.
fn warn_if_secret(env: &[(String,String)]);  // 키/토큰 패턴 경고 (보안)
```

## C. `pty/session_tracker.rs` (new — sid watcher, best-effort)

```rust
/// PID shim 우회: sessions/<child_pid>.json 우선, 없으면 sessions/*.json에서 sessionId==expected 스캔(결정적).
pub fn resolve_session_file(child_pid: u32, expected_sid: Uuid) -> Option<PathBuf>;
/// spawn 직후 1회: <pid>.json 존재 + sessionId==expected 확인. 불일치 시 큰 로그(가정붕괴 조기감지).
pub fn self_test(child_pid: u32, expected_sid: Uuid) -> SelfTestResult;
/// sessions/ 디렉토리 watch(파일 아님 — 교체 견딤) + 파일명 필터. sessionId 변경 시 on_change(new_sid).
pub fn spawn_session_watcher(agent_id: AgentId, expected: Uuid, on_change: impl Fn(Uuid));
// version 게이트 + feature 토글. 읽기 공유위반 시 짧은 재시도.
```
> Update callback → **immediate atomic persist** (prevents a stale restore if a crash happens between clear → observation → persist, 1-b).

## D. `pty/manager.rs` changes

```rust
pub fn spawn_agent(&self, profile: &AgentProfile, mode: SpawnMode) -> Result<AgentInfo, PtyError>;
//  guard: id가 sessions 맵에 있으면 거부(이중 spawn)
//  cwd = dunce::canonicalize(profile.cwd)
//  sid = profile.claude_session_id (없으면 새 uuid)
//  args: Claude+Fresh→[--session-id sid], Claude+Resume→[--resume sid], Shell→그대로
//  spawn → self_test(child_pid, sid) → spawn_session_watcher
pub fn restore_all(&self) -> Vec<RestoreOutcome>;
//  auto_restore 프로필만, stagger. Claude→Resume 시도.
//  --resume 실패(exit1/no-conversation) → 새 uuid Fresh fallback(명시), old_session_ids 이력, restore-result emit
//  ※ silent stale 금지: 불확실하면 fresh
pub fn restart_agent(&self, id: AgentId);  // [게이트] 전용 태스크에서만. 사다리 resume2→fresh→정지. epoch++.
```

## E. LLD revisions (fable's 7 items — close the §6 conflict)

**Backend `backend-lld-stage1.md`:**
- (a) §9: on restart, **insert a new PtySession by replacing the map entry** under the same AgentId + `epoch++` (allow Exited→re-entry)
- (b) §10 thread list: one row for a **restart-dedicated task** (drain only signals; the task does the respawn)
- (c) §7 unsubscribe: a stale sinkId finding nothing in retain is harmless (state it explicitly)
- (d) §5/§8: reflect `restore-result` + `epoch` in StatusSink

**Frontend `frontend-integration-lld.md`:**
- (e) §4: subscribe effect deps `[agentId, epoch]` (an epoch increase = a resubscribe trigger)
- (f) §4-1: restart banner order (terminal.reset → banner → replay)
- (g) §1/§4: show `Restarting`, or allow Exited→Running re-entry

## F. `commands/` + `lib.rs`

- profile CRUD: `create_agent` (profile) / `update_profile` / `delete_agent` / `list_profiles`
- setup: trigger `restore_all()` after `init_logging` (stagger)
- emit the `agent-restore-result` event (extend TauriStatusSink)

## G. Implementation order

1. Add the `dunce` dependency (Cargo.toml)
2. Extend types (AgentProfile, epoch, RestoreOutcome)
3. persistence (atomic save/load)
4. **session_tracker + an added spike** (spawn through the PtyManager path → self-test that `<child_pid>.json` matches, confirms 1-a)
5. manager: spawn_agent(profile)/restore_all + fresh fallback
6. LLD revisions (a~g) → on top of that, the commands/lib restore trigger + epoch resubscribe (frontend)
7. **[Gate]** the restart_agent dedicated task (after the core is stable)

## Security and defense
- No credentials in `profile.env` (docs + pattern warning), ideally an exclusion list so secrets are not persisted
- Keep the `sessions/<pid>.json` at best-effort grade (do not depend on it for correctness)
- Consider `CLAUDE_CONFIG_DIR` (resolving the sessions path)

## H. 3-party review integration (mandatory reinforcement before starting — fable/Gemini/GPT)

**Verdict: 3 parties conditional GO.** Full reviews: `s9-code-plan-review-fable.md`, `code-plan-review-gemini.md`, `-gpt.md`.

### H-1. Unanimous/majority reinforcements
1. **self_test (3 parties):** a single immediate call = a race (`sessions/<pid>.json` is not yet created right after spawn). → **bounded retry / exponential-backoff polling** (within a limit) + `startedAt`/`updatedAt` verification + on failure save a degraded state (turn off only the tracking, without damage).
2. **Implementation order (3 parties):** **put E (LLD revisions) before D (manager).** The manager implements E's semantics (map replacement/epoch/Exited→re-entry), so the LLD must come first.
3. **Stronger atomic persist (Gemini/GPT):** flush→rename alone is not enough → **tmp in the same directory + `sync_all` + rename + parent dir fsync.** Handle `std::fs::rename` cross-filesystem errors.
4. **Add a ProfileRegistry (fable's biggest omission + GPT):** `Mutex<HashMap<AgentId, AgentProfile>>` + save on change. The **single in-memory owner** of profiles — the update target for sid generation/update, CRUD, and watcher callbacks. **When `sid==None`, generating and persisting a new sid = the ProfileRegistry's responsibility** (not spawn_agent's).
5. **Make epoch++ a rule (fable):** **every point** that does a "same AgentId map replacement" = epoch++ (restart + **including the fresh fallback respawn**).
6. **watcher (Gemini/fable):** a **single watcher** in the manager + dispatch by file name (no N per agent) + **return a stop handle** (to prevent zombies on kill/respawn). State thread safety in the signature.
7. **Precise fallback (Gemini/fable):** resume failure detection = **an "early exit window"** (`Exited{code≠0}` within T seconds after spawn = resume failure → fallback). No forced conversation loss by a wrong fallback. **Endpoint: if fresh also fails → `Failed` + stop (no recursion).**
8. **restore_all return (GPT/fable):** `Vec<{agent_id, epoch, outcome: Resumed | FreshFallback{old_sid,new_sid,reason} | Blocked | Failed}>`. No synchronous call in setup → **a background task** (prevents blocking the app window).

### H-2. Future extension (3 parties: only a trait boundary + YAGNI)
- **Two things to do now:** isolate claude-specific knowledge into `pty/claude.rs` (prepares for codex + cleanup benefit) / put the profile in a neutral `profile.rs` (not types.rs).
- **YAGNI:** a full `AgentSession` trait / generalizing non-PTY APIs = at the second implementation. Mobile WS and codex are absorbed through the OutputSink/StatusSink seam (no preparation needed now).

### H-3. A data reinforcement
- `restart_policy` (add only the field + default `Never` — saves a schema migration) / `last_restore` (restored because it is within core GO scope).

### H-4. Revised implementation order (E-first applied)
1. `dunce` + `profile.rs` (neutral types) + **ProfileRegistry**
2. persistence (stronger atomic)
3. session_tracker (single watcher + self_test polling) + **spike: spawn through the PtyManager path → confirm `<child_pid>.json` PID match**
4. **LLD revisions a~g (before the core)**
5. `pty/claude.rs` isolation + manager: spawn_agent / restore_all (background) / fallback (endpoint)
6. commands/lib + frontend epoch resubscribe
7. **[Gate]** the restart dedicated task

### H-5. Distribution rules
Add a ProfileRegistry unit. After A, B and C can run in parallel. **State the "implemented LLD section number" in each module's deliverable** (efficient compliance cross-checking).
