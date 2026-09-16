//! Session-owned catalog discovery, file loading, and transpile warmup.
//!
//! This module keeps catalog and source preparation mechanics separate from
//! play, wall, focus, and slot lifecycle control while preserving Session's
//! public controls and their existing behavior.

use std::path::{Path, PathBuf};

use crate::session::Session;

fn default_catalog_browse_dir(last: Option<&Path>) -> PathBuf {
    crate::script_picker::default_load_browse_dir(last)
}

impl Session {
    pub(crate) fn catalog_root(&self) -> Result<Option<PathBuf>, String> {
        match self.server_profile.as_ref() {
            Some(profile) => Ok(profile.catalog_root().map(Path::to_path_buf)),
            None if self.profile_options.is_some() => self
                .resolve_profile()
                .map(|selection| selection.catalog_root().map(Path::to_path_buf)),
            None => Ok(script::rs2b0t_root()),
        }
    }

    /// First Load/Browse: fill the JS library from the `$RS2B0T` catalog
    /// exactly once per session. `$RS2B0T` (or the persisted root path) is
    /// read only here — never in [`Session::new`] — so boot and unit tests
    /// that merely construct a `Session` do not parse a real catalog or
    /// rewrite `~/.274bot/rs2b0t-path`. No V8 here: sources are read and
    /// classified only; the isolate is spawned on Start. Transpile is
    /// [`JsLibrary::ensure_js`] on first click / Start / Transpile all.
    pub fn fill_rs2b0t_cards_once(&mut self) {
        if self.rs2b0t_filled {
            return;
        }
        let root = match self.catalog_root() {
            Ok(Some(root)) => root,
            Ok(None) => return,
            Err(error) => {
                self.error = Some(format!("server profile: {error}"));
                return;
            }
        };
        self.rs2b0t_filled = true;
        if let Err(e) = self
            .js
            .register_rs2b0t(&root, &script::default_rs2b0t_path_file())
        {
            if host::debug_enabled() {
                eprintln!("[panel] $RS2B0T registry: {e}");
            }
        }
    }

    fn rs2b0t_import_file(&self) -> PathBuf {
        script::default_rs2b0t_import_file()
    }

    fn rs2b0t_path_file(&self) -> PathBuf {
        script::default_rs2b0t_path_file()
    }

    /// Opening Browse: fill from `$RS2B0T`/persisted root, or prompt for a
    /// clone root, or honour a prior defer.
    pub fn on_script_browse_open(&mut self) {
        if self.rs2b0t_filled {
            return;
        }
        let selected = match self.catalog_root() {
            Ok(selected) => selected,
            Err(error) => {
                self.error = Some(format!("server profile: {error}"));
                return;
            }
        };
        self.rs2b0t_filled = true;
        if let Some(root) = selected {
            if let Err(e) = self.js.register_rs2b0t(&root, &self.rs2b0t_path_file()) {
                if host::debug_enabled() {
                    eprintln!("[panel] $RS2B0T registry: {e}");
                }
            }
            return;
        }
        if script::rs2b0t_import_deferred_at(&self.rs2b0t_import_file()) {
            return;
        }
        self.rs2b0t_catalog_open = true;
        self.rs2b0t_catalog_defer_ok = true;
        self.script_dialog_search.clear();
        self.rs2b0t_catalog_dir =
            default_catalog_browse_dir(self.ui.script_catalog_last_dir.as_deref());
    }

    /// Operator chose **Not now** on the first-run catalog prompt.
    pub fn defer_rs2b0t_catalog(&mut self) {
        let _ = script::set_rs2b0t_import_deferred_at(&self.rs2b0t_import_file());
        self.rs2b0t_catalog_open = false;
    }

    /// Import catalog cards from a clone root that contains
    /// `src/bot/scripts/index.ts`. Persists `rs2b0t-path` and clears defer.
    pub fn import_rs2b0t_catalog(&mut self, root: &Path) -> Result<usize, String> {
        if !crate::script_picker::rs2b0t_root_has_index(root) {
            return Err(format!(
                "no catalog at {}",
                script::registry_index_path(root).display()
            ));
        }
        let n = self.js.register_rs2b0t(root, &self.rs2b0t_path_file())?;
        let _ = script::clear_rs2b0t_import_at(&self.rs2b0t_import_file());
        self.ui.script_catalog_last_dir = Some(root.to_path_buf());
        if self.persist_ui {
            crate::ui_state::save(&self.ui);
        }
        self.rs2b0t_catalog_open = false;
        Ok(n)
    }

    /// The card currently at the front of the warmup queue, if any.
    pub fn transpile_front(&self) -> Option<(script::ScriptSource, &str)> {
        self.transpile_queue
            .front()
            .map(|(source, name)| (*source, name.as_str()))
    }

