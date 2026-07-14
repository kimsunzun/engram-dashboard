# ADR-0076: 활성화=기존 세션 resume, Fresh는 새 sid 발급(재사용 금지) — ADR-0008 정련

- 상태: 확정 (2026-07-13, 근거: 데몬 콜드부팅 후 예약 프로필 활성화 재현 — "Session ID <sid> is already in use" 즉사)
- 관련: ADR-0008(세션복원 sid 통제) 정련 · `agent/manager.rs::spawn_agent`/`fallback_fresh` · `agent/profile.rs::new_session_id`/`ensure_session_id` · `daemon/connection_core.rs::SpawnProfile` · step-log · Amended by ADR-0077 (수동 활성화(activate_profile)도 resume 조기종료 시 restore_one 과 동일한 fresh-fallback 을 공유한다) · Amended by ADR-0082 (fallback_fresh 관련 불변식·"fresh-fallback 유효" 문구 폐지: 활성화=resume·Fresh=새 sid·sid 발급 단일점은 유효)

## 맥락
사용자 결정: **"에이전트를 활성화하면 기존 세션으로 이어져야 한다. 새로 로드될 거면 에이전트를 새로 만든다."** 활성화(activate)는 대화 이어받기(resume)이고, Fresh 시작은 진짜 신규 에이전트(이전 세션 없음)에게만 해당한다.

재현된 결함(데몬 콜드부팅 → 예약 프로필 활성화): claude 가 `Error: Session ID <sid> is already in use` 로 즉시 종료(wire status `Exited, code:null`). 두 결함이 겹쳐 있었다.
1. **잘못된 모드** — 활성화가 프로필에 이전 `claude_session_id` 가 있어도 `SpawnMode::Fresh` 로 spawn 했다(프론트 `spawnProfile(id, false)` → 데몬이 `resume=false → Fresh` 로 매핑). 이어받아야 할 상황에 새로 시작.
2. **Fresh 가 옛 sid 재사용** — `spawn_agent` 이 두 모드 모두 `ensure_session_id`(있으면 그대로 반환)를 불러, Fresh 가 `--session-id <저장된 sid>` 로 떴다. 디스크에 이미 그 세션 파일이 있으니 claude 가 충돌로 즉사. fresh-fallback(ADR-0008)도 같은 재사용 결함으로 재충돌 위험이 있었다.

## 결정
- **모드 유도(백엔드 권위)** — 프로필 활성화 시 세션 존재 여부로 모드를 정한다: `claude_session_id.is_some()` → **Resume**(`--resume <저장 sid>`, 대화 이어받기), `None` → **Fresh**(신규). 데몬 `SpawnProfile` 핸들러가 `mode = resume-요청 OR 세션-존재` 로 유도해, 프론트 wire `resume=false` 와 무관하게 세션 있는 프로필은 항상 이어받는다. 명시적 `resume=true` 는 그대로 존중.
- **Fresh 는 항상 새 sid** — `spawn_agent` 이 모드별로 sid 를 발급한다: Resume=`ensure_session_id`(저장값 그대로), Fresh=`new_session_id`(**항상 새 uuid**, 옛 sid 는 `old_session_ids` 이력으로 밀기). `new_session_id` 를 신설해 이 계약을 강제하고, `ensure_session_id` 는 Resume 전용으로 의미를 좁힌다.
- **sid 발급 단일 권위점** — 발급은 `spawn_agent` 한 곳에서만. `fallback_fresh` 는 더 이상 sid 를 미리 심지 않고 epoch 만 bump 한 뒤, `spawn_agent(Fresh)` 가 발급한 새 sid 를 읽어 보고한다(이중 발급 제거).

## 거부한 대안
- **활성화를 항상 Fresh 로** — 대화가 사라진다(사용자 결정 정면 위반: "이어져야 한다"). 이게 재현 버그의 원인 절반.
- **Fresh 가 저장된 sid 를 재사용(현 버그)** — `--session-id <기존 sid>` 가 디스크 세션과 충돌해 claude 즉사("already in use"). Fresh = 새 대화이므로 반드시 새 sid 여야 한다.
- **`spawn_agent` 안에서 mode 를 세션 존재로 무조건 재유도** — `fallback_fresh` 는 새 sid 를 심은 뒤 의도적으로 Fresh 로 부른다(세션이 `Some` 이어도 Fresh 여야 함). `spawn_agent` 이 존재만 보고 Resume 로 뒤집으면 fallback 이 깨진다. 그래서 모드 유도는 호출자(핸들러), sid 발급 규칙은 `spawn_agent` 으로 분리.

## 근거
데몬 콜드부팅 후 예약 프로필 활성화로 "Session ID already in use" 즉사가 재현됐고(daemon-debug.log), 원인이 (1) mode=Fresh (2) `ensure_session_id` 의 sid 재사용임을 코드·로그로 확인했다. 백엔드 권위(세션 존재 → Resume) + 발급 단일점으로 "어떤 호출자든 mode 만 맞으면 sid 충돌이 원천 봉인"된다. ADR-0008 의 통제-sid resume 인프라를 그대로 재사용하며, fresh-fallback 도 새 sid 로 재충돌이 없다.

## 영향 / 불변식
- **ADR-0008 정련(폐기 아님)** — "우리가 sid 통제 → `--resume` 무손실 복원"·"resume 조기종료 → fresh fallback → 종점 Failed" 는 유효. 이 ADR 은 그 위에 "활성화=resume · Fresh=새 sid(재사용 금지) · 발급 단일점" 을 명확화·수정한다.
- **`ensure_session_id` = Resume 전용** — Fresh 경로에서 부르면 옛 sid 재사용으로 "already in use" 회귀. Fresh 는 `new_session_id` 만.
- **sid 발급은 `spawn_agent` 한 곳** — `fallback_fresh` 등 다른 경로가 sid 를 직접 심으면 이중 발급으로 인과가 꼬인다(옛 sid 가 곧장 이력으로 밀림).
- 앵커: `// ADR-0076` (profile.rs `new_session_id`, manager.rs `spawn_agent` sid 분기·`fallback_fresh`, connection_core.rs `SpawnProfile`).
