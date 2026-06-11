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

## H. 3자 검토 통합 (착수 전 필수 보강 — fable/Gemini/GPT)

**판정: 3자 조건부 GO.** 리뷰 전문: `s9-code-plan-review-fable.md`, `code-plan-review-gemini.md`, `-gpt.md`.

### H-1. 만장일치/다수 보강
1. **self_test (3자):** 즉시 1회 호출 = 레이스(spawn 직후 `sessions/<pid>.json` 미생성). → **bounded retry/지수백오프 폴링**(상한 내) + `startedAt`/`updatedAt` 검증 + 실패 시 degraded 상태 저장(추적만 끔, 무손상).
2. **구현 순서 (3자):** **E(LLD 개정)를 D(manager) 앞으로.** manager가 E의 의미론(맵 교체/epoch/Exited→재진입)을 구현하므로 LLD 선행 필수.
3. **atomic persist 강화 (Gemini/GPT):** flush→rename만으론 부족 → **같은 디렉토리 tmp + `sync_all` + rename + parent dir fsync.** `std::fs::rename` 크로스 파일시스템 오류 처리.
4. **ProfileRegistry 신설 (fable 최대누락 + GPT):** `Mutex<HashMap<AgentId, AgentProfile>>` + 변경시 save. 프로필 인메모리 **단일 소유자** — sid 생성·갱신·CRUD·watcher 콜백의 갱신 대상. **`sid==None`일 때 새 sid 생성·persist = ProfileRegistry 책임**(spawn_agent 아님).
5. **epoch++ 규칙화 (fable):** "같은 AgentId 맵 교체" **모든 지점** = epoch++ (restart + **fresh fallback respawn 포함**).
6. **watcher (Gemini/fable):** manager **단일 watcher** + 파일명 디스패치(에이전트당 N개 금지) + **정지 핸들 반환**(kill/respawn 시 좀비 방지). 시그니처에 스레드 안정성 명시.
7. **fallback 정밀 (Gemini/fable):** resume 실패 감지 = **"조기 종료 윈도"**(spawn 후 T초 내 `Exited{code≠0}` = resume 실패 → fallback). 잘못된 fallback로 대화 강제 유실 금지. **종점: fresh도 실패 → `Failed`+정지(재귀 금지).**
8. **restore_all 반환 (GPT/fable):** `Vec<{agent_id, epoch, outcome: Resumed | FreshFallback{old_sid,new_sid,reason} | Blocked | Failed}>`. setup 동기 호출 금지 → **백그라운드 태스크**(앱 창 블로킹 방지).

### H-2. 미래 확장 (3자: trait 경계만 + YAGNI)
- **지금 할 것 2건:** claude 전용 지식 → `pty/claude.rs` 격리(codex 대비+정리 이득) / 프로필 → 중립 `profile.rs`(types.rs 아님).
- **YAGNI:** 전면 `AgentSession` trait / 비-PTY API 일반화 = 두 번째 구현 때. 모바일 WS·codex는 OutputSink/StatusSink seam으로 흡수(지금 준비 불요).

### H-3. A 데이터 보강
- `restart_policy`(필드만 추가 + 기본 `Never` — schema 마이그레이션 절약) / `last_restore`(코어 GO 범위라 복원).

### H-4. 개정 구현 순서 (E 선행 반영)
1. `dunce` + `profile.rs`(중립 타입) + **ProfileRegistry**
2. persistence (atomic 강화)
3. session_tracker (단일 watcher + self_test 폴링) + **spike: PtyManager 경로 spawn → `<child_pid>.json` PID 일치 확인**
4. **LLD 개정 a~g (코어 前 선행)**
5. `pty/claude.rs` 격리 + manager: spawn_agent / restore_all(백그라운드) / fallback(종점)
6. commands/lib + 프론트 epoch 재구독
7. **[게이트]** restart 전용 태스크

### H-5. 분배 규칙
ProfileRegistry 단위 추가. A 후 B·C 병렬 가능. **각 모듈 산출물에 "구현 LLD 절 번호" 명기**(컴플라이언스 대조 효율).
