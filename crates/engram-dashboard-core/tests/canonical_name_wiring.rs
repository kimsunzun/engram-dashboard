//! canonical 이름 **배선** 통합테스트(ADR-0101 WYSIWYA — 생산 경로 회귀 그물).
//!
//! ★왜 이 파일이 따로 있나(뮤테이션 프로브 D9-b)★: `agent/name.rs` 의 파생 테스트는 헬퍼를
//!   **직접** 부른다 — 그래서 생산 코드가 그 헬퍼를 **쓰지 않게 되어도** 전부 초록이다. 실제로
//!   `manager::resolve_canonical_name` 에 다음 세 뮤테이션을 넣어도 core 290 + daemon 412 테스트가 모두
//!   통과했다: ① `session.cwd` 대신 `profile.cwd` ② 가드 있는 `canonical_name_or_id_fallback` 대신
//!   `resolve_display_name` ③ 파생을 건너뛰고 저장된 `AgentProfile.name` 필드 반환. ③은 ADR-0101 이 고친
//!   버그 그 자체(저장 이름 = 종종 full cwd 문자열을 라우팅 주소로 사용 → 트리 표시 ≠ 라우팅 주소)다.
//!
//! ★그래서 여기서는 헬퍼를 부르지 않는다★: 실 세션을 스폰해 **manager 의 공개 투영**
//!   (`spawn_agent` 반환 `AgentInfo.name` · `list_agents()` · `canonical_name(id)`)만 본다. 세 뮤테이션이
//!   각각 다른 케이스에서 붙잡히도록 케이스를 갈랐다.
//!
//! ★잠듦 파킹 키와의 관계(ADR-0116 결정 1)★: 잠든 프로필의 이름은
//!   `AgentProfile::canonical_name_when_live()` 가 파생하고, 그 값은 **여기서 단언하는 산 이름과 같아야**
//!   한다(다르면 파킹 키와 복원 후 이름이 어긋나 편지가 24h TTL 로 조용히 만료된다). 그래서 각 케이스는
//!   두 파생이 **같은 문자열**을 내는지도 함께 못 박는다 — 한쪽만 바뀌면 여기서 깨진다.
//!
//! Windows 전용 — cmd.exe 를 스폰한다.
// ADR-0101 / ADR-0116

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::PresetRegistry;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

fn make_manager(tag: &str) -> AgentManager {
    let sink: Arc<dyn StatusSink> = Arc::new(NoopSink);
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-canon-{tag}-{}", Uuid::new_v4())),
    ))));
    let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
        std::env::temp_dir().join(format!("engram-canon-preset-{tag}-{}", Uuid::new_v4())),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    AgentManager::new(sink, profiles, presets, tracker)
}

/// 스폰해도 즉시 죽지 않는 무해한 자식(대화형 cmd.exe — PTY 아래서 프롬프트를 띄우고 대기).
/// 이름 파생은 프로세스 동작과 무관하므로 "산 세션이 존재한다" 만 필요하다.
fn idle_shell(name: &str, cwd: PathBuf) -> AgentProfile {
    AgentProfile::new(
        name.to_string(),
        AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        },
        cwd,
        vec![],
        false,
    )
}

/// 하나만 보면 다른 경로가 갈렸어도 안 잡힌다 — 그래서 셋을 다 본다.
fn projected_name(manager: &AgentManager, spawned: &AgentInfo) -> String {
    let listed = manager
        .list_agents()
        .into_iter()
        .find(|a| a.id == spawned.id)
        .expect("스폰한 세션은 목록에 있다");
    let by_id = manager.canonical_name(spawned.id).expect("id → 이름");
    assert_eq!(
        spawned.name, listed.name,
        "spawn 반환과 list_agents 의 이름이 갈렸다(같은 코어를 써야 한다)"
    );
    assert_eq!(
        spawned.name, by_id,
        "agent_info 와 canonical_name 이 갈렸다(봉투 sender ≠ 라우팅 주소가 된다 — ADR-0101)"
    );
    spawned.name.clone()
}

