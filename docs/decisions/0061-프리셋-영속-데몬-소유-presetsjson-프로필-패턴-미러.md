# ADR-0061: 프리셋 영속 = 데몬 소유 (presets.json, 프로필 패턴 미러)

- 상태: 확정 (2026-07-10, 근거: pre-PRD 설계-결정 리서치 + 사용자 결정, step-log 2026-07-10)
- 관련: CLAUDE.md §5(LLM-우선 제어)·아키텍처 §4(AgentManager 소유=데몬) · ADR-0024(`.engram-data/` 위치) · ADR-0029(embedded 제거, 데몬 데이터 단일소유) · ADR-0028(백엔드 이벤트버스 소유) · `crates/engram-dashboard-core/src/persistence/mod.rs`(FileProfileStore 미러 대상) · step-log "에이전트 트리·프리셋"

## 맥락

에이전트 트리 + 프리셋 MVP에서 **프리셋**("등록해둔 cwd 경로" 목록)을 어디에 저장하느냐를 정해야 했다. 프리셋은 배경 우클릭 → "에이전트 생성" picker에 뜨는 등록 경로 목록으로, `{ id, cwd }`만 담는다(model·icon·backend·inject는 나중). 이건 **이 프로젝트 최초의 백엔드-영속 유저 데이터**다 — 지금까지 유저가 만든 영속 데이터는 프로필(agents.json)뿐이었고, 프론트 상태(테마·채팅 스타일)는 localStorage에 있었다.

저장 위치가 곧 "권위(authority)"를 정한다: 여러 창(main + 팝아웃 웹뷰)이 같은 프리셋 목록을 봐야 하고(멀티모니터), §5(LLM-우선 제어)상 프리셋 CRUD는 백엔드측 LLM(두뇌)이 쥐는 핸들이어야 한다.

## 결정

**프리셋은 데몬이 소유한다.** 프로필(agents.json) 시스템을 그대로 미러:

- **저장:** `.engram-data/presets.json`(ADR-0024 `default_data_dir()`). core에 `FilePresetStore`(atomic write: tmp → sync_all → rename) + `PresetRegistry`(in-memory + store 위임) — `FileProfileStore`/`ProfileRegistry` 패턴 복제.
- **소유:** 데몬이 startup에 `build_manager`에서 `FilePresetStore::new(data_dir)` → `PresetRegistry`를 구성해 `AgentManager`가 보유(ADR-0029: 데몬이 유저 데이터 단일 소유).
- **wire 계약:** `AgentCommand::{ListPresets, CreatePreset{cwd}, DeletePreset{preset_id}}` + `AgentEvent::{PresetList{request_id, presets}(조회 응답), PresetListUpdated{presets}(CRUD 후 broadcast)}` — 프로필의 `ListProfiles`/`CreateProfile`/`DeleteProfile` + `ProfileList`/`ProfileListUpdated`와 1:1 대응.
- **broadcast:** create/delete 후 `broadcast_preset_list()`로 전 연결에 push(ADR-0028 단일 push 채널) → 모든 창이 자동 동기화.
- **데이터 모델:** `Preset { id: Uuid, cwd: PathBuf }`. 이름은 파생(cwd basename) — 저장 안 함.

## 거부한 대안

- **프론트 localStorage** — 웹뷰(창)마다 저장소가 갈려 팝아웃 멀티모니터에서 desync. §5 위반(프론트가 데이터 권위를 쥠 = 두뇌가 못 닿음). 프론트 상태는 창-로컬 UI 취향(테마·폰트)에만 쓰고, 공유 유저 데이터는 백엔드가 정답.
- **src-tauri(클라이언트 셸) 소유** — src-tauri는 단일 프로세스라 창끼리는 동기화되지만, ADR-0029가 유저 데이터 단일 소유를 데몬으로 못 박았고 §5 두뇌(=데몬측 LLM)가 프리셋을 제어해야 한다. src-tauri에 두면 이중 소유 위험 원천(ADR-0029가 없앤 바로 그 문제)이 재발.
- **프로필(agents.json)에 프리셋 필드 겸용** — 프로필 = 스폰된/예약된 에이전트 인스턴스(sid·epoch·restart 정책 있음, ADR-0016). 프리셋 = 스폰 전 "경로 북마크"(인스턴스 아님). 의미가 다른 둘을 한 저장소에 섞으면 수명·삭제 의미가 꼬인다(프리셋 삭제 ≠ 에이전트 끔). 별도 store로 분리.
- **프리셋을 새 SlotContent variant 데이터로만** — 프리셋 목록은 슬롯 렌더(PresetPalette)와 별개로 "등록된 경로 집합"이라는 영속 상태다. UI variant config에 묶으면 레이아웃 저장/복원과 수명이 얽힌다 — 데이터(데몬)와 표현(SlotContent)을 분리.

## 근거

- **프로필 선례가 이미 검증됨** — `FileProfileStore`의 atomic write·버전체크·손상보존, `ProfileListUpdated` broadcast, ProtocolClient 조회/구독이 실동작 중. 프리셋은 이 검증된 경로를 복제만 하면 됨(바닥부터 안 짬 — 참조 구현 원칙).
- **§5·ADR-0029 정합** — 데몬 = 두뇌 + 유저 데이터 단일 소유. 프리셋을 여기 두면 멀티창 동기화·LLM 제어가 공짜로 따라옴(broadcast 채널 재사용).
- **최소 스키마** — `{id, cwd}`만. model/icon/inject는 실수요 나올 때 필드 추가(over-engineering 회피, 단 store/wire seam은 지금 깔아 확장은 값싸게).

## 영향 / 불변식

- **`presets.json`는 데몬만 쓴다** — 프론트·src-tauri가 직접 파일을 만지지 않는다. 프론트는 ProtocolClient wire로만 CRUD(ADR-0011 제어 표면).
- **create/delete는 반드시 broadcast로 이어진다** — 안 그러면 다른 창이 stale. 프로필 `broadcast_profile_list` 규약과 동일.
- **프리셋 삭제 ≠ 에이전트 종료** — 프리셋은 스폰 전 북마크. 그 프리셋으로 이미 스폰된 에이전트는 프리셋 삭제와 무관하게 산다(수명 분리).
- **이름은 저장하지 않는다(cwd basename 파생)** — 나중 프리셋 리치화(사용자 지정 이름)에서 필드 추가 시 이 파생 규칙과 충돌하지 않게 `name: Option<String>` 오버라이드로 확장(파생이 기본).