    /// Select a Browse card. Transpile is deferred: cache hits fill `js`
    /// immediately (disk read); misses enqueue **this card only**.
    pub fn select_script_card(&mut self, source: script::ScriptSource, name: impl Into<String>) {
        let name = name.into();
        let sel = script::ScriptSel::Loaded(source, name.clone());
        self.script_sel = Some(sel.clone());
        if let Some(profile) = self.focused_name() {
            self.set_pending_browse(&profile, sel);
        }
        self.enqueue_transpile(source, name, true);
    }

    /// Operator opted into warming every unwarmed card. Still one
    /// `ensure_js` per armed frame — never a catalog-wide click.
    pub fn queue_transpile_all(&mut self) {
        let need = self.js.cards_needing_transpile();
        if need.is_empty() {
            return;
        }
        self.transpile_done = 0;
        self.transpile_total = need.len();
        for (source, name) in need {
            if !self
                .transpile_queue
                .iter()
                .any(|(s, n)| *s == source && n == &name)
            {
                self.transpile_queue.push_back((source, name));
            }
        }
    }

    /// First call after enqueue only arms (so the frame can paint
    /// `transpiling…`). The next call runs one `ensure_js`, then arms
    /// again if the queue still has work.
    pub fn pump_script_transpile(&mut self) {
        if self.transpile_armed {
            if let Some((source, name)) = self.transpile_queue.pop_front() {
                match self.js.ensure_js(source, &name) {
                    Ok(()) => {
                        self.error = None;
                        self.transpile_done = self.transpile_done.saturating_add(1);
                    }
                    Err(e) => self.error = Some(format!("transpile {name}: {e}")),
                }
            }
            self.transpile_armed = false;
        }
        if !self.transpile_queue.is_empty() {
            self.transpile_armed = true;
        } else {
            self.transpile_done = 0;
            self.transpile_total = 0;
        }
    }

    pub(crate) fn enqueue_transpile(
        &mut self,
        source: script::ScriptSource,
        name: String,
        to_front: bool,
    ) {
        if self.js.js_is_ready(source, &name) {
            return;
        }
        let Some(card) = self.js.get(source, &name) else {
            return;
        };
        if self.js.cache().is_cached(card.origin.as_bytes()) {
            if let Err(e) = self.js.ensure_js(source, &name) {
                self.error = Some(format!("transpile {name}: {e}"));
            }
            return;
        }
        if let Some(idx) = self
            .transpile_queue
            .iter()
            .position(|(s, n)| *s == source && n == &name)
        {
            if to_front && idx > 0 {
                if let Some(item) = self.transpile_queue.remove(idx) {
                    self.transpile_queue.push_front(item);
                }
            }
            return;
        }
        if self.transpile_total == 0 {
            self.transpile_total = 1;
            self.transpile_done = 0;
        }
        if to_front {
            self.transpile_queue.push_front((source, name));
        } else {
            self.transpile_queue.push_back((source, name));
        }
    }

    /// Same-folder `./Foo.js` siblings cached for isolate Start.
    pub(crate) fn sibling_modules_for_card(
        &self,
        card: &script::JsCard,
    ) -> Result<Vec<(String, String)>, String> {
        script::resolve_sibling_modules(
            &card.path,
            &card.origin,
            self.js.cache(),
            script::CacheMeta {
                kind: card.kind,
                source: card.source,
                shape: None,
                api_family: Some(card.api_family.as_str().into()),
            },
        )
    }

    /// Load a local JS/TS file into the library (registers a picker card,
    /// persists `~/.274bot/js-scripts.json`), select it for Start, and
    /// remember the parent directory. Errors set [`Session::error`].
    pub fn load_js(&mut self, path: &Path) {
        if !path.is_file() {
            self.error = Some(format!("script: not a file: {}", path.display()));
            return;
        }
        match self.js.load(path) {
            Ok(card) => {
                self.error = None;
                let sel = script::ScriptSel::Loaded(card.source, card.identity_id());
                self.script_sel = Some(sel.clone());
                if let Some(profile) = self.focused_name() {
                    self.set_pending_browse(&profile, sel);
                }
                if let Some(parent) = path.parent() {
                    self.ui.script_load_last_dir = Some(parent.to_path_buf());
                    if self.persist_ui {
                        crate::ui_state::save(&self.ui);
                    }
                }
                self.script_load_open = false;
            }
            Err(e) => self.error = Some(format!("load: {e}")),
        }
    }

    /// Open the Load file browser at the last visited directory, else the
    /// process working directory (where the OS started the app).
    pub fn open_script_load_browser(&mut self) {
        self.script_load_dir =
            crate::script_picker::default_load_browse_dir(self.ui.script_load_last_dir.as_deref());
        self.script_load_sel = 0;
        self.script_dialog_search.clear();
        self.script_load_open = true;
    }
}