#[test]
fn the_stored_profile_name_field_is_never_used_as_the_routing_address() {
    // ★ADR-0101 이 고친 버그의 회귀 그물(뮤테이션 ③)★: `AgentProfile.name` 은 CreateProfile 때 받은 원본
    //   문자열이라 **종종 경로**다. 그걸 이름으로 쓰면 트리 표시명(basename)과 라우팅 주소가 갈린다.
    let manager = make_manager("namefield");
    let cwd = std::env::temp_dir();
    let raw_name = "C:\\some\\raw\\name-field-must-not-be-the-address";
    let profile = idle_shell(raw_name, cwd.clone());

    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
    let name = projected_name(&manager, &info);
    manager.kill_agent(info.id).ok();

    assert_ne!(
        name, raw_name,
        "저장된 name 필드가 라우팅 주소로 새어 나왔다(ADR-0101 회귀)"
    );
    let expected = dunce::canonicalize(&cwd)
        .expect("temp_dir 은 실재한다")
        .file_name()
        .expect("basename")
        .to_string_lossy()
        .to_string();
    assert_eq!(name, expected, "이름 = basename(canonicalize(cwd))");
}

#[test]
fn a_relative_cwd_derives_the_name_from_the_canonicalized_session_cwd() {
    // ★뮤테이션 ①(session.cwd → profile.cwd) 적출★: 프로필엔 raw 경로가 들어올 수 있고(`"."`·`".."`·
    //   심링크·대소문자 다른 Windows 경로) spawn 은 그걸 canonicalize 해 session.cwd 로 쓴다. 프론트 트리는
    //   `basename(AgentInfo.cwd)` = basename(session.cwd) 를 그리므로, 파생을 profile.cwd 로 하면 트리
    //   표시(`engram-dashboard-core`)와 라우팅 주소(`"."`)가 갈린다.
    let manager = make_manager("relcwd");
    let profile = idle_shell("rel", PathBuf::from("."));

    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
    let name = projected_name(&manager, &info);
    manager.kill_agent(info.id).ok();

    assert_ne!(name, ".", "raw cwd basename 이 주소가 되면 안 된다");
    let expected = dunce::canonicalize(".")
        .expect("cwd 는 실재한다")
        .file_name()
        .expect("basename")
        .to_string_lossy()
        .to_string();
    assert_eq!(name, expected, "canonicalize 된 cwd 의 basename 이어야");
    assert_eq!(
        profile.canonical_name_when_live(),
        name,
        "잠듦 파킹 키와 산 이름이 갈리면 편지가 주인을 못 만난다"
    );
}

#[test]
fn a_blank_display_name_override_is_ignored_by_the_production_path() {
    // ★뮤테이션 ②(canonical_name_or_id_fallback → resolve_display_name) 적출★: 후자는 빈/공백-only
    //   override 를 **그대로 이름으로 쓴다** — 주소가 공백 문자열이 되면 지목이 불가능하고 트리엔 빈 칸이
    //   그려진다. 생산 경로는 그 가드가 있는 헬퍼를 써야 한다.
    let manager = make_manager("blankoverride");
    let cwd = std::env::temp_dir();
    let mut profile = idle_shell("blank", cwd.clone());
    profile.display_name = Some("   ".into());

    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
    let name = projected_name(&manager, &info);
    manager.kill_agent(info.id).ok();

    assert!(
        !name.trim().is_empty(),
        "공백-only override 가 주소로 새어 나왔다: {name:?}"
    );
    let expected = dunce::canonicalize(&cwd)
        .expect("temp_dir 은 실재한다")
        .file_name()
        .expect("basename")
        .to_string_lossy()
        .to_string();
    assert_eq!(name, expected, "공백 override 는 무시하고 cwd basename");
    assert_eq!(
        profile.canonical_name_when_live(),
        name,
        "잠듦 파킹 키도 같은 가드를 통과해야(두 파생 단일 규칙)"
    );
}

#[test]
fn a_real_display_name_override_wins_over_the_cwd_basename() {
    // ★삭제 정리(ADR-0116 결정 3)의 전제★: 이 등식이 깨지면 정리가 엉뚱한 큐를 지목한다.
    let manager = make_manager("override");
    let mut profile = idle_shell("ovr", std::env::temp_dir());
    profile.display_name = Some("Renamed-Boss".into());

    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn")
        .into_started()
        .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
    let name = projected_name(&manager, &info);
    manager.kill_agent(info.id).ok();

    assert_eq!(name, "Renamed-Boss");
    assert_eq!(profile.canonical_name_when_live(), name);
}
