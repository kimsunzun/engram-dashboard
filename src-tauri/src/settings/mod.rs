//! 셸 설정 서비스 — 스키마 표([`registry`]) 와 파일([`store`]) 위의 단일 진입점. 사람 경로(Tauri 명령)와
//! LLM 경로(버스 명령)가 같은 [`SettingsService`] 를 부른다(TRD §5).
//!
//! - **값은 언제나 정규 문자열이다**(TRD §5-2) — `get` · `set` 답 · 알림이 같은 철자를 싣는다.
//! - **적재는 읽기 전용이다** — [`SettingsService::load`] 는 파일을 만들지도 고치지도 않고 로그도 내지 않는다
//!   (로거보다 먼저 돈다). 쓰기와 적재 로그는 [`SettingsService::enable_writes`] 뒤에만 나간다(셸 `setup` 이
//!   로거를 세운 뒤 부른다 — 빌드 전 적재가 디스크를 바꾸지 않게).
//! - **락 셋 — 순서 = 쓰기 직렬화(`io`) → 알림 순서(`announce`) → 상태(`state`)**:
//!   - `io` = 쓰기끼리 줄 세우는 락 — 디스크 손잡이를 쥔다. 읽고-고치고-쓰기와 `sync_all` 동안 쥔다.
//!   - `state` = 메모리 값 · `rev` 의 짧은 락. **잎이다** — 쥔 채 다른 락 · IO · emit 을 하지 않는다. 읽기
//!     (`get` · `effective` · 테마 밀기)는 이것만 잡으므로 진행 중인 쓰기의 `sync_all` 을 기다리지 않는다.
//!   - `announce` = 확정(`rev` 발급)과 그 알림을 한 덩이로 묶는다 — 알림 순서 = `rev` 순서. 알림 중에 쥔 락은
//!     이것 하나다(`io` 는 그 전에 놓는다).
//!   - ★`state` 를 쥔 채 `io` 를 잡지 말 것★ — 읽기가 다시 쓰기의 디스크 대기 뒤에 줄 선다. 테마 관문
//!     (`theme::EffectiveThemes`)은 `state` 만 짧게 잡고, 설정 쓰기는 모든 락을 놓은 뒤에야 테마 밀기를
//!     부르므로 거꾸로 잡는 길이 없다.
//! - **알림 포트는 셸의 쓰기 진입점([`set_and_notify`] · [`reset_and_notify`])이 준다** — 서비스는 확정 직후
//!   그 진입점의 콜백을 `announce` 아래서 부를 뿐 Tauri 도 emit 도 모른다. 그래서 앱 없이 시험이 돈다.
//! - `rev` = 적재 때 0, **유효 값이 실제로 바뀐 호출**마다 1 씩 오른다. 파일만 바로잡은 호출(접힌 못 쓸 값 ·
//!   밖에서 고친 값)은 안 올린다.
//! - 앱 밖에서 고친 값은 실행 중에 반영되지 않는다 — 다음 적재에 읽힌다(쓰기는 그 키를 쓰거나 되돌릴 때만
//!   그 값을 덮는다).

mod registry;
mod store;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;
use ts_rs::TS;

use registry::SettingDef;
pub use store::SettingsFiles;

