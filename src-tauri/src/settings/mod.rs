//! Shell settings service — the single entry point over the schema table ([`registry`]) and the file ([`store`]). The human path (Tauri command) and the
//! LLM path (bus command) call the same [`SettingsService`] (TRD §5).
//!
//! - **A value is always a canonical string** (TRD §5-2) — `get` · the `set` answer · the notification carry the same spelling.
//! - **Loading is read-only** — [`SettingsService::load`] neither creates nor edits the file and emits no log
//!   (it runs before the logger). Writes and the load log go out only after [`SettingsService::enable_writes`] (the shell `setup` calls it
//!   after setting up the logger — so that a load before build does not change the disk).
//! - **Three locks — order = write serialization (`io`) → notification order (`announce`) → state (`state`)**:
//!   - `io` = the lock that queues writes against each other — holds the disk handle. Held during read-modify-write and `sync_all`.
//!   - `state` = a short lock on the in-memory values · `rev`. **A leaf** — while holding it, no other lock · IO · emit is done. Reads
//!     (`get` · `effective` · theme push) take only this, so they do not wait for the `sync_all` of a write in progress.
//!   - `announce` = binds the commit (issuing `rev`) and its notification into one unit — notification order = `rev` order. The only lock held during a notification is
//!     this one (`io` is released before that).
//!   - ★Do not take `io` while holding `state`★ — a read would queue again behind a write's disk wait. The theme gate
//!     (`theme::EffectiveThemes`) takes only `state` briefly, and a settings write calls the theme push only after releasing all locks, so
//!     there is no path that takes them the other way round.
//! - **The notification port is given by the shell's write entry points ([`set_and_notify`] · [`reset_and_notify`])** — right after the commit the service just calls
//!   that entry point's callback under `announce` and knows nothing of Tauri or emit. So tests run without an app.
//! - `rev` = 0 at load, and rises by 1 for **each call in which the effective value actually changed**. A call that only corrected the file (a collapsed unwritable value ·
//!   a value edited from outside) does not raise it.
//! - A value edited outside the app is not reflected while running — it is read at the next load (a write overwrites that
//!   value only when it writes or reverts that key).

mod registry;
mod store;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;
use ts_rs::TS;

use registry::SettingDef;
pub use store::SettingsFiles;

