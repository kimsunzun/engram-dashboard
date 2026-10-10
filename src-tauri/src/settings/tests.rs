use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::store::fake::MemFiles;
use super::*;

fn service(files: &MemFiles) -> SettingsService {
    let svc = SettingsService::load(Box::new(files.clone()));
    svc.enable_writes();
    svc
}

fn value_of(svc: &SettingsService, key: &str) -> String {
    svc.effective(key).expect("표에 있는 키")
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "engram-settings-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("시계")
            .as_nanos()
    ))
}

fn names_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_service_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SettingsService>();
}

// ── 적재 · 쓰기 관문 ──

#[test]
fn loading_never_touches_the_disk() {
    for files in [
        MemFiles::default(),
        MemFiles::with_text(r#"{"theme.default":"light"}"#),
        MemFiles::with_text("{broken"),
        MemFiles::with_text(r#"{"theme.default":"purple"}"#),
    ] {
        let before = files.text();
        let svc = SettingsService::load(Box::new(files.clone()));
        svc.get(None).unwrap();
        let disk = files.disk();
        assert_eq!(disk.text, before);
        assert_eq!(disk.writes, 0);
        assert_eq!(disk.copies, 0);
    }
}

#[test]
fn writes_are_refused_until_enabled() {
    let files = MemFiles::with_text("{broken");
    let svc = SettingsService::load(Box::new(files.clone()));

    assert!(matches!(
        svc.set("theme.default", "light"),
        Err(SettingsError::Internal(_))
    ));
    assert!(matches!(
        svc.reset("chat.style."),
        Err(SettingsError::Internal(_))
    ));
    // 인자 오류가 관문보다 먼저다.
    assert!(matches!(
        svc.set("no.such", "x"),
        Err(SettingsError::NotFound(_))
    ));
    assert!(matches!(
        svc.set("theme.default", "purple"),
        Err(SettingsError::InvalidArgument(_))
    ));
    {
        let disk = files.disk();
        assert_eq!(disk.writes, 0);
        assert_eq!(disk.copies, 0, "관문 전엔 떠 두지도 않는다");
    }
    assert_eq!(svc.get(None).unwrap().rev, 0);

    svc.enable_writes();
    assert!(svc.set("theme.default", "light").unwrap().changed);
    assert_eq!(files.disk().writes, 1);
}

// ── set ──

#[test]
fn set_writes_the_canonical_value_and_bumps_rev() {
    let files = MemFiles::default();
    let svc = service(&files);
    let out = svc.set("chat.style.fontSize", " 15.0PX ").unwrap();
    assert_eq!(
        out,
        SetOutcome {
            rev: 1,
            key: "chat.style.fontSize".into(),
            value: "15px".into(),
            changed: true,
        }
    );
    assert_eq!(value_of(&svc, "chat.style.fontSize"), "15px");
    assert_eq!(
        files.json(),
        serde_json::json!({"$version": 1, "chat.style.fontSize": "15px"})
    );
}

#[test]
fn the_same_value_is_not_written() {
    let files = MemFiles::default();
    let svc = service(&files);
    svc.set("theme.default", "light").unwrap();
    let out = svc.set("theme.default", "LIGHT").unwrap();
    assert!(!out.changed);
    assert_eq!(out.rev, 1);
    assert_eq!(out.value, "light");
    assert_eq!(files.disk().writes, 1);
}

#[test]
fn setting_the_default_on_a_default_key_is_not_written() {
    let files = MemFiles::default();
    let svc = service(&files);
    let out = svc.set("theme.default", "dark").unwrap();
    assert!(!out.changed);
    assert_eq!(out.rev, 0);
    assert_eq!(files.disk().writes, 0);
    assert_eq!(files.text(), None, "파일을 만들지 않는다");
}

#[test]
fn setting_the_default_value_removes_the_key_from_the_file() {
    let files = MemFiles::with_text(r#"{"$version":1,"theme.default":"light","x.y":1}"#);
    let svc = service(&files);
    let out = svc.set("theme.default", "Dark").unwrap();
    assert!(out.changed);
    assert_eq!(out.value, "dark");
    assert_eq!(files.json(), serde_json::json!({"$version": 1, "x.y": 1}));
    let item = &svc.get(Some("theme.default")).unwrap().items[0];
    assert!(item.is_default);
}

#[test]
fn set_argument_errors() {
    let files = MemFiles::default();
    let svc = service(&files);
    assert!(matches!(
        svc.set("theme.unknown", "dark"),
        Err(SettingsError::NotFound(_))
    ));
    assert!(matches!(
        svc.set("chat.style.", "1px"),
        Err(SettingsError::InvalidArgument(_))
    ));
    match svc.set("chat.style.fontSize", "48rem") {
        Err(SettingsError::InvalidArgument(msg)) => {
            assert!(msg.starts_with("chat.style.fontSize: "), "{msg}");
            assert!(msg.contains("범위"), "{msg}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(files.disk().writes, 0);
}

#[test]
fn a_failed_disk_write_leaves_memory_rev_and_file_unchanged() {
    let original = r#"{"$version":1,"theme.default":"light"}"#;
    let files = MemFiles::with_text(original);
    let svc = service(&files);
    files.disk().fail_write = true;

    assert!(matches!(
        svc.set("theme.default", "e-ink"),
        Err(SettingsError::Internal(_))
    ));
    assert!(matches!(
        svc.reset("theme.default"),
        Err(SettingsError::Internal(_))
    ));
    assert_eq!(value_of(&svc, "theme.default"), "light");
    assert_eq!(svc.get(None).unwrap().rev, 0);
    assert_eq!(files.text().as_deref(), Some(original));

    files.disk().fail_write = false;
    assert_eq!(svc.set("theme.default", "e-ink").unwrap().rev, 1);
}

// ── 파일 관용 ──

#[test]
fn unknown_keys_survive_every_write() {
    let files = MemFiles::with_text(r#"{"$version":1,"future.key":{"deep":[1,2]},"zz":"keep"}"#);
    let svc = service(&files);
    svc.set("theme.default", "light").unwrap();
    svc.set("chat.style.userPy", "9px").unwrap();
    svc.reset("chat.style.").unwrap();
    let doc = files.json();
    assert_eq!(doc["future.key"], serde_json::json!({"deep": [1, 2]}));
    assert_eq!(doc["zz"], "keep");
    assert_eq!(doc["theme.default"], "light");
}

#[test]
fn an_unusable_file_is_untouched_until_the_first_write_then_copied_aside() {
    let files = MemFiles::with_text("{not json");
    let svc = service(&files);
    assert_eq!(value_of(&svc, "theme.default"), "dark");
    assert_eq!(files.text().as_deref(), Some("{not json"));

    svc.set("theme.default", "light").unwrap();
    assert_eq!(files.disk().corrupt.as_deref(), Some("{not json"));
    assert_eq!(
        files.json(),
        serde_json::json!({"$version": 1, "theme.default": "light"})
    );

    // 다음 쓰기는 멀쩡한 파일 위다 — 다시 뜨면 하나뿐인 사본이 새 파일로 덮인다.
    svc.set("chat.style.userPy", "9px").unwrap();
    let disk = files.disk();
    assert_eq!(disk.copies, 1);
    assert_eq!(disk.corrupt.as_deref(), Some("{not json"));
}

#[test]
fn rmw_keeps_a_key_written_by_someone_else_between_writes() {
    let files = MemFiles::default();
    let svc = service(&files);
    svc.set("theme.default", "light").unwrap();

    // 앱 밖의 편집자가 다른 키를 더한다.
    let mut doc = files.json();
    doc["chat.style.fontSize"] = "20px".into();
    files.disk().text = Some(doc.to_string());

    svc.set("chat.style.userPy", "9px").unwrap();
    assert_eq!(
        files.json(),
        serde_json::json!({
            "$version": 1,
            "theme.default": "light",
            "chat.style.fontSize": "20px",
            "chat.style.userPy": "9px",
        })
    );
    // 실행 중엔 반영되지 않는다 — 다음 적재에 읽힌다.
    assert_eq!(value_of(&svc, "chat.style.fontSize"), "13px");
    let reloaded = SettingsService::load(Box::new(files.clone()));
    assert_eq!(value_of(&reloaded, "chat.style.fontSize"), "20px");
}

#[test]
fn rev_rises_by_one_only_on_actual_writes() {
    let files = MemFiles::default();
    let svc = service(&files);
    let mut revs = vec![svc.get(None).unwrap().rev];
    revs.push(svc.set("theme.default", "light").unwrap().rev);
    revs.push(svc.set("theme.default", "light").unwrap().rev);
    revs.push(svc.set("chat.style.userPy", "9px").unwrap().rev);
    revs.push(svc.reset("theme.").unwrap().rev);
    revs.push(svc.reset("theme.").unwrap().rev);
    revs.push(svc.set("chat.style.userPy", "7px").unwrap().rev);
    assert_eq!(revs, vec![0, 1, 1, 2, 3, 3, 4]);
    assert_eq!(files.disk().writes, 4);
}

#[test]
fn concurrent_writes_to_different_keys_all_land() {
    let files = MemFiles::default();
    let svc = std::sync::Arc::new(service(&files));
    let keys = [
        "chat.style.userPy",
        "chat.style.userPx",
        "chat.style.userMy",
    ];
    let handles: Vec<_> = keys
        .into_iter()
        .map(|key| {
            let svc = svc.clone();
            std::thread::spawn(move || svc.set(key, "11px").unwrap())
        })
        .collect();
    let mut revs: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap().rev).collect();
    revs.sort_unstable();
    assert_eq!(revs, vec![1, 2, 3]);
    let doc = files.json();
    for key in keys {
        assert_eq!(doc[key], "11px", "{key}");
    }
}

// ── reset ──

#[test]
fn reset_by_prefix_lists_every_key_but_only_reports_actual_changes() {
    let files = MemFiles::with_text(
        r#"{"$version":1,"theme.default":"light","chat.style.userPy":"9px","chat.style.fontSize":"bad","x":1}"#,
    );
    let svc = service(&files);
    let out = svc.reset("chat.style.").unwrap();
    assert_eq!(out.rev, 1);
    assert_eq!(out.reset.len(), 11);
    assert_eq!(out.reset[0], "chat.style.railRowPt", "표 순서");
    assert_eq!(
        out.changed,
        vec![SettingItem {
            key: "chat.style.userPy".into(),
            value: "7px".into(),
            is_default: true,
        }]
    );
    // 접혀 있던 못 쓸 값도 함께 지운다. 범위 밖 키는 그대로.
    assert_eq!(
        files.json(),
        serde_json::json!({"$version": 1, "theme.default": "light", "x": 1})
    );
}

#[test]
fn reset_with_nothing_in_memory_or_on_disk_does_not_write() {
    let files = MemFiles::with_text(r#"{"x":1}"#);
    let svc = service(&files);
    let out = svc.reset("chat.style.fontSize").unwrap();
    assert_eq!(out.rev, 0);
    assert_eq!(out.reset, vec!["chat.style.fontSize".to_string()]);
    assert!(out.changed.is_empty());
    assert_eq!(files.disk().writes, 0);
}

#[test]
fn reset_needs_a_matching_selector() {
    let files = MemFiles::default();
    let svc = service(&files);
    for selector in ["", ".", "chat.style", "no.such.", "THEME.DEFAULT"] {
        assert!(
            matches!(svc.reset(selector), Err(SettingsError::NotFound(_))),
            "{selector:?}"
        );
    }
}

// ── get · schema · effective ──

#[test]
fn get_returns_items_in_table_order_with_default_flags() {
    let files = MemFiles::with_text(r#"{"chat.style.lineHeight":"1.6"}"#);
    let svc = service(&files);
    let all = svc.get(None).unwrap();
    assert_eq!(all.rev, 0);
    assert_eq!(all.items.len(), 12);
    assert_eq!(all.items[0].key, "theme.default");
    let line = all
        .items
        .iter()
        .find(|i| i.key == "chat.style.lineHeight")
        .unwrap();
    assert_eq!((line.value.as_str(), line.is_default), ("1.6", false));
    assert!(all
        .items
        .iter()
        .filter(|i| i.key != "chat.style.lineHeight")
        .all(|i| i.is_default));

    assert_eq!(svc.get(Some("chat.style.")).unwrap().items.len(), 11);
    assert_eq!(svc.get(Some("theme.default")).unwrap().items.len(), 1);
    assert!(matches!(
        svc.get(Some("nope")),
        Err(SettingsError::NotFound(_))
    ));
}

#[test]
fn schema_describes_each_key() {
    let svc = service(&MemFiles::default());
    assert_eq!(svc.schema(None).unwrap().len(), 12);
    let theme = svc.schema(Some("theme.default")).unwrap();
    assert_eq!(
        theme,
        vec![SchemaItem {
            key: "theme.default".into(),
            kind: "choice",
            default: "dark".into(),
            choices: Some(vec!["dark".into(), "light".into(), "e-ink".into()]),
            min: None,
            max: None,
            description: "창별 덮어쓰기가 없는 창의 테마".into(),
        }]
    );
    let font = &svc.schema(Some("chat.style.fontSize")).unwrap()[0];
    assert_eq!(font.min.as_deref(), Some("8px, 0.5rem, 0.5em"));
    assert!(matches!(
        svc.schema(Some("chat.")),
        Ok(items) if items.len() == 11
    ));
    assert!(matches!(
        svc.schema(Some("x.")),
        Err(SettingsError::NotFound(_))
    ));
}

#[test]
fn effective_is_none_for_unknown_keys_and_prefixes() {
    let svc = service(&MemFiles::default());
    assert_eq!(svc.effective("theme.default").as_deref(), Some("dark"));
    assert_eq!(svc.effective("chat.style."), None);
    assert_eq!(svc.effective("nope"), None);
}

#[test]
fn outcomes_serialize_with_snake_case_fields() {
    let item = SettingItem {
        key: "theme.default".into(),
        value: "dark".into(),
        is_default: true,
    };
    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        serde_json::json!({"key": "theme.default", "value": "dark", "is_default": true})
    );
}

// ── 알림 진입점(set_and_notify · reset_and_notify) ──

#[derive(Default)]
struct Heard {
    changed: Mutex<Vec<SettingsSnapshot>>,
    /// 알림 순서 — `changed` 다음에 `theme` 이 와야 한다.
    order: Mutex<Vec<&'static str>>,
}

impl SettingsEvents for Heard {
    fn changed(&self, change: &SettingsSnapshot) {
        self.changed.lock().unwrap().push(change.clone());
        self.order.lock().unwrap().push("changed");
    }

    fn theme_default_changed(&self) {
        self.order.lock().unwrap().push("theme");
    }
}

impl Heard {
    fn changes(&self) -> Vec<SettingsSnapshot> {
        self.changed.lock().unwrap().clone()
    }

    fn order(&self) -> Vec<&'static str> {
        self.order.lock().unwrap().clone()
    }
}

#[test]
fn the_theme_key_constant_names_a_key_in_the_table() {
    assert_eq!(
        registry::find(THEME_DEFAULT).map(|d| d.key),
        Some(THEME_DEFAULT)
    );
}

#[test]
fn a_changing_set_is_announced_once_with_its_rev_and_theme_last() {
    let files = MemFiles::default();
    let svc = service(&files);
    let heard = Heard::default();

    let out = set_and_notify(&svc, &heard, "theme.default", "LIGHT").unwrap();

    assert_eq!(out.rev, 1);
    assert_eq!(
        heard.changes(),
        vec![SettingsSnapshot {
            rev: 1,
            items: vec![SettingItem {
                key: "theme.default".to_string(),
                value: "light".to_string(),
                is_default: false,
            }],
        }]
    );
    assert_eq!(heard.order(), vec!["changed", "theme"]);
}

#[test]
fn setting_the_default_value_announces_it_as_default() {
    let files = MemFiles::with_text(r#"{"chat.style.fontSize":"15px"}"#);
    let svc = service(&files);
    let heard = Heard::default();

    set_and_notify(&svc, &heard, "chat.style.fontSize", "13px").unwrap();

    let changes = heard.changes();
    assert_eq!(changes.len(), 1);
    assert!(changes[0].items[0].is_default, "{changes:?}");
    assert_eq!(
        heard.order(),
        vec!["changed"],
        "테마 키가 아니면 테마 밀기는 없다"
    );
}

#[test]
fn a_set_that_changes_nothing_or_fails_is_silent() {
    let files = MemFiles::default();
    let svc = service(&files);
    let heard = Heard::default();

    assert!(
        !set_and_notify(&svc, &heard, "theme.default", "dark")
            .unwrap()
            .changed
    );
    assert!(set_and_notify(&svc, &heard, "no.such", "x").is_err());
    assert!(set_and_notify(&svc, &heard, "theme.default", "purple").is_err());
    files.disk().fail_write = true;
    assert!(set_and_notify(&svc, &heard, "theme.default", "light").is_err());

    assert!(heard.order().is_empty(), "{:?}", heard.order());
}

#[test]
fn a_reset_announces_only_the_keys_it_changed() {
    let files = MemFiles::with_text(
        r#"{"theme.default":"e-ink","chat.style.fontSize":"15px","chat.style.userPx":"4px"}"#,
    );
    let svc = service(&files);
    let heard = Heard::default();

    let out = reset_and_notify(&svc, &heard, "chat.style.").unwrap();

    assert_eq!(out.reset.len(), 11);
    let changes = heard.changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].rev, out.rev);
    let keys: Vec<&str> = changes[0].items.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(keys, vec!["chat.style.userPx", "chat.style.fontSize"]);
    assert_eq!(heard.order(), vec!["changed"]);

    // 이미 다 기본값 — 쓰지도 알리지도 않는다.
    reset_and_notify(&svc, &heard, "chat.style.").unwrap();
    assert_eq!(heard.changes().len(), 1);

    reset_and_notify(&svc, &heard, "theme.").unwrap();
    assert_eq!(heard.order(), vec!["changed", "changed", "theme"]);
}

// ── 실제 디스크 ──

#[test]
fn real_disk_round_trip_creates_the_folder_only_on_the_first_write() {
    let root = temp_dir("roundtrip");
    let dir = root.join("shell").join("config");

    let svc = SettingsService::load_from_dir(&dir);
    assert!(!root.exists(), "적재는 폴더를 만들지 않는다");
    svc.enable_writes();
    svc.set("theme.default", "light").unwrap();

    let path = dir.join("settings.json");
    let text = std::fs::read_to_string(&path).expect("파일");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        serde_json::json!({"$version": 1, "theme.default": "light"})
    );
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["settings.json".to_string()],
        "임시 파일이 남았다"
    );

    let reloaded = SettingsService::load_from_dir(&dir);
    assert_eq!(value_of(&reloaded, "theme.default"), "light");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn real_disk_corrupt_file_is_kept_on_load_and_copied_aside_on_first_write() {
    let dir = temp_dir("corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    let copy = dir.join("settings.json.corrupt");
    std::fs::write(&path, b"\xff\xfe not utf-8").unwrap();
    std::fs::write(&copy, b"earlier copy").unwrap();

    let svc = SettingsService::load_from_dir(&dir);
    assert_eq!(value_of(&svc, "theme.default"), "dark");
    assert_eq!(std::fs::read(&path).unwrap(), b"\xff\xfe not utf-8");

    svc.enable_writes();
    svc.set("theme.default", "e-ink").unwrap();
    assert_eq!(
        names_in(&dir),
        vec!["settings.json", "settings.json.corrupt"]
    );
    assert_eq!(
        std::fs::read(&copy).unwrap(),
        b"\xff\xfe not utf-8",
        "앞서 떠 둔 사본은 덮인다"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
            .unwrap(),
        serde_json::json!({"$version": 1, "theme.default": "e-ink"})
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn real_disk_oversized_file_counts_as_unusable() {
    let dir = temp_dir("oversized");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    let big = format!(
        "{{\"theme.default\":\"light\",\"pad\":\"{}\"}}",
        "x".repeat(64 * 1024)
    );
    std::fs::write(&path, &big).unwrap();

    let svc = SettingsService::load_from_dir(&dir);
    assert_eq!(
        value_of(&svc, "theme.default"),
        "dark",
        "상한 초과는 통째로 못 쓴다"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), big);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn real_disk_large_broken_file_is_copied_aside_whole() {
    let dir = temp_dir("huge");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    let broken = vec![b'x'; 2 * 1024 * 1024];
    std::fs::write(&path, &broken).unwrap();

    let svc = SettingsService::load_from_dir(&dir);
    svc.enable_writes();
    svc.set("theme.default", "light").unwrap();

    assert_eq!(
        names_in(&dir),
        vec!["settings.json", "settings.json.corrupt"]
    );
    assert_eq!(
        std::fs::read(dir.join("settings.json.corrupt")).unwrap(),
        broken
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
            .unwrap(),
        serde_json::json!({"$version": 1, "theme.default": "light"})
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn real_disk_bom_and_float_version_load() {
    let dir = temp_dir("bom");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("settings.json"),
        b"\xef\xbb\xbf{\"$version\": 1.0, \"theme.default\": \"light\"}",
    )
    .unwrap();

    let svc = SettingsService::load_from_dir(&dir);
    assert_eq!(value_of(&svc, "theme.default"), "light");

    std::fs::remove_dir_all(&dir).ok();
}

// ── 못 쓰는 파일 · 없어진 파일 위의 쓰기 ──

#[test]
fn a_write_over_a_file_broken_by_hand_keeps_the_values_memory_shows() {
    let files = MemFiles::with_text(r#"{"theme.default":"light","chat.style.userPy":"9px"}"#);
    let svc = service(&files);
    files.disk().text = Some("{broken".to_string());

    svc.set("chat.style.fontSize", "15px").unwrap();

    assert_eq!(files.disk().corrupt.as_deref(), Some("{broken"));
    assert_eq!(
        files.json(),
        serde_json::json!({
            "$version": 1,
            "theme.default": "light",
            "chat.style.userPy": "9px",
            "chat.style.fontSize": "15px",
        })
    );
}

#[test]
fn a_reset_over_a_broken_file_keeps_the_other_values_memory_shows() {
    let files = MemFiles::with_text(r#"{"theme.default":"light","chat.style.userPy":"9px"}"#);
    let svc = service(&files);
    files.disk().unusable = true;

    let out = svc.reset("theme.").unwrap();

    assert_eq!(out.rev, 1);
    assert_eq!(files.disk().copies, 1);
    assert_eq!(
        files.json(),
        serde_json::json!({"$version": 1, "chat.style.userPy": "9px"})
    );
}

#[test]
fn a_write_after_the_file_was_deleted_keeps_the_values_memory_shows() {
    let files = MemFiles::with_text(r#"{"theme.default":"light","chat.style.userPy":"9px"}"#);
    let svc = service(&files);
    files.disk().text = None;

    svc.set("chat.style.fontSize", "15px").unwrap();

    assert_eq!(files.disk().copies, 0);
    assert_eq!(
        files.json(),
        serde_json::json!({
            "$version": 1,
            "theme.default": "light",
            "chat.style.userPy": "9px",
            "chat.style.fontSize": "15px",
        })
    );
}

#[test]
fn the_file_is_never_absent_when_the_replace_fails_after_the_copy() {
    let files = MemFiles::with_text("{broken");
    let svc = service(&files);
    files.disk().fail_write = true;

    assert!(matches!(
        svc.set("theme.default", "light"),
        Err(SettingsError::Internal(_))
    ));

    let disk = files.disk();
    assert_eq!(disk.text.as_deref(), Some("{broken"));
    assert_eq!(disk.corrupt.as_deref(), Some("{broken"));
    drop(disk);
    assert_eq!(value_of(&svc, "theme.default"), "dark");
    assert_eq!(svc.get(None).unwrap().rev, 0);
}

#[test]
fn a_replace_that_keeps_failing_keeps_one_copy_of_the_broken_file() {
    let files = MemFiles::with_text("{broken");
    let svc = service(&files);
    files.disk().fail_write = true;

    for _ in 0..3 {
        assert!(matches!(
            svc.set("theme.default", "light"),
            Err(SettingsError::Internal(_))
        ));
    }
    assert_eq!(files.disk().corrupt.as_deref(), Some("{broken"));

    files.disk().fail_write = false;
    svc.set("theme.default", "light").unwrap();
    svc.set("chat.style.userPy", "9px").unwrap();
    assert_eq!(
        files.disk().corrupt.as_deref(),
        Some("{broken"),
        "갈아끼운 뒤의 쓰기가 사본을 새 파일로 덮지 않는다"
    );
    assert_eq!(
        files.json(),
        serde_json::json!({"$version": 1, "theme.default": "light", "chat.style.userPy": "9px"})
    );
}

#[test]
fn a_failed_copy_aside_fails_the_write_and_changes_nothing() {
    let files = MemFiles::with_text("{broken");
    let svc = service(&files);
    files.disk().fail_copy = true;

    assert!(matches!(
        svc.set("theme.default", "light"),
        Err(SettingsError::Internal(_))
    ));

    assert_eq!(files.text().as_deref(), Some("{broken"));
    assert_eq!(files.disk().writes, 0);
    assert_eq!(value_of(&svc, "theme.default"), "dark");
    assert_eq!(svc.get(None).unwrap().rev, 0);
}

#[test]
fn a_write_that_would_exceed_the_read_cap_is_refused() {
    let pad = "x".repeat(64 * 1024 - 20);
    let original = format!("{{\"pad\":\"{pad}\"}}");
    let files = MemFiles::with_text(&original);
    let svc = service(&files);

    assert!(matches!(
        svc.set("theme.default", "light"),
        Err(SettingsError::Internal(_))
    ));

    assert_eq!(files.text().as_deref(), Some(original.as_str()));
    assert_eq!(value_of(&svc, "theme.default"), "dark");
    assert_eq!(svc.get(None).unwrap().rev, 0);
}

// ── 쓸지는 파일로, changed 는 유효 값으로 ──

#[test]
fn set_rewrites_a_file_that_disagrees_without_announcing() {
    let files = MemFiles::default();
    let svc = service(&files);
    // 앱 밖의 편집 — 메모리는 기본값 그대로다.
    files.disk().text = Some(r#"{"theme.default":"light","x":1}"#.to_string());
    let heard = Heard::default();

    let out = set_and_notify(&svc, &heard, "theme.default", "dark").unwrap();

    assert!(!out.changed);
    assert_eq!(out.rev, 0);
    assert!(heard.order().is_empty(), "{:?}", heard.order());
    assert_eq!(files.json(), serde_json::json!({"$version": 1, "x": 1}));
    assert_eq!(files.disk().writes, 1);
    assert_eq!(svc.get(None).unwrap().rev, 0);
}

#[test]
fn set_rewrites_a_non_canonical_spelling_without_announcing() {
    let files = MemFiles::with_text(r#"{"theme.default":"LIGHT"}"#);
    let svc = service(&files);
    let heard = Heard::default();

    let out = set_and_notify(&svc, &heard, "theme.default", "light").unwrap();

    assert!(!out.changed);
    assert!(heard.order().is_empty());
    assert_eq!(
        files.json(),
        serde_json::json!({"$version": 1, "theme.default": "light"})
    );
}

#[test]
fn reset_clears_a_folded_bad_value_without_announcing() {
    let files = MemFiles::with_text(r#"{"theme.default":"purple","x":1}"#);
    let svc = service(&files);
    let heard = Heard::default();

    let out = reset_and_notify(&svc, &heard, "theme.default").unwrap();

    assert_eq!(out.rev, 0);
    assert!(out.changed.is_empty());
    assert!(heard.order().is_empty());
    assert_eq!(files.json(), serde_json::json!({"$version": 1, "x": 1}));
    assert_eq!(files.disk().writes, 1);
}

#[test]
fn a_change_the_file_already_holds_is_committed_without_a_write() {
    let files = MemFiles::with_text(r#"{"theme.default":"light"}"#);
    let svc = service(&files);
    // 밖에서 그 키를 지웠다 — 파일은 이미 목표(기본값)다.
    files.disk().text = Some(r#"{"$version":1}"#.to_string());
    let heard = Heard::default();

    let out = set_and_notify(&svc, &heard, "theme.default", "dark").unwrap();

    assert!(out.changed);
    assert_eq!(out.rev, 1);
    assert_eq!(heard.order(), vec!["changed", "theme"]);
    assert_eq!(files.disk().writes, 0);
    assert_eq!(value_of(&svc, "theme.default"), "dark");
}

#[test]
fn an_unreadable_file_falls_back_to_memory_for_the_write_decision() {
    let files = MemFiles::with_text(r#"{"theme.default":"light"}"#);
    let svc = service(&files);
    files.disk().fail_read = true;

    let same = svc.set("theme.default", "light").unwrap();
    assert!(!same.changed);
    assert!(svc.reset("chat.style.").unwrap().changed.is_empty());
    assert_eq!(files.disk().writes, 0);

    let err = svc.set("theme.default", "e-ink").unwrap_err();
    assert!(
        matches!(&err, SettingsError::Internal(message) if message.contains("못 읽어") && !message.contains("못 썼다")),
        "읽기 실패를 쓰기 실패로 말하지 않는다: {err:?}"
    );
    assert_eq!(value_of(&svc, "theme.default"), "light");
}

#[test]
fn an_unusable_file_is_not_rewritten_for_a_no_op() {
    let files = MemFiles::with_text("{broken");
    let svc = service(&files);

    assert!(!svc.set("theme.default", "dark").unwrap().changed);
    assert!(svc.reset("theme.").unwrap().changed.is_empty());

    let disk = files.disk();
    assert_eq!(disk.writes, 0);
    assert_eq!(disk.copies, 0);
    assert_eq!(disk.text.as_deref(), Some("{broken"));
}

// ── 새 판이 쓴 파일(ADR-0291 R2) ──

/// 새 판이 쓴 파일 위의 쓰기는 `Conflict` 이고 덮지도 떠 두지도 않는다 — 메모리 · `rev` · 알림 그대로.
#[test]
fn a_newer_version_file_refuses_writes_with_conflict_and_is_left_alone() {
    let text = r#"{"$version":2,"theme.default":"light"}"#;
    let files = MemFiles::with_text(text);
    let svc = service(&files);
    let heard = Heard::default();
    assert_eq!(
        value_of(&svc, "theme.default"),
        "dark",
        "새 판은 읽지 않는다"
    );

    assert!(matches!(
        set_and_notify(&svc, &heard, "theme.default", "e-ink"),
        Err(SettingsError::Conflict(_))
    ));
    // 유효 값이 안 바뀌는 호출은 파일을 가를 수 없어 쓰지 않는다 — 거절도 아니다.
    assert!(!svc.set("chat.style.fontSize", "13px").unwrap().changed);
    assert!(reset_and_notify(&svc, &heard, "chat.style.")
        .unwrap()
        .changed
        .is_empty());

    let disk = files.disk();
    assert_eq!(disk.text.as_deref(), Some(text));
    assert_eq!((disk.writes, disk.copies), (0, 0));
    drop(disk);
    assert!(heard.order().is_empty(), "{:?}", heard.order());
    assert_eq!(value_of(&svc, "theme.default"), "dark");
}

/// 실행 중에 새 판으로 바뀐 파일도 쓰기마다 다시 읽어 거절하고, 치워지면 다음 쓰기가 풀린다(ADR-0291 R16).
#[test]
fn a_file_that_turns_newer_while_running_is_refused_until_it_is_replaced() {
    let files = MemFiles::with_text(r#"{"theme.default":"light"}"#);
    let svc = service(&files);
    let newer = r#"{"$version":2}"#;
    files.disk().text = Some(newer.to_string());

    assert!(matches!(
        svc.reset("theme."),
        Err(SettingsError::Conflict(_))
    ));
    assert_eq!(value_of(&svc, "theme.default"), "light");
    assert_eq!(files.text().as_deref(), Some(newer));

    files.disk().text = None;
    assert!(svc.set("theme.default", "e-ink").unwrap().changed);
    assert_eq!(
        files.json(),
        serde_json::json!({"$version": 1, "theme.default": "e-ink"})
    );
}

// ── 락 — 읽기는 쓰기를 기다리지 않고, 알림은 rev 순서로 나간다 ──

/// 쓰기마다 `entered` 로 알리고 `release` 에서 하나를 받을 때까지 멈추는 디스크(송신단이 사라지면 멈추지
/// 않는다).
struct GatedFiles {
    inner: MemFiles,
    entered: Sender<()>,
    release: Receiver<()>,
}

impl SettingsFiles for GatedFiles {
    fn read(&self) -> io::Result<String> {
        self.inner.read()
    }

    fn write_atomic(&self, text: &str) -> io::Result<()> {
        let _ = self.entered.send(());
        let _ = self.release.recv();
        self.inner.write_atomic(text)
    }

    fn copy_aside(&self) -> io::Result<std::path::PathBuf> {
        self.inner.copy_aside()
    }

    fn origin(&self) -> String {
        self.inner.origin()
    }
}

/// (쓰기에 들어섰다는 신호, 쓰기를 풀어 줄 손잡이, 쓰기가 열린 서비스).
fn gated_service() -> (Receiver<()>, Sender<()>, Arc<SettingsService>) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let svc = SettingsService::load(Box::new(GatedFiles {
        inner: MemFiles::default(),
        entered: entered_tx,
        release: release_rx,
    }));
    svc.enable_writes();
    (entered_rx, release_tx, Arc::new(svc))
}

#[test]
fn readers_do_not_wait_for_a_write_in_flight() {
    let (entered, release, svc) = gated_service();
    let writer = {
        let svc = Arc::clone(&svc);
        std::thread::spawn(move || svc.set("theme.default", "light").unwrap())
    };
    entered.recv().unwrap();

    let (read_tx, read_rx) = mpsc::channel();
    {
        let svc = Arc::clone(&svc);
        std::thread::spawn(move || {
            let snapshot = svc.get(Some("theme.default")).unwrap();
            let effective = svc.effective("theme.default");
            let _ = read_tx.send((snapshot, effective));
        });
    }
    // 실패할 때만 닿는 상한이다 — 읽기가 쓰기 뒤에 줄 서면 여기서 끊고 쓰기를 풀어 준다.
    let read = read_rx.recv_timeout(Duration::from_secs(10));
    release.send(()).unwrap();
    let (snapshot, effective) = read.expect("읽기가 진행 중인 쓰기의 디스크 대기를 기다렸다");

    assert_eq!(snapshot.rev, 0);
    assert_eq!(snapshot.items[0].value, "dark");
    assert_eq!(effective.as_deref(), Some("dark"));
    assert_eq!(writer.join().unwrap().rev, 1);
    assert_eq!(svc.get(None).unwrap().rev, 1);
}

/// `changed` 안에서 서비스를 다시 읽고, 그때 어느 락이 잡혀 있는지 적는다.
struct ReadingEvents {
    svc: Arc<SettingsService>,
    /// (알림 rev, 그 자리의 `get` rev, 그 자리의 유효 값, 알림 순서 락이 잡혀 있었나, 쓰기 직렬화 락이 잡혀
    /// 있었나).
    seen: Mutex<Vec<(u64, u64, Option<String>, bool, bool)>>,
    /// 테마 밀기 자리에서 (알림 순서 락, 쓰기 직렬화 락, 상태 락)이 잡혀 있었나.
    theme: Mutex<Vec<(bool, bool, bool)>>,
}

impl SettingsEvents for ReadingEvents {
    fn changed(&self, change: &SettingsSnapshot) {
        let snapshot = self.svc.get(None).unwrap();
        let effective = self.svc.effective(&change.items[0].key);
        let announce_held = self.svc.announce.try_lock().is_err();
        let io_held = self.svc.io.try_lock().is_err();
        self.seen.lock().unwrap().push((
            change.rev,
            snapshot.rev,
            effective,
            announce_held,
            io_held,
        ));
    }

    fn theme_default_changed(&self) {
        let held = (
            self.svc.announce.try_lock().is_err(),
            self.svc.io.try_lock().is_err(),
            self.svc.state.try_lock().is_err(),
        );
        self.theme.lock().unwrap().push(held);
    }
}

#[test]
fn a_notification_may_read_the_service_and_sees_its_own_write() {
    let svc = Arc::new(service(&MemFiles::default()));
    let events = ReadingEvents {
        svc: Arc::clone(&svc),
        seen: Mutex::default(),
        theme: Mutex::default(),
    };

    set_and_notify(&svc, &events, "theme.default", "light").unwrap();
    reset_and_notify(&svc, &events, "theme.").unwrap();

    assert_eq!(
        *events.seen.lock().unwrap(),
        vec![
            (1, 1, Some("light".to_string()), true, false),
            (2, 2, Some("dark".to_string()), true, false),
        ],
        "알림은 알림 순서 락만 쥔 채 나가고, 그 자리에서 읽으면 자기 쓰기가 보인다"
    );
    assert_eq!(
        *events.theme.lock().unwrap(),
        vec![(false, false, false); 2],
        "테마 밀기는 락을 하나도 쥐지 않은 채 불린다"
    );
}

/// 첫 알림 안에서 `hold` 를 받을 때까지 멈추고(들어섰다고 `in_first` 로 알린다), 알림 rev 를 적는다.
struct HoldingEvents {
    heard: Mutex<Vec<u64>>,
    in_first: Mutex<Option<Sender<()>>>,
    hold: Mutex<Option<Receiver<()>>>,
}

impl SettingsEvents for HoldingEvents {
    fn changed(&self, change: &SettingsSnapshot) {
        let hold = self.hold.lock().unwrap().take();
        if let Some(hold) = hold {
            if let Some(in_first) = self.in_first.lock().unwrap().take() {
                let _ = in_first.send(());
            }
            let _ = hold.recv();
        }
        self.heard.lock().unwrap().push(change.rev);
    }

    fn theme_default_changed(&self) {}
}

/// ★회귀를 매번 잡지는 않는다★ — 순서 락이 빠지면 B 의 알림이 A 를 풀기 전에 나갈 수도, 뒤에 나갈 수도
/// 있다(스케줄러 몫). 고친 코드에서는 언제나 초록이다.
#[test]
fn notifications_leave_in_rev_order_across_writers() {
    let (entered, release, svc) = gated_service();
    release.send(()).unwrap();
    release.send(()).unwrap();
    let (in_first_tx, in_first_rx) = mpsc::channel();
    let (hold_tx, hold_rx) = mpsc::channel();
    let events = Arc::new(HoldingEvents {
        heard: Mutex::default(),
        in_first: Mutex::new(Some(in_first_tx)),
        hold: Mutex::new(Some(hold_rx)),
    });
    let write = |key: &'static str| {
        let svc = Arc::clone(&svc);
        let events = Arc::clone(&events);
        std::thread::spawn(move || set_and_notify(&svc, &*events, key, "9px").unwrap())
    };

    // A 가 rev 1 을 확정하고 그 알림 안에서 멈춘다.
    let first = write("chat.style.userPy");
    entered.recv().unwrap();
    in_first_rx.recv().unwrap();
    // 그사이 B 가 디스크 쓰기에 들어선다 — B 의 확정 · 알림은 A 의 알림이 끝날 때까지 못 나간다.
    let second = write("chat.style.userPx");
    entered.recv().unwrap();
    hold_tx.send(()).unwrap();

    assert_eq!(first.join().unwrap().rev, 1);
    assert_eq!(second.join().unwrap().rev, 2);
    assert_eq!(*events.heard.lock().unwrap(), vec![1, 2]);
}

// ── 적재 로그 — 로거가 선 뒤(enable_writes)에 나간다 ──

#[derive(Debug, Clone, PartialEq)]
struct Line {
    level: tracing::Level,
    message: String,
    fields: Vec<(String, String)>,
}

/// 이 스레드에서 낸 이벤트를 전부 모은다.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<Line>>>);

impl Captured {
    fn lines(&self) -> Vec<Line> {
        self.0.lock().unwrap().clone()
    }
}

struct Fields<'a>(&'a mut Line);

impl tracing::field::Visit for Fields<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}");
        if field.name() == "message" {
            self.0.message = text;
        } else {
            self.0.fields.push((field.name().to_string(), text));
        }
    }
}

impl tracing::Subscriber for Captured {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = Line {
            level: *event.metadata().level(),
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut Fields(&mut line));
        self.0.lock().unwrap().push(line);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

fn field<'a>(line: &'a Line, name: &str) -> Option<&'a str> {
    line.fields
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.as_str())
}

#[test]
fn load_notes_wait_for_enable_writes_and_carry_no_values() {
    let files = MemFiles::with_text(
        r#"{"theme.default":"purple","chat.style.userPy":"9px","future.key":"secret"}"#,
    );
    let captured = Captured::default();

    let svc = tracing::subscriber::with_default(captured.clone(), || {
        SettingsService::load(Box::new(files.clone()))
    });
    assert!(
        captured.lines().is_empty(),
        "적재는 로그를 내지 않는다(로거가 아직 없다): {:?}",
        captured.lines()
    );

    tracing::subscriber::with_default(captured.clone(), || {
        svc.enable_writes();
        svc.enable_writes();
    });

    let lines = captured.lines();
    let warns: Vec<&Line> = lines
        .iter()
        .filter(|line| line.level == tracing::Level::WARN)
        .collect();
    assert_eq!(warns.len(), 1, "{lines:?}");
    assert_eq!(field(warns[0], "key"), Some("theme.default"));
    assert_eq!(field(warns[0], "source"), Some("memory"));
    assert_eq!(field(warns[0], "module"), Some("settings"));
    let summary: Vec<&Line> = lines
        .iter()
        .filter(|line| line.level == tracing::Level::DEBUG)
        .collect();
    assert_eq!(
        summary.len(),
        1,
        "두 번째 부름은 다시 내지 않는다: {lines:?}"
    );
    assert_eq!(field(summary[0], "overrides"), Some("1"));
    assert_eq!(field(summary[0], "unknown"), Some("1"));
    let all = format!("{lines:?}");
    assert!(!all.contains("purple") && !all.contains("secret"), "{all}");
}

#[test]
fn an_unusable_file_is_reported_as_an_error_once_writes_are_enabled() {
    let svc = SettingsService::load(Box::new(MemFiles::with_text("{broken")));
    let captured = Captured::default();

    tracing::subscriber::with_default(captured.clone(), || svc.enable_writes());

    let errors: Vec<Line> = captured
        .lines()
        .into_iter()
        .filter(|line| line.level == tracing::Level::ERROR)
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", captured.lines());
    assert_eq!(field(&errors[0], "source"), Some("memory"));
}

#[test]
fn a_copied_aside_file_is_logged_as_a_warning_with_where_it_went() {
    let files = MemFiles::with_text("{broken");
    let svc = service(&files);
    let captured = Captured::default();

    tracing::subscriber::with_default(captured.clone(), || {
        svc.set("theme.default", "light").unwrap();
    });

    let lines = captured.lines();
    let warns: Vec<&Line> = lines
        .iter()
        .filter(|line| line.level == tracing::Level::WARN)
        .collect();
    assert_eq!(warns.len(), 1, "{lines:?}");
    assert_eq!(field(warns[0], "copied_to"), Some("memory.corrupt"));
    assert_eq!(field(warns[0], "source"), Some("memory"));
    assert!(
        !lines.iter().any(|line| line.level == tracing::Level::ERROR),
        "{lines:?}"
    );
}

// ── 격리 — 이 모듈은 Tauri 도 async 런타임도 모른다(그래서 앱 없이 시험이 돈다) ──

#[test]
fn the_settings_module_names_neither_tauri_nor_tokio() {
    // 바늘을 쪼개 적는다 — 이 파일도 훑는 대상이다.
    let needles = [
        concat!("tau", "ri::"),
        concat!("tok", "io::"),
        concat!("use ", "tauri"),
        concat!("use ", "tokio"),
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("settings");
    let mut pending = vec![root];
    let mut scanned = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("settings 폴더") {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for needle in needles {
                    assert!(
                        !code.contains(needle),
                        "{}:{}: `{needle}` — 설정 모듈은 셸 런타임을 모른다",
                        path.display(),
                        number + 1
                    );
                }
            }
            scanned.push(path);
        }
    }
    assert!(
        scanned.iter().any(|path| path.ends_with("mod.rs")) && scanned.len() >= 4,
        "훑은 파일이 없다 — 폴더가 옮겨졌나: {scanned:?}"
    );
}