/// 창별 덮어쓰기가 없는 창의 테마 — 이 키가 바뀌면 셸이 모든 창의 유효 테마를 다시 민다
/// ([`SettingsEvents::theme_default_changed`]).
pub const THEME_DEFAULT: &str = "theme.default";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// 모르는 키, 또는 맞는 키가 하나도 없는 접두.
    #[error("{0}")]
    NotFound(String),
    /// 형식 · 범위 위반(문구에 기대 형식), 또는 정확한 키 자리에 온 접두.
    #[error("{0}")]
    InvalidArgument(String),
    /// 이 셸보다 새 판이 쓴 설정 파일이라 덮지 않았다(메모리 · `rev` 불변 · 알림 없음 · 파일 그대로). 그 파일이
    /// 바뀌기 전에는 다시 해도 같다.
    // ADR-0291 R2
    #[error("{0}")]
    Conflict(String),
    /// 쓰기 직전에 지금 파일을 못 읽어 덮지 않았다(메모리 · `rev` 불변 · 알림 없음 · 파일 그대로). 쓸 때마다 다시
    /// 읽으므로 대개 잠깐 쥔 잠김이고 조금 뒤 다시 하면 된다.
    // ADR-0291 R17 — 사용자 결정 2026-10-11: 실행 중 재판정의 못 읽음은 일시 거절이다.
    #[error("{0}")]
    Unreadable(String),
    /// 디스크 쓰기 실패(메모리 · `rev` 불변 · 알림 없음), 또는 [`SettingsService::enable_writes`] 전의 쓰기.
    #[error("{0}")]
    Internal(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SettingItem {
    pub key: String,
    /// 정규형. 덮어쓴 값이 없으면 기본값.
    pub value: String,
    /// 덮어쓴 값이 없다. 파일의 못 쓸 값이 기본값으로 접힌 키도 `true` 다.
    pub is_default: bool,
}

/// `settings_get` 의 답이자 `settings:changed` 알림의 짐 — 알림이면 `items` 는 바뀐 키만이다.
///
/// ★받는 쪽은 **키마다** `rev` 로 적용한다★ — 항목은 그 키를 처음 보거나, 그 키에 마지막으로 적용한 `rev`
/// 보다 클 때 적용한다. ★처음 보는 키는 그대로 적용한다★ — 셸을 띄운 직후의 답은 `rev` 0 이라, 「0 보다
/// 클 때」로 비교하면 첫 값을 버린다. `settings_get` 답도 항목마다 답의 `rev` 로 같은 규칙을 탄다: 답을
/// 통째로 버리면 그 답에만 있는 키를 잃고, 통째로 적용하면 먼저 도착한 새 알림을 옛 값으로 덮는다. 알림끼리는
/// `rev` 순서로 나간다.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SettingsSnapshot {
    // ts-rs 의 u64 기본 매핑은 bigint 이나 serde_json 은 number 로 싣는다.
    #[ts(type = "number")]
    pub rev: u64,
    /// 표 순서.
    pub items: Vec<SettingItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SetOutcome {
    /// `changed: false` 면 호출 전과 같은 값.
    #[ts(type = "number")]
    pub rev: u64,
    pub key: String,
    /// 정규형.
    pub value: String,
    /// `false` = 유효 값이 이미 그 값이었다 — 알림도 `rev` 도 없다. 파일은 바로잡았을 수 있다
    /// ([`SettingsService::set`]).
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct ResetOutcome {
    #[ts(type = "number")]
    pub rev: u64,
    /// 선택자가 덮은 키 전부(표 순서) — 호출 뒤 모두 기본값이다.
    pub reset: Vec<String>,
    /// 그중 유효 값이 실제로 바뀐 키의 새 값(= 기본값). 비었으면 알림도 `rev` 도 없다 — 파일은 바로잡았을 수
    /// 있다([`SettingsService::reset`]).
    pub changed: Vec<SettingItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SchemaItem {
    pub key: String,
    /// `"choice"` · `"css-length"` · `"css-number"`.
    pub kind: &'static str,
    pub default: String,
    /// `choice` 만 — 정규 철자.
    pub choices: Option<Vec<String>>,
    /// 양 끝 포함 · 정규형. `css-length` 는 받는 단위마다 한 값씩 `, ` 로 잇는다(`"8px, 0.5rem, 0.5em"`).
    /// `choice` 는 `None`.
    pub min: Option<String>,
    /// [`SchemaItem::min`] 과 같은 모양.
    pub max: Option<String>,
    pub description: String,
}

/// `settings_schema` 의 답 — 버스 `settings.schema` 와 같은 모양(TRD §5-4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SettingsSchema {
    /// 표 순서.
    pub items: Vec<SchemaItem>,
}

// ADR-0265
pub struct SettingsService {
    /// 쓰기 직렬화 + 디스크 — 락 순서는 모듈 헤더.
    io: Mutex<Box<dyn SettingsFiles>>,
    announce: Mutex<()>,
    state: Mutex<State>,
    /// 로그에 실을 파일 출처 — 적재 때 한 번 받는다.
    origin: String,
}

struct State {
    overrides: store::Overrides,
    rev: u64,
    writable: bool,
    /// 적재가 모은 로그 — [`SettingsService::enable_writes`] 가 한 번 내고 비운다.
    load_notes: Vec<store::LoadNote>,
}

impl State {
    fn effective(&self, def: &SettingDef) -> &str {
        self.overrides
            .get(def.key)
            .map(String::as_str)
            .unwrap_or(def.default)
    }

    fn item(&self, def: &SettingDef) -> SettingItem {
        SettingItem {
            key: def.key.to_string(),
            value: self.effective(def).to_string(),
            is_default: !self.overrides.contains_key(def.key),
        }
    }

    fn ensure_writable(&self) -> Result<(), SettingsError> {
        if self.writable {
            Ok(())
        } else {
            Err(SettingsError::Internal(
                "설정 쓰기가 아직 열리지 않았다(셸 setup 전)".to_string(),
            ))
        }
    }

    /// 메모리에 반영하고 `rev` 를 올린다 — 파일 쓰기가 끝난 뒤에만 부른다.
    fn commit(&mut self, changes: &[(&'static str, Option<String>)]) -> u64 {
        for (key, value) in changes {
            match value {
                Some(value) => self.overrides.insert(*key, value.clone()),
                None => self.overrides.remove(key),
            };
        }
        self.rev += 1;
        self.rev
    }
}

const NOT_FOUND: &str = "모르는 설정 키이거나 맞는 키가 없는 접두다 — 목록은 settings.schema";

/// `None` = 전부. 맞는 키가 없으면 `NotFound`.
fn select(selector: Option<&str>) -> Result<Vec<&'static SettingDef>, SettingsError> {
    let defs = match selector {
        None => registry::SETTINGS.iter().collect(),
        Some(selector) => registry::select(selector),
    };
    if defs.is_empty() {
        return Err(SettingsError::NotFound(NOT_FOUND.to_string()));
    }
    Ok(defs)
}

/// 파일을 쓸지 — 파일을 읽었으면 파일이 목표와 다른가로, 못 읽었으면(IO 실패 · 통째로 못 쓰는 파일 · 새 판이
/// 쓴 파일) 유효 값이 바뀌나로 가른다.
fn needs_write(
    document: &store::Document,
    disk_differs: impl FnOnce(&store::Document) -> Option<bool>,
    memory_changed: bool,
) -> bool {
    disk_differs(document).unwrap_or(memory_changed)
}

impl SettingsService {
    /// 읽기 전용 적재 — 파일이 없거나 못 쓸 것이어도 기본값으로 서고, 디스크를 바꾸지 않는다.
    pub fn load(files: Box<dyn SettingsFiles>) -> Self {
        let (overrides, load_notes) = store::load(&*files);
        let origin = files.origin();
        Self {
            io: Mutex::new(files),
            announce: Mutex::new(()),
            state: Mutex::new(State {
                overrides,
                rev: 0,
                writable: false,
                load_notes,
            }),
            origin,
        }
    }

    /// 운영 적재 — `config_dir` = 셸 config 폴더(`DataLayout::shell_config_dir`). 폴더가 없어도 만들지 않는다.
    pub fn load_from_dir(config_dir: &Path) -> Self {
        Self::load(Box::new(store::FsSettingsFiles::in_dir(config_dir)))
    }

    /// 이 뒤로 `set` · `reset` 이 디스크에 쓴다. 되돌리는 길은 없다.
    ///
    /// ★로거를 세운 뒤에 부른다★ — 적재가 모은 로그(못 쓰는 파일 · 접힌 값)를 처음 부를 때 한 번 여기서 낸다.
    pub fn enable_writes(&self) {
        let notes = {
            let mut state = self.lock_state();
            state.writable = true;
            std::mem::take(&mut state.load_notes)
        };
        for note in &notes {
            note.emit(&self.origin);
        }
    }

    /// `key` = 정확한 키 또는 `.` 으로 끝나는 접두 · `None` = 전부.
    pub fn get(&self, key: Option<&str>) -> Result<SettingsSnapshot, SettingsError> {
        let defs = select(key)?;
        let state = self.lock_state();
        Ok(SettingsSnapshot {
            rev: state.rev,
            items: defs.into_iter().map(|def| state.item(def)).collect(),
        })
    }

    /// 정확한 키 하나에 값을 쓴다. 기본값과 같은 값이면 파일에서 그 키를 지운다.
    ///
    /// ★쓸지는 파일로, `changed` 는 유효 값으로 가른다★ — 파일이 이미 목표대로면(기본값 = 키 없음 · 아니면
    /// 정규 철자) 안 쓴다. 다르면 유효 값이 같아도 쓴다 — 밖에서 고친 값이나 접힌 못 쓸 값을 바로잡는다. 그때
    /// 답은 `changed: false` 이고 `rev` · 알림은 그대로다. 파일을 못 읽으면(IO 실패 · 통째로 못 쓰는 파일 · 새 판이
    /// 쓴 파일) 유효 값이 바뀔 때만 쓴다 — IO 실패면 그 쓰기는 `Internal`, 새 판이 쓴 파일이면 덮지 않고 `Conflict`.
    ///
    /// 알림은 내지 않는다 — 셸의 쓰기는 [`set_and_notify`] 로 부른다.
    ///
    /// 인자 오류(`NotFound` · `InvalidArgument`)가 [`Self::enable_writes`] 전의 `Internal` 보다 먼저다.
    pub fn set(&self, key: &str, value: &str) -> Result<SetOutcome, SettingsError> {
        self.set_announced(key, value, |_| {})
    }

    /// [`Self::set`] — 확정 직후 `announce` 를 알림 순서 락 아래서 부른다(`changed: false` 여도 부른다).
    fn set_announced(
        &self,
        key: &str,
        value: &str,
        announce: impl FnOnce(&SetOutcome),
    ) -> Result<SetOutcome, SettingsError> {
        if key.ends_with('.') {
            return Err(SettingsError::InvalidArgument(
                "set 은 정확한 키 하나를 받는다 — 접두는 get · reset · schema 만".to_string(),
            ));
        }
        let def =
            registry::find(key).ok_or_else(|| SettingsError::NotFound(NOT_FOUND.to_string()))?;
        let value = def
            .normalize(value)
            .map_err(|e| SettingsError::InvalidArgument(format!("{}: {e}", def.key)))?;
        let stored = (value != def.default).then(|| value.clone());

        let io = self.lock_io();
        let memory_changed = {
            let state = self.lock_state();
            state.ensure_writable()?;
            state.effective(def) != value
        };
        let document = store::read_document(&**io);
        if needs_write(
            &document,
            |doc| doc.differs(def.key, stored.as_deref()),
            memory_changed,
        ) {
            self.persist(&**io, document, &[(def.key, stored.clone())])?;
            if !memory_changed {
                tracing::info!(
                    module = "settings",
                    key = def.key,
                    "설정 파일만 바로잡았다(유효 값 그대로)"
                );
            }
        }

        let _order = self.lock_announce();
        let rev = {
            let mut state = self.lock_state();
            if memory_changed {
                state.commit(&[(def.key, stored)])
            } else {
                state.rev
            }
        };
        drop(io);
        if memory_changed {
            tracing::info!(module = "settings", key = def.key, value = %value, rev, "설정을 바꿨다");
        }
        let outcome = SetOutcome {
            rev,
            key: def.key.to_string(),
            value,
            changed: memory_changed,
        };
        announce(&outcome);
        Ok(outcome)
    }

    /// 선택자(정확한 키 또는 `.` 으로 끝나는 접두)가 덮은 키를 기본값으로 — 전체 초기화는 없다.
    ///
    /// 쓸지 · `changed` 의 가름은 [`Self::set`] 과 같다 — 파일에 선택자의 키가 하나라도 있으면(접힌 못 쓸 값 ·
    /// 밖에서 쓴 기본값 포함) 그 키들을 지워 쓰고, 유효 값이 바뀐 키만 `changed` 에 싣는다. 오류 순서도
    /// [`Self::set`] 과 같다. 알림은 내지 않는다 — 셸의 쓰기는 [`reset_and_notify`] 로 부른다.
    pub fn reset(&self, key: &str) -> Result<ResetOutcome, SettingsError> {
        self.reset_announced(key, |_| {})
    }

    /// [`Self::reset`] — 확정 직후 `announce` 를 알림 순서 락 아래서 부른다(`changed` 가 비어도 부른다).
    fn reset_announced(
        &self,
        key: &str,
        announce: impl FnOnce(&ResetOutcome),
    ) -> Result<ResetOutcome, SettingsError> {
        let defs = select(Some(key))?;

        let io = self.lock_io();
        let changed: Vec<&'static SettingDef> = {
            let state = self.lock_state();
            state.ensure_writable()?;
            defs.iter()
                .copied()
                .filter(|def| state.overrides.contains_key(def.key))
                .collect()
        };
        let document = store::read_document(&**io);
        let disk_differs = |doc: &store::Document| {
            defs.iter()
                .map(|def| doc.differs(def.key, None))
                .try_fold(false, |any, differs| Some(any || differs?))
        };
        if needs_write(&document, disk_differs, !changed.is_empty()) {
            let removals: Vec<_> = defs.iter().map(|def| (def.key, None)).collect();
            self.persist(&**io, document, &removals)?;
            if changed.is_empty() {
                tracing::info!(
                    module = "settings",
                    keys = defs.len(),
                    "설정 파일만 바로잡았다(유효 값 그대로)"
                );
            }
        }

        let _order = self.lock_announce();
        let rev = {
            let mut state = self.lock_state();
            if changed.is_empty() {
                state.rev
            } else {
                let removals: Vec<_> = changed.iter().map(|def| (def.key, None)).collect();
                state.commit(&removals)
            }
        };
        drop(io);
        if !changed.is_empty() {
            tracing::info!(
                module = "settings",
                changed = changed.len(),
                rev,
                "설정을 기본값으로 되돌렸다"
            );
        }
        let outcome = ResetOutcome {
            rev,
            reset: defs.iter().map(|def| def.key.to_string()).collect(),
            changed: changed
                .iter()
                .map(|def| SettingItem {
                    key: def.key.to_string(),
                    value: def.default.to_string(),
                    is_default: true,
                })
                .collect(),
        };
        announce(&outcome);
        Ok(outcome)
    }

    /// `key` 는 [`Self::get`] 과 같다.
    pub fn schema(&self, key: Option<&str>) -> Result<Vec<SchemaItem>, SettingsError> {
        Ok(select(key)?
            .into_iter()
            .map(|def| {
                let (min, max) = def.bounds();
                SchemaItem {
                    key: def.key.to_string(),
                    kind: def.kind_name(),
                    default: def.default.to_string(),
                    choices: def.choices(),
                    min,
                    max,
                    description: def.desc.to_string(),
                }
            })
            .collect())
    }

    /// 정확한 키 하나의 지금 값(기본값 포함). `None` = 표에 없는 키(접두도 `None`).
    pub fn effective(&self, key: &str) -> Option<String> {
        let def = registry::find(key)?;
        Some(self.lock_state().effective(def).to_string())
    }

    /// `io` 를 쥔 채 부른다 — 실패하면 메모리 · `rev` 그대로다. 새 판이 쓴 파일 = `Conflict` · 지금 파일을 못 읽음 =
    /// `Unreadable` · 쓰기 실패 = `Internal`.
    fn persist(
        &self,
        files: &dyn SettingsFiles,
        document: store::Document,
        changes: &[(&'static str, Option<String>)],
    ) -> Result<(), SettingsError> {
        let memory = self.lock_state().overrides.clone();
        store::write(files, document, &memory, changes).map_err(|e| match e {
            store::WriteError::Newer { found } => {
                tracing::warn!(
                    module = "settings",
                    source = %self.origin,
                    keys = changes.len(),
                    found,
                    "설정 파일이 이 셸보다 새 판이 쓴 것이라 덮지 않고 바꾸지 않았다"
                );
                SettingsError::Conflict(format!(
                    "설정 파일이 이 앱보다 새 버전(판 {found})이 쓴 것이라 덮지 않았다 — 값은 그대로다"
                ))
            }
            store::WriteError::Unreadable(e) => {
                tracing::warn!(
                    module = "settings",
                    source = %self.origin,
                    keys = changes.len(),
                    "설정 파일을 못 읽어 덮지 않고 바꾸지 않았다: {e}"
                );
                SettingsError::Unreadable(format!(
                    "설정 파일을 지금 못 읽어 쓰지 않았다 — 값은 그대로다. 대개 잠깐 쥔 잠김이라 조금 뒤 다시 하면 된다: {e}"
                ))
            }
            store::WriteError::Io(e) => {
                tracing::warn!(
                    module = "settings",
                    source = %self.origin,
                    keys = changes.len(),
                    "설정을 디스크에 못 써 바꾸지 않았다: {e}"
                );
                SettingsError::Internal(format!("설정 파일을 못 썼다: {e}"))
            }
        })
    }

    // 셋 다 중독돼도 계속 돈다 — 메모리는 파일 쓰기가 성공한 뒤에만 바뀌므로 패닉한 쓰기가 반쯤 바꾼 상태를
    //   남기지 않고, `io` · `announce` 는 순서만 지킨다.
    fn lock_io(&self) -> MutexGuard<'_, Box<dyn SettingsFiles>> {
        sync::lock(&self.io)
    }

    fn lock_announce(&self) -> MutexGuard<'_, ()> {
        sync::lock(&self.announce)
    }

    fn lock_state(&self) -> MutexGuard<'_, State> {
        sync::lock(&self.state)
    }
}

/// 실제로 바뀐 쓰기를 알리는 자리 — 운영 = 셸의 Tauri 어댑터, 시험 = 가짜(ADR-0012).
///
/// 바뀐 것이 없는 호출(`changed: false` · 빈 `changed`)과 오류에는 부르지 않는다.
pub trait SettingsEvents: Send + Sync {
    /// 바뀐 키의 새 값 — `rev` = 그 쓰기의 번호, `items` = 바뀐 키만(표 순서). 부르는 순서 = `rev` 순서.
    ///
    /// ★알림 순서 락을 쥔 채 불린다 — 막히지 말 것★: 그동안 다른 쓰기의 확정이 줄 선다(운영 구현 = Tauri
    /// `emit`, 이벤트 루프로 넘기기만 한다). 서비스를 읽는 것(`get` · `effective`)은 된다 — 그것들은 상태 락만
    /// 잠깐 잡는다. ★설정을 쓰면(`set` · `reset`) 교착이다★.
    fn changed(&self, change: &SettingsSnapshot);
    /// 바뀐 키에 [`THEME_DEFAULT`] 가 들었다 — 같은 쓰기의 [`Self::changed`] 뒤에 한 번. ★서비스의 락을 하나도
    /// 쥐지 않은 채 불린다★ — 밀기는 테마 관문 → 상태 락을 잡으므로, 여기서 다른 락을 쥐고 있으면 순환이 생길
    /// 자리가 된다.
    fn theme_default_changed(&self);
}

/// [`SettingsService::set`] + 알림 — 사람 경로(Tauri 명령)와 LLM 경로(버스 명령)가 함께 부르는 쓰기.
///
/// ★디스크를 기다린다(`sync_all` 까지)★ — async 워커에서 부르지 말 것.
// ADR-0081
pub fn set_and_notify(
    service: &SettingsService,
    events: &dyn SettingsEvents,
    key: &str,
    value: &str,
) -> Result<SetOutcome, SettingsError> {
    let outcome = service.set_announced(key, value, |outcome| {
        if outcome.changed {
            // 기본값과 같은 값은 덮어쓰기를 지우므로(`set`) 그 키는 기본값 상태다 — `get` 이 내는 것과 같은 판정.
            let is_default =
                registry::find(&outcome.key).is_some_and(|def| def.default == outcome.value);
            events.changed(&SettingsSnapshot {
                rev: outcome.rev,
                items: vec![SettingItem {
                    key: outcome.key.clone(),
                    value: outcome.value.clone(),
                    is_default,
                }],
            });
        }
    })?;
    if outcome.changed && outcome.key == THEME_DEFAULT {
        events.theme_default_changed();
    }
    Ok(outcome)
}

/// [`SettingsService::reset`] + 알림 — [`set_and_notify`] 의 짝.
// ADR-0081
pub fn reset_and_notify(
    service: &SettingsService,
    events: &dyn SettingsEvents,
    key: &str,
) -> Result<ResetOutcome, SettingsError> {
    let outcome = service.reset_announced(key, |outcome| {
        if !outcome.changed.is_empty() {
            events.changed(&SettingsSnapshot {
                rev: outcome.rev,
                items: outcome.changed.clone(),
            });
        }
    })?;
    if outcome.changed.iter().any(|item| item.key == THEME_DEFAULT) {
        events.theme_default_changed();
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;