/// The theme of windows with no per-window override — when this key changes, the shell pushes the effective theme of every window again
/// ([`SettingsEvents::theme_default_changed`]).
pub const THEME_DEFAULT: &str = "theme.default";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// An unknown key, or a prefix with no matching key at all.
    #[error("{0}")]
    NotFound(String),
    /// A format · range violation (the expected format is in the message), or a prefix that came in the place of an exact key.
    #[error("{0}")]
    InvalidArgument(String),
    /// The settings file was written by a version newer than this shell, so it was not overwritten (memory · `rev` unchanged · no notification · file as is). Until that file
    /// changes, trying again gives the same result.
    // ADR-0291 R2
    #[error("{0}")]
    Conflict(String),
    /// It could not read the current file just before writing, so it did not overwrite (memory · `rev` unchanged · no notification · file as is). It reads again at every write,
    /// so it is usually a briefly held lock and trying again a little later is enough.
    // ADR-0291 R17 — user decision 2026-10-11: a failed read in a re-decision while running is a temporary rejection.
    #[error("{0}")]
    Unreadable(String),
    /// Disk write failure (memory · `rev` unchanged · no notification), or a write before [`SettingsService::enable_writes`].
    #[error("{0}")]
    Internal(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SettingItem {
    pub key: String,
    /// Canonical form. The default if there is no overridden value.
    pub value: String,
    /// There is no overridden value. A key whose unusable file value collapsed to the default is `true` too.
    pub is_default: bool,
}

/// The answer of `settings_get` and the payload of the `settings:changed` notification — for a notification `items` holds only the changed keys.
///
/// ★The receiver applies **per key** by `rev`★ — an item is applied when it is the first time seeing that key, or when it is greater than the `rev`
/// last applied to that key. ★A key seen for the first time is applied as is★ — the answer right after the shell starts has `rev` 0, so comparing with 「0 보다
/// 클 때」(when greater than 0) would drop the first value. The `settings_get` answer also follows the same rule per item with the answer's `rev`: if the answer
/// is dropped as a whole, keys only in that answer are lost, and if applied as a whole, a new notification that arrived earlier is overwritten with the old value. Notifications go out among themselves
/// in `rev` order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SettingsSnapshot {
    // The default ts-rs mapping of u64 is bigint, but serde_json carries it as a number.
    #[ts(type = "number")]
    pub rev: u64,
    /// Table order.
    pub items: Vec<SettingItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SetOutcome {
    /// If `changed: false`, the same value as before the call.
    #[ts(type = "number")]
    pub rev: u64,
    pub key: String,
    /// Canonical form.
    pub value: String,
    /// `false` = the effective value was already that value — no notification and no `rev`. The file may have been corrected
    /// ([`SettingsService::set`]).
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct ResetOutcome {
    #[ts(type = "number")]
    pub rev: u64,
    /// All keys the selector covers (table order) — all are at the default after the call.
    pub reset: Vec<String>,
    /// Among them, the new values (= default) of keys whose effective value actually changed. If empty, no notification and no `rev` — the file may have
    /// been corrected ([`SettingsService::reset`]).
    pub changed: Vec<SettingItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SchemaItem {
    pub key: String,
    /// `"choice"` · `"css-length"` · `"css-number"`.
    pub kind: &'static str,
    pub default: String,
    /// Only `choice` — canonical spelling.
    pub choices: Option<Vec<String>>,
    /// Both ends inclusive · canonical form. `css-length` joins one value per accepted unit with `, ` (`"8px, 0.5rem, 0.5em"`).
    /// `choice` is `None`.
    pub min: Option<String>,
    /// Same shape as [`SchemaItem::min`].
    pub max: Option<String>,
    pub description: String,
}

/// The answer of `settings_schema` — same shape as bus `settings.schema` (TRD §5-4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SettingsSchema {
    /// Table order.
    pub items: Vec<SchemaItem>,
}

// ADR-0265
pub struct SettingsService {
    /// Write serialization + disk — lock order in the module header.
    io: Mutex<Box<dyn SettingsFiles>>,
    announce: Mutex<()>,
    state: Mutex<State>,
    /// File origin to put in the log — received once at load.
    origin: String,
}

struct State {
    overrides: store::Overrides,
    rev: u64,
    writable: bool,
    /// Logs the load collected — [`SettingsService::enable_writes`] emits them once and empties it.
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

    /// Reflects into memory and raises `rev` — called only after the file write is finished.
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

/// `None` = all. `NotFound` if no key matches.
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

/// Whether to write the file — if the file was read, by whether the file differs from the target; if it could not be read (IO failure · a file unusable as a whole · a file written
/// by a newer version), by whether the effective value changes.
fn needs_write(
    document: &store::Document,
    disk_differs: impl FnOnce(&store::Document) -> Option<bool>,
    memory_changed: bool,
) -> bool {
    disk_differs(document).unwrap_or(memory_changed)
}

impl SettingsService {
    /// Read-only load — even if the file is missing or unusable it stands on the defaults, and does not change the disk.
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

    /// Production load — `config_dir` = the shell config folder (`DataLayout::shell_config_dir`). It does not create the folder even if it is missing.
    pub fn load_from_dir(config_dir: &Path) -> Self {
        Self::load(Box::new(store::FsSettingsFiles::in_dir(config_dir)))
    }

    /// After this, `set` · `reset` write to the disk. There is no way back.
    ///
    /// ★Call it after the logger is set up★ — the logs the load collected (an unusable file · collapsed values) are emitted here once, at the first call.
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

    /// `key` = an exact key or a prefix ending in `.` · `None` = all.
    pub fn get(&self, key: Option<&str>) -> Result<SettingsSnapshot, SettingsError> {
        let defs = select(key)?;
        let state = self.lock_state();
        Ok(SettingsSnapshot {
            rev: state.rev,
            items: defs.into_iter().map(|def| state.item(def)).collect(),
        })
    }

    /// Writes a value to one exact key. If the value equals the default, that key is removed from the file.
    ///
    /// ★Whether to write is decided by the file, `changed` by the effective value★ — if the file is already as targeted (default = no key · otherwise the
    /// canonical spelling), it does not write. If different, it writes even if the effective value is the same — correcting a value edited from outside or a collapsed unusable value. In that case the
    /// answer is `changed: false` and `rev` · notification stay as they are. If the file cannot be read (IO failure · a file unusable as a whole · a file written by a newer
    /// version), it writes only when the effective value changes — on IO failure that write is `Internal`, and for a file written by a newer version it does not overwrite and gives `Conflict`.
    ///
    /// It does not emit a notification — the shell's write calls [`set_and_notify`].
    ///
    /// Argument errors (`NotFound` · `InvalidArgument`) come before the `Internal` given before [`Self::enable_writes`].
    pub fn set(&self, key: &str, value: &str) -> Result<SetOutcome, SettingsError> {
        self.set_announced(key, value, |_| {})
    }

    /// [`Self::set`] — right after the commit calls `announce` under the notification order lock (it calls even if `changed: false`).
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

    /// Returns keys covered by the selector (an exact key or a prefix ending in `.`) to the default — there is no full reset.
    ///
    /// The decision of whether to write · `changed` is the same as [`Self::set`] — if the file has even one key of the selector (including a collapsed unusable value ·
    /// a default written from outside), those keys are removed and written, and only keys whose effective value changed go into `changed`. The error order is also the same as
    /// [`Self::set`]. It does not emit a notification — the shell's write calls [`reset_and_notify`].
    pub fn reset(&self, key: &str) -> Result<ResetOutcome, SettingsError> {
        self.reset_announced(key, |_| {})
    }

    /// [`Self::reset`] — right after the commit calls `announce` under the notification order lock (it calls even if `changed` is empty).
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

    /// `key` is the same as [`Self::get`].
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

    /// The current value of one exact key (including the default). `None` = a key not in the table (a prefix is `None` too).
    pub fn effective(&self, key: &str) -> Option<String> {
        let def = registry::find(key)?;
        Some(self.lock_state().effective(def).to_string())
    }

    /// Called while holding `io` — if it fails, memory · `rev` stay as they are. A file written by a newer version = `Conflict` · cannot read the current file =
    /// `Unreadable` · write failure = `Internal`.
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

    // It keeps running even if all three are poisoned — memory changes only after the file write succeeds, so a panicked write leaves no half-changed state,
    //   and `io` · `announce` only keep order.
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

/// The place that announces a write that actually changed something — production = the shell's Tauri adapter, tests = a fake (ADR-0012).
///
/// It is not called for a call with nothing changed (`changed: false` · empty `changed`) and for errors.
pub trait SettingsEvents: Send + Sync {
    /// New values of the changed keys — `rev` = the number of that write, `items` = only the changed keys (table order). Call order = `rev` order.
    ///
    /// ★Called while holding the notification order lock — do not block★: meanwhile the commits of other writes queue (production implementation = Tauri
    /// `emit`, which only hands off to the event loop). Reading the service (`get` · `effective`) is fine — those take only the state lock
    /// briefly. ★Writing settings (`set` · `reset`) is a deadlock★.
    fn changed(&self, change: &SettingsSnapshot);
    /// The changed keys include [`THEME_DEFAULT`] — once, after [`Self::changed`] of the same write. ★Called while holding none of the service's
    /// locks★ — the push takes the theme gate → state lock, so holding another lock here would become the place
    /// where a cycle arises.
    fn theme_default_changed(&self);
}

/// [`SettingsService::set`] + notification — the write that the human path (Tauri command) and the LLM path (bus command) both call.
///
/// ★Waits for the disk (up to `sync_all`)★ — do not call it from an async worker.
// ADR-0081
pub fn set_and_notify(
    service: &SettingsService,
    events: &dyn SettingsEvents,
    key: &str,
    value: &str,
) -> Result<SetOutcome, SettingsError> {
    let outcome = service.set_announced(key, value, |outcome| {
        if outcome.changed {
            // A value equal to the default removes the override (`set`), so that key is in the default state — the same decision as what `get` gives.
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

/// [`SettingsService::reset`] + notification — the pair of [`set_and_notify`].
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
