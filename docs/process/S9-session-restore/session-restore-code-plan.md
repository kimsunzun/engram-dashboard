# S9 상세 코드 기획 — 코어 GO 범위

근거: `session-restore-lld.md`(§12 확정) + `s9-final-review-fable.md`(LLD 개정 7건) + `spike-results.md`.
범위: **코어 GO**(프로필/분리/spawn/persist/복원/watcher/fallback). **자동재시작은 게이트** — 설계만 남기고 구현 보류.

## A. 데이터 모델 — `pty/types.rs` 확장

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

## B. `persistence/` (신규 모듈)

```rust
pub fn save_profiles(profiles: &[AgentProfile]) -> io::Result<()>;
// agents.json.tmp 쓰기 → flush → rename(atomic). 전역 Mutex 직렬화. schema_version 포함.
pub fn load_profiles() -> io::Result<Vec<AgentProfile>>;
// 손상 시 agents.json.corrupt-<ts> 보존 후 빈 목록. schema_version 게이트.
fn warn_if_secret(env: &[(String,String)]);  // 키/토큰 패턴 경고 (보안)
```

## C. `pty/session_tracker.rs` (신규 — sid watcher, best-effort)

```rust
/// PID shim 우회: sessions/<child_pid>.json 우선, 없으면 sessions/*.json에서 sessionId==expected 스캔(결정적).
pub fn resolve_session_file(child_pid: u32, expected_sid: Uuid) -> Option<PathBuf>;
/// spawn 직후 1회: <pid>.json 존재 + sessionId==expected 확인. 불일치 시 큰 로그(가정붕괴 조기감지).
pub fn self_test(child_pid: u32, expected_sid: Uuid) -> SelfTestResult;
/// sessions/ 디렉토리 watch(파일 아님 — 교체 견딤) + 파일명 필터. sessionId 변경 시 on_change(new_sid).
pub fn spawn_session_watcher(agent_id: AgentId, expected: Uuid, on_change: impl Fn(Uuid));
// version 게이트 + feature 토글. 읽기 공유위반 시 짧은 재시도.
```
> 갱신 콜백 → **즉시 atomic persist** (clear→관측→persist 전 크래시 시 stale 복원 방지, 1-b).

## D. `pty/manager.rs` 변경

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

## E. LLD 개정 (fable 7건 — §6 충돌 닫기)

**백엔드 `backend-lld-stage1.md`:**
- (a) §9: 재시작 시 같은 AgentId로 **새 PtySession 맵 교체 삽입** + `epoch++` (Exited→재진입 허용)
- (b) §10 스레드 목록: **restart 전용 태스크** 1행 (drain은 신호만, respawn은 태스크가)
- (c) §7 unsubscribe: stale sinkId retain not-found 무해 (명시)
- (d) §5/§8: StatusSink에 `restore-result` + `epoch` 반영

**프론트 `frontend-integration-lld.md`:**
- (e) §4: subscribe effect deps `[agentId, epoch]` (epoch 증가 = 재구독 트리거)
- (f) §4-1: 재시작 배너 순서 (terminal.reset → 배너 → replay)
- (g) §1/§4: `Restarting` 표시 또는 Exited→Running 재진입 허용

## F. `commands/` + `lib.rs`

- profile CRUD: `create_agent`(프로필) / `update_profile` / `delete_agent` / `list_profiles`
- setup: `init_logging` 후 `restore_all()` 트리거 (stagger)
- `agent-restore-result` 이벤트 emit (TauriStatusSink 확장)

## G. 구현 순서

1. `dunce` 의존 추가 (Cargo.toml)
2. types 확장 (AgentProfile, epoch, RestoreOutcome)
3. persistence (atomic save/load)
4. **session_tracker + spike 추가**(PtyManager 경로 spawn → `<child_pid>.json` 일치 self-test, 1-a 확인)
5. manager: spawn_agent(profile)/restore_all + fresh fallback
6. LLD 개정 (a~g) → 그 위에 commands/lib 복원 트리거 + epoch 재구독(프론트)
7. **[게이트]** restart_agent 전용 태스크 (코어 안정 후)

## 보안·방어
- `profile.env` 자격증명 금지(문서+패턴 경고), 이상적으로 시크릿 persist 제외 목록
- `sessions/<pid>.json` best-effort 등급 유지(correctness 의존 금지)
- `CLAUDE_CONFIG_DIR` 고려(sessions 경로 해석)
