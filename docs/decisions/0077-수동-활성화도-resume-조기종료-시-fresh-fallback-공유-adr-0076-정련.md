# ADR-0077: 수동 활성화도 resume 조기종료 시 fresh-fallback 공유 — ADR-0076 정련

- 상태: 확정 (2026-07-13, 근거: 데몬 콜드부팅 후 미대화 프로필 활성화 → claude "No conversation found with session ID <sid>" 로 즉사, fresh-fallback 미발화(daemon-debug.log))
- 관련: Amends ADR-0076 (수동 활성화(activate_profile)도 resume 조기종료 시 restore_one 과 동일한 fresh-fallback 을 공유한다) · ADR-0008(세션복원 sid 통제) · `agent/manager.rs::activate_profile`/`resume_with_fresh_fallback`/`restore_one`/`fallback_fresh` · `daemon/connection_core.rs::SpawnProfile` · step-log

## 맥락
사용자 결정: **"스폰만 하고 한마디도 안 한 건 그냥 새로 시작하는 게 좋지."** 즉 이어받을 대화가 실제로 있는 세션만 resume 하고, 이어받을 수 없는 세션(빈/미대화/삭제/손상 — claude 가 `--resume <sid>` 에서 "No conversation found with session ID ..." 로 즉시 종료)은 죽이지 말고 **새 sid 로 fresh 시작**해야 한다.

재현된 결함(데몬 콜드부팅 → 미대화 프로필 활성화): ADR-0076 이 활성화 모드 유도(`claude_session_id.is_some()` → Resume)와 Fresh 새 sid 발급은 고쳤으나, **resume 조기종료 → fresh-fallback 로직이 부팅 복원 경로(`restore_one`/`fallback_fresh`)에만 있고 수동 활성화 경로(SpawnProfile → `manager.spawn_agent`)에는 없었다.** `spawn_agent` 은 프로세스를 띄우기만 하고 조기종료를 감지하지 않는다. 그래서 대화가 없는 세션을 활성화하면 resume child 가 즉사 → reaper 가 수거 → 에이전트가 그냥 죽었다(fresh-fallback 미발화, 로그 확인).

## 결정
- **공용 fresh-fallback 헬퍼 추출** — `restore_one` 안의 "resume 시도 → EARLY_EXIT_WINDOW 조기종료 감지 → `fallback_fresh`" 규율을 `resume_with_fresh_fallback(profile) -> RestoreOutcome` 로 뽑는다. `restore_one`(부팅 복원)은 resumable 게이트만 두고 이 헬퍼를 호출한다(동작 불변).
- **수동 활성화 진입점 신설** — `AgentManager::activate_profile(profile, mode) -> Result<AgentInfo, PtyError>`. Resume 모드면 `resume_with_fresh_fallback` 을 태워 부팅 복원과 **동일한** 조기종료→fresh-fallback 규율을 적용하고, 결말(Resumed/FreshFallback → 살아있는 세션의 AgentInfo, Failed → Err)로 번역한다. Fresh 모드는 `spawn_agent(Fresh)` 를 그대로 위임(이어받을 대화가 없어 조기종료 감지 무의미 — 기존 동작 보존).
- **핸들러 배선 교체** — 데몬 `SpawnProfile` 핸들러가 `spawn_agent` 대신 `activate_profile` 을 호출한다. 모드 유도(ADR-0076 — resume-요청 OR 세션-존재)는 그대로 두고 실행부만 교체.

## 거부한 대안
- **spawn 전에 빈 세션을 선제 감지** — claude 의 세션 저장소를 들여다봐 "대화 있음/없음" 을 미리 판정해 모드를 정하는 방식. claude 내부 세션 파일 포맷·경로에 결합돼 취약하고(포맷 변경 시 깨짐), 백엔드 지식 격리(ADR-0004)를 침해한다. 반응형(resume child 가 조기종료하면 fallback)은 claude 가 실제로 "못 이어받는다" 고 신고한 사실에만 의존하므로 견고하고, 이미 검증된 ADR-0008 S9 인프라를 그대로 재사용한다.
- **`spawn_agent` 안에서 조기종료 감지** — `spawn_agent` 은 sid 발급 단일 권위점이자 `fallback_fresh` 자신이 Fresh 모드로 부르는 함수다. 여기에 조기종료 감지+fallback 을 넣으면 (1) `fallback_fresh` 의 fresh spawn 이 또 감지·재fallback 해 인과가 꼬이고 (2) 모든 spawn 이 EARLY_EXIT_WINDOW(3s) 블록된다. 그래서 감지·fallback 은 상위 진입점(`activate_profile`/`restore_one`)에만 두고 `spawn_agent` 은 순수 spawn 으로 유지한다.
- **수동 활성화에 fallback 을 아예 안 넣음(현 버그 유지)** — 사용자 결정 정면 위반(못 이어받는 세션이 죽음). 부팅 복원만 살아나고 수동 활성화는 죽는 비대칭이 재현 버그의 본체.

## 근거
데몬 콜드부팅 후 미대화 프로필 활성화 시 "No conversation found with session ID <sid>" 즉사가 재현됐고(daemon-debug.log), 원인이 "fresh-fallback 이 restore 경로에만 있고 활성화 경로엔 없음" 임을 코드·로그로 확인했다. 두 경로가 같은 헬퍼(`resume_with_fresh_fallback`)를 공유하므로 규율이 한 곳에서 유지돼 divergence 가 없다. ADR-0019 reaper single-consumer·finalize-once·epoch/kill 불변식은 `fallback_fresh`(기존)를 그대로 재사용해 불변으로 남는다 — `remove_session`(join_pump 로 stale terminal 소진) → epoch++ → `spawn_agent(Fresh)` 인과가 restore 와 동일.

## 영향 / 불변식
- **ADR-0076 정련(폐기 아님)** — "활성화=resume · Fresh=새 sid(재사용 금지) · sid 발급 단일점" 은 유효. 이 ADR 은 그 위에 "resume 조기종료 → fresh-fallback 을 부팅 복원·수동 활성화가 **공유**" 를 추가한다.
- **fresh-fallback 규율 단일 출처** — `resume_with_fresh_fallback` 이 정본. `restore_one`·`activate_profile` 둘 다 이걸 부른다 — 한쪽만 고치는 divergence 금지(수동 활성화가 죽던 원인이 바로 이 divergence).
- **`spawn_agent` = 순수 spawn** — 조기종료 감지·fallback 을 여기 넣지 않는다(sid 발급 단일점 + `fallback_fresh` 재진입 꼬임 + 무조건 3s 블록 회피).
- **결말 번역** — `activate_profile` Resume: Resumed/FreshFallback → 살아있는 세션 AgentInfo(fallback 후 `agent_info_by_id` 로 재조회), Failed → Err. Fresh → `spawn_agent(Fresh)` 그대로.
- 앵커: `// ADR-0077` (manager.rs `activate_profile`·`resume_with_fresh_fallback`), `// ADR-0076` (connection_core.rs `SpawnProfile` — 모드 유도 + activate_profile 배선).
