# PRD(후보) — 에이전트/데몬 수명 정책: 변수 스키마 단계

- 상태: **선택 대기** (consult 교차검증 완료, 사용자 결정으로 PRD 확정 예정)
- 범위: persist 데몬이 든 claude 에이전트의 (1)부팅 복원 (2)런타임 재시작 (3)크래시 폭주 가드. **이번 단계 = 변수(스키마)만 정의, 동작 로직은 TODO.**
- 근거: `/consult` job `20260616-001941-consult-lifecycle-policy` — GPT·Gemini·Claude(opus) 블라인드 교차검증 + judge 판정. 원자료: `I:\Engram\agents\web-runner\shared\<job>\`.

## 고정 제약 (재론 금지)
- 데몬 = persist-until-kill, 콘솔=detachable 뷰어 (ADR-0015). UI 열림이 복원 트리거가 되면 안 됨.
- 스케줄러 = 독립 캡슐, 이번 범위 밖.
- 변수만 정의, 동작 TODO.

## A. 합의된 결론 (3종 + judge 일치 — 거의 확정, 변수에 반영)
1. **`auto_restore`(의도) ≠ "마지막 running"(관측)은 직교 축 — 분리.** 한 필드로 뭉개면 "사용자가 끈 걸 데몬이 부활"·"잠깐 멈춤이 영구 제외" 혼란. → 관측 상태를 별도 변수로.
2. **Failed는 콜드부팅 간 영속돼야** — 현재 `agents.json`에 status 필드가 없어 부팅하면 Failed가 증발하고 무조건 복원 사다리 재진입. "Failed=자동재시도 중단·데이터 보존"을 부팅 너머로 유지하려면 영속 변수 필요.
3. **데몬 가드 × 에이전트 가드 카운터 휘발 상호작용** — 데몬 재부팅이 in-memory 에이전트 카운터를 0으로 리셋하면 무한루프를 못 막음. 가드 상태 일부 영속 필요(또는 데몬 가드가 잡아줌).
4. **전역 튜닝값은 신규 `daemon-config.json`** (C2) — `agents.json`은 per-agent라 부적합. `schema_version` 포함. ProfileStore 동형 trait으로 격리.
5. **`last_restore` → `last_start_at` rename** — 소비자 0건이라 ts-rs 재생성만으로 충분(deprecated 호환레이어 불필요).
6. **per-agent 가드 분리 잘됨**(전역 카운터 아님), **스케줄러 분리 잘됨**(wake/restore에 시각·주기 안 섞임).
7. **데몬 재spawn 가드 주체 = Tauri Core(Rust 메인 런타임), 프론트(React) 아님** — UI 닫힌 채 데몬이 죽을 수 있으므로. 변수도 Rust 측에.
8. **exit-kind 분류 필요** — `Killed`(사람이 죽인 것)는 절대 자동재시작 대상 아님(안 그러면 kill해도 부활). OnCrash 판정의 전제.

## B. ★사용자 결정 대기 쟁점★ (PRD 확정에 필요)

### B-1. restart_policy 기본값 — Never vs OnCrash
- **Never(추천: 변수 단계 안전)**: 동작 구현 순간 "기존 프로필 전부 자동재시작"되는 의미 전환(마이그레이션 충격) 회피. exit-kind 분류도 미구현이라 보수적이 맞음.
- **OnCrash(제품 철학)**: 자율 에이전트 데몬엔 자연스러우나, 가드·exit판정 검증 전엔 위험.
- **절충안(judge가 우위로 본 것)**: **스키마 default = Never**로 두고, **신규 생성 UI의 프리셋만 OnCrash**로 — "스키마 기본값 ≠ 생성폼 프리셋" 분리. → **결정 필요.**

### B-2. ★가드 리셋 stable_secs 값 + 슬라이딩 윈도 보조★ (가장 중요 — 내 이전 제안 정정)
- 내가 앞서 제안한 **stable_secs=30초는 claude엔 위험**으로 판정됨(Gemini 단독 정량 적출, judge 인정): claude는 정상 1프롬프트가 **1~2분** 걸리는데, 30초 안정 기준이면 "35초 살다 죽는"(API 타임아웃·요금부족) 루프가 **매번 카운터를 0으로 리셋해 무한 폭주**를 못 막음.
- 올바른 방향(두 개 병행 권장):
  - (a) **stable_secs를 워크로드에 맞게 크게** — 예: **300초(5분)** 이상.
  - (b) **슬라이딩 윈도 보조 스키마 자리만 지금 열어둠** — `window_secs: Option`, `window_max: Option`(초기 None). "stable_secs보다 약간 길게 살다 반복적으로 죽는" 케이스 대비.
- → **결정 필요: stable_secs 값(예 300) + 슬라이딩 윈도 보조 필드를 지금 열지.**

### B-3. Failed 콜드부팅 정책 — 유지 vs 1회 재시도
- `SkipUntilManualRestart`(추천): 부팅해도 Failed 유지, 사람/LLM이 풀 때까지 자동복원 제외.
- `RetryOnDaemonColdBoot`: OS 재부팅·업데이트 후 1회 재시도(환경 문제였으면 회복). 단 데몬폭주×에이전트폭주 합쳐질 위험.
- enum으로 두 선택지 다 표현 가능. → **기본값 결정 필요.**

### B-4. 복원 대상 — 단순(auto_restore 전부) vs 관측결합(auto_restore AND was_running)
- 합의는 "의도/관측 분리"(A.1)지만, **초기 동작**을 단순(auto_restore=true 전부 복원)으로 갈지, 관측(`was_running`)까지 결합할지는 선택. persist-until-kill이라 "마지막 running"은 *데몬 크래시/OS재부팅* 때만 의미(평상시 무관). → **결정 필요**(변수는 어차피 까두되, 복원 함수가 무엇을 보는지).

### B-5. 가드 카운터 영속 여부
- A.3 때문에 필요는 인정. **per-agent `consecutive_failures`를 `agents.json`에 영속**(데몬 재부팅 넘어 유지, 단 죽음마다 파일 rewrite 비용) vs **in-memory + 데몬 가드가 폭주를 잡음**. → **결정 필요.**

## C. TRD로 넘길 seam 결정 (변수만으로 안 닫힘 — A=Claude 단독 적출)
- **★런타임 죽음 관측 훅이 코드에 없음★**: 현재 `AgentManager`는 terminal 신호(`status_changed`, pump 단독 발행)를 **구독하지 않는다**. 즉 "에이전트가 런타임에 죽었다"를 manager가 알 방법이 없음 → **재시작 캡슐은 단순 동작 TODO가 아니라 "관측 seam 신설"이 전제.** 후보: StatusSink 되먹임 / OutputCore done_rx supervisor join / 신규 LifecycleSink. (TRD에서 결정)
- **부팅 복원 이중 경로**: 데몬 + src-tauri(Embedded) 둘 다 `restore_all` 호출 — "평상시 복원 안 돎" 가정이 Embedded 직접 실행서 깨짐. 모드 토글 정착 전까지 처리 방침 필요.
- daemon_respawn 가드: `ensure_daemon`에 재시도 루프 자체가 없음 → Tauri Core 측 신규 상태+루프 필요.
- (참고) PTY 고아: 데몬 급사 시 JobObject(`KILL_ON_JOB_CLOSE`)가 자식 동반 종료를 이미 보장 → Gemini의 "고아 이중 토큰" 우려는 Windows에선 완화됨(비Windows·비정상 경로만 주의).

## D. correctness-merge 변수 스키마 초안 (선택에 따라 가감)
> 골격=GPT, E값/슬라이딩=Gemini, 죽음관측 seam·last_restore 처리=Claude 채택. judge가 틀렸다 한 것 제외(30s 단언·PTY고아 과장·deprecated 레이어).

**per-profile (agents.json):**
- `auto_restore: bool` (있음) — 복원 의도/자격.
- `restart_policy: {Never,OnCrash,Always}` (있음) — 기본값 B-1 결정.
- `last_known_state: {Stopped,Running,Failed}` — 신규. 관측 상태(A.1)+Failed 영속(A.2). "Stopped=사용자가 끔 / Failed=가드 격리".
- `last_start_at: Option<i64>` — `last_restore` rename(A.5). 안정가동 리셋 경과시간 측정.
- `consecutive_failures: u32` — 신규. 가드 카운터(영속 여부 B-5).
- (선택) `last_exit_kind: Option<{ManualStop,NormalExit,CrashExit,SpawnFailed,RestoreFailed,DaemonShutdown,Unknown}>`, `failed_at`, `failed_reason` — exit 분류·Failed 사유(A.8). 지금 다 넣을지 결정.

**전역 (daemon-config.json 신규):**
- `schema_version: u32`
- `restore_target_policy: {AutoRestoreProfiles, LastRunningOnly}` (B-4, 기본 AutoRestoreProfiles)
- `failed_restore_policy: {SkipUntilManualRestart, RetryOnDaemonColdBoot}` (B-3)
- `agent_restart_guard: { stable_secs(예 300), restart_max(예 5), window_secs: Option, window_max: Option }` (B-2)
- `daemon_respawn_guard: { stable_secs(예 60), respawn_max(예 3) }`

**Tauri Core(Rust, in-memory) — 데몬 감시 (A.7):**
- `DaemonSupervisorState { daemon_respawn_count: u32, last_daemon_start_at: Option<Instant> }` — 프론트 아님, daemon-config와 분리.

## 다음
- 위 **B-1~B-5 + (선택) exit-kind 필드 범위**를 사용자가 고르면 → PRD 확정 → TRD(C의 seam 결정) → ADR-0016 → 코더로 필드 추가(동작 0).
