//! Shell settings service — the single entry point over the schema table ([`registry`]) and the file ([`store`]). The human path (Tauri command) and the LLM path (bus command) call the same [`SettingsService`] (TRD §5).
//! - A value is always a canonical string (TRD §5-2) — `get` · the `set` answer · the notification carry the same spelling.
//! - Loading is read-only: [`SettingsService::load`] neither creates nor edits the file and emits no log (it runs before the logger). Writes and the load log go out only after [`SettingsService::enable_writes`] (the shell `setup` calls it after setting up the logger, so a load before build does not change the disk).
//! - Three locks — order = write serialization (`io`) → notification order (`announce`) → state (`state`):
//!   - `io` queues writes against each other and holds the disk handle; held during read-modify-write and `sync_all`.
//!   - `state` is a short lock on the in-memory values · `rev` — a leaf: while holding it no other lock · IO · emit. Reads (`get` · `effective` · theme push) take only this, so they do not wait for a write's `sync_all`.
//!   - `announce` binds the commit (issuing `rev`) and its notification into one unit — notification order = `rev` order. The only lock held during a notification (`io` is released before).
//!   - Do not take `io` while holding `state` (a read would queue behind a write's disk wait). The theme gate (`theme::EffectiveThemes`) takes only `state` briefly and a settings write calls the theme push only after releasing all locks, so nothing takes them the other way round.
//! - The notification port is given by the shell's write entry points ([`set_and_notify`] · [`reset_and_notify`]): right after the commit the service calls that callback under `announce` and knows nothing of Tauri or emit, so tests run without an app.
//! - `rev` = 0 at load, +1 for each call in which the effective value actually changed; a call that only corrected the file (a collapsed unwritable value · an externally edited value) does not raise it.
//! - A value edited outside the app is not reflected while running — it is read at the next load (a write overwrites it only when it writes or reverts that key).

mod registry;
mod store;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;
use ts_rs::TS;

use registry::SettingDef;
pub use store::SettingsFiles;

/// The theme of windows with no per-window override — when this key changes the shell pushes every window's effective theme again ([`SettingsEvents::theme_default_changed`]).
pub const THEME_DEFAULT: &str = "theme.default";

/// The UI language (`en` | `ko`) — when this key changes the shell relabels its own text: automatic tab names · OS window titles · the tray ([`SettingsEvents::ui_language_changed`]).
// ADR-0372
pub const UI_LANGUAGE: &str = "ui.language";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    InvalidArgument(String),
    /// The settings file was written by a version newer than this shell, so it was not overwritten (memory · `rev` unchanged · no notification · file as is). Retrying gives the same result until that file changes.
    // ADR-0291 R2
    #[error("{0}")]
    Conflict(String),
    /// The current file could not be read just before writing, so it was not overwritten (memory · `rev` unchanged · no notification · file as is). It is re-read on every write, so usually a briefly held lock and retrying a little later is enough.
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
/// last applied to that key. ★A key seen for the first time is applied as is★ — the answer right after the shell starts has `rev` 0, so comparing with 「greater than 0」
/// would drop the first value. The `settings_get` answer also follows the same rule per item with the answer's `rev`: if the answer
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
    io: Mutex<Box<dyn SettingsFiles>>,
    announce: Mutex<()>,
    state: Mutex<State>,
    origin: String,
}

struct State {
    overrides: store::Overrides,
    rev: u64,
    writable: bool,
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
                "settings writes are not open yet (before shell setup)".to_string(),
            ))
        }
    }

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

const NOT_FOUND: &str =
    "unknown settings key, or a prefix matching no key — see settings.schema for the list";

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

/// Whether to write the file — when the file was read, by whether it differs from the target; when it could not be read (IO failure · a wholly unusable file · a newer-version file), by whether the effective value changes.
fn needs_write(
    document: &store::Document,
    disk_differs: impl FnOnce(&store::Document) -> Option<bool>,
    memory_changed: bool,
) -> bool {
    disk_differs(document).unwrap_or(memory_changed)
}

impl SettingsService {
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

    pub fn load_from_dir(config_dir: &Path) -> Self {
        Self::load(Box::new(store::FsSettingsFiles::in_dir(config_dir)))
    }

    /// After this `set` · `reset` write to disk; no way back. Call it after the logger is set up — the logs the load collected (an unusable file · collapsed values) are emitted here once, at the first call.
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

    pub fn get(&self, key: Option<&str>) -> Result<SettingsSnapshot, SettingsError> {
        let defs = select(key)?;
        let state = self.lock_state();
        Ok(SettingsSnapshot {
            rev: state.rev,
            items: defs.into_iter().map(|def| state.item(def)).collect(),
        })
    }

    /// Writes a value to one exact key; a value equal to the default removes the key from the file.
    /// Whether to write is decided by the file, `changed` by the effective value: if the file is already as targeted (default = no key · otherwise the canonical spelling) nothing is written; if different it writes even when the effective value is the same — correcting an externally edited or collapsed unusable value — and the answer is `changed: false` with `rev` · notification unchanged. If the file cannot be read (IO failure · a wholly unusable file · a newer-version file), it writes only when the effective value changes — on IO failure that write is `Internal`, and for a newer-version file it does not overwrite and gives `Conflict`.
    /// It emits no notification — the shell's write calls [`set_and_notify`]. Argument errors (`NotFound` · `InvalidArgument`) come before the `Internal` given before [`Self::enable_writes`].
    pub fn set(&self, key: &str, value: &str) -> Result<SetOutcome, SettingsError> {
        self.set_announced(key, value, |_| {})
    }

