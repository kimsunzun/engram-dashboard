# ADR-0070: 디스플레이 이름 = 백엔드 저장 override (Preset.name / AgentProfile.display_name, serde default·무마이그레이션)

- 상태: 확정 (2026-07-12, 근거: 구현+리뷰 deep 3인 PASS·qa full PASS)
- 관련: CLAUDE.md §5(LLM-우선 제어) · ADR-0061(프리셋 영속·cwd basename 이름 파생) 확장 · ADR-0057(탭 rename) 미러 · `crates/engram-dashboard-core/src/agent/preset.rs`(`Preset.name`·`rename`) · `.../agent/profile.rs`(`display_name`·`rename`) · `crates/engram-dashboard-protocol/src/messages.rs`(`RenamePreset`/`RenameProfile`) · `crates/engram-dashboard-daemon/src/connection_core.rs`(dispatch) · step-log

## 맥락

프리셋 팔레트(`PresetPalette`)·에이전트 트리(`AgentList`)에 뜨는 이름은 지금까지 등록 cwd 의 basename 으로 **파생**돼(ADR-0061) 사용자가 직접 바꿀 수 없었다. 탭은 이미 인라인 rename(ADR-0057) 가능한데 프리셋·에이전트는 불가라 비대칭이다.

기존 필드를 표시명으로 재사용할 수 없다는 게 핵심 제약이다:
- **`AgentProfile.name`** 은 표시명이 아니다 — `CreateProfile` 경로는 claude 프로필명, ad-hoc `SpawnByCwd` 경로는 **cwd 전체 문자열**이 들어가는 오염 축이다(`connection_core.rs`). 그래서 프론트 트리는 이 값을 무시하고 cwd basename 을 그려왔다.
- **`Preset`** 은 이름 필드 자체가 없다(cwd basename 파생, ADR-0061).

## 결정

표시명 override 를 **백엔드에 저장**한다. 두 필드를 신설:
- `Preset.name: Option<String>` (`#[serde(default)]`)
- `AgentProfile.display_name: Option<String>` (`#[serde(default)]`) — 기존 `name` 과 **별개 축**

표시 규칙: `Some` → 그대로 표시, `None` → cwd basename 파생(**기존 동작 불변**). rename 은 protocol command `RenamePreset`/`RenameProfile` 로 노출돼 override 를 set(`Some`)/clear(`None`) 한다. 정규화(trim·빈 문자열 거부·미변경 스킵)는 **프론트가 확정 직전에** 처리(TabBar rename 과 동형) — 백엔드엔 유효 값 또는 명시적 `None` 만 도달한다. 성공 시 `PresetListUpdated`/`ProfileListUpdated` 를 전 연결에 broadcast(낙관 갱신 X — 모든 창 동기화).

## 거부한 대안

- **프론트 전용 localStorage 저장** — §5 위반. rename 이 프론트 상태에만 살면 백엔드측 LLM/오케스트레이터가 닿는 제어 핸들이 없어 **사람 클릭으로만** 이름 변경이 되고, 창·머신 간 비영속이다.
- **기존 `AgentProfile.name` 재사용** — 오염 축이라 못 쓴다(위 맥락). `name` 을 표시명으로 덮으면 프로필 식별이라는 원래 의미가 파괴된다 → 별개 축 `display_name` 신설.
- **필드 신설 + 마이그레이션 스크립트** — 불필요. `#[serde(default)]` 라 이 필드가 없는 옛 `presets.json`/`agents.json` 은 로드 시 `None` 으로 흡수 → 무마이그레이션.

## 근거

§5(LLM-우선 제어)가 백엔드 저장의 핵심 이유다 — rename 이 command 로 노출돼 백엔드측 LLM/오케스트레이터도 동일 핸들로 이름을 바꾼다(사람 클릭은 같은 핸들의 보조 입력). serde default 무마이그레이션은 unit 테스트(필드 없는 JSON → `None` round-trip)로 확인. 리뷰 deep 3인 PASS · qa full PASS(GUI 실측: 프리셋·에이전트 양쪽 §5 핸들로 실 daemon 라운드트립 `null↔override↔null`, broadcast 반영).

## 영향 / 불변식

- **표시명 우선순위 = override(`Some`) > cwd basename.** 프론트 트리·팔레트는 `AgentProfile.name`(프로필 식별 축)을 표시명으로 쓰지 않는다 — `display_name`/`preset.name` 우선.
- 이 두 필드를 지우거나 `name` 으로 합치면 §5 rename·무마이그레이션 흡수가 깨진다.
- `#[serde(default)]` 제거 금지 — 옛 JSON(필드 없음) 로드가 깨진다.
- 동시 rename 의 영속 정합성은 ADR-0071(락 규율)이 담보한다 — §5 command 노출이 그 동시성 창을 여는 원인이라 두 ADR 은 한 슬라이스다.