    fn set_announced(
        &self,
        key: &str,
        value: &str,
        announce: impl FnOnce(&SetOutcome),
    ) -> Result<SetOutcome, SettingsError> {
        if key.ends_with('.') {
            return Err(SettingsError::InvalidArgument(
                "set takes exactly one key — prefixes work only with get · reset · schema"
                    .to_string(),
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
                    "only the settings file was corrected (the valid value stays)"
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
            tracing::info!(module = "settings", key = def.key, value = %value, rev, "setting changed");
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

    /// Returns keys covered by the selector (an exact key or a prefix ending in `.`) to the default — there is no full reset. Whether to write · `changed` is as in [`Self::set`]: if the file has even one key of the selector (including a collapsed unusable value or an externally written default), those keys are removed and written, and only keys whose effective value changed go into `changed`. Error order is also the same. It emits no notification — the shell's write calls [`reset_and_notify`].
    pub fn reset(&self, key: &str) -> Result<ResetOutcome, SettingsError> {
        self.reset_announced(key, |_| {})
    }

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
                    "only the settings file was corrected (the valid value stays)"
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
                "setting reset to the default"
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

    pub fn effective(&self, key: &str) -> Option<String> {
        let def = registry::find(key)?;
        Some(self.lock_state().effective(def).to_string())
    }

    /// Called while holding `io` — on failure memory · `rev` stay. A newer-version file = `Conflict` · cannot read the current file = `Unreadable` · write failure = `Internal`.
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
                    "the settings file was written by a newer version than this shell; not overwritten and not changed"
                );
                SettingsError::Conflict(format!(
                    "the settings file was written by a newer version of the app (version {found}); not overwritten — the value stays"
                ))
            }
            store::WriteError::Unreadable(e) => {
                tracing::warn!(
                    module = "settings",
                    source = %self.origin,
                    keys = changes.len(),
                    "could not read the settings file; not overwritten and not changed: {e}"
                );
                SettingsError::Unreadable(format!(
                    "could not read the settings file right now, so nothing was written — the value stays. It is usually a brief lock; try again shortly: {e}"
                ))
            }
            store::WriteError::Io(e) => {
                tracing::warn!(
                    module = "settings",
                    source = %self.origin,
                    keys = changes.len(),
                    "could not write the setting to disk; not changed: {e}"
                );
                SettingsError::Internal(format!("could not write the settings file: {e}"))
            }
        })
    }

    // Keeps running even if all three are poisoned — memory changes only after the file write succeeds, so a panicked write leaves no half-changed state, and `io` · `announce` only keep order.
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

/// Announces a write that actually changed something — production = the shell's Tauri adapter, tests = a fake (ADR-0012). Not called for a call with nothing changed (`changed: false` · empty `changed`) nor for errors.
pub trait SettingsEvents: Send + Sync {
    /// New values of the changed keys — `rev` = that write's number, `items` = only the changed keys (table order). Call order = `rev` order.
    /// Called while holding the notification order lock — do not block: other writes' commits queue meanwhile (production = Tauri `emit`, which only hands off to the event loop). Reading the service (`get` · `effective`) is fine (only the state lock, briefly); writing settings (`set` · `reset`) deadlocks.
    fn changed(&self, change: &SettingsSnapshot);
    /// The changed keys include [`THEME_DEFAULT`] — once, after [`Self::changed`] of the same write, holding none of the service's locks: the push takes the theme gate → state lock, so holding another lock here would be where a cycle arises.
    fn theme_default_changed(&self);
    /// The changed keys include [`UI_LANGUAGE`] — once, after [`Self::changed`] of the same write and after [`Self::theme_default_changed`] if that is called too, holding none of the service's locks (like [`Self::theme_default_changed`]). Carries no value — the implementation reads the current effective value, so hooks of two writes running in reverse order still end on the last stored value.
    /// Never called on the main thread: its callers (the `settings_set` · `settings_reset` commands and the bus verbs) run on the blocking pool, and the production implementation does not wait for the main thread either — it posts the tray relabel there and returns, so the write's reply is not held by a busy main thread.
    // ADR-0373
    fn ui_language_changed(&self);
}

/// [`SettingsService::set`] + notification — the write both the human path (Tauri command) and the LLM path (bus command) call. Waits for the disk (up to `sync_all`) — do not call it from an async worker.
// ADR-0081
pub fn set_and_notify(
    service: &SettingsService,
    events: &dyn SettingsEvents,
    key: &str,
    value: &str,
) -> Result<SetOutcome, SettingsError> {
    let outcome = service.set_announced(key, value, |outcome| {
        if outcome.changed {
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
    if outcome.changed && outcome.key == UI_LANGUAGE {
        events.ui_language_changed();
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
    if outcome.changed.iter().any(|item| item.key == UI_LANGUAGE) {
        events.ui_language_changed();
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;
