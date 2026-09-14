//! Per-profile script assignment, bulk Start/Stop, live settings, reload
//! and catalog refresh. Session is the integration owner.

use std::path::Path;

use serde_json::{Map, Value};
use vault::ScriptAssignment;

use crate::session::Session;

#[derive(Debug, Clone)]
pub struct ReloadWarning {
    pub identity_key: String,
    pub source: script::ScriptSource,
    pub lookup: String,
    pub running: Vec<String>,
    pub paused: Vec<String>,
    pub paused_during_prep: Vec<String>,
}

impl Default for ReloadWarning {
    fn default() -> Self {
        Self {
            identity_key: String::new(),
            source: script::ScriptSource::Catalog,
            lookup: String::new(),
            running: Vec::new(),
            paused: Vec::new(),
            paused_during_prep: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    NothingChanged,
    NeedsConfirm,
    Applied {
        restarted: usize,
        stopped_paused: usize,
    },
    Failed(String),
}

impl Session {
    pub fn restore_script_heading(&mut self, profile: &str) {
        if let Some(sel) = self.pending_browse.get(profile).cloned() {
            self.script_sel = Some(sel);
            return;
        }
        if let Some(asg) = self.profile_assignment(profile) {
            self.script_sel = sel_from_assignment(&asg);
            return;
        }
        self.script_sel = None;
    }

    pub fn profile_assignment(&self, profile: &str) -> Option<ScriptAssignment> {
        self.vault
            .as_ref()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_assignment.clone())
    }

    pub fn heading_is_pending(&self) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        let Some(pending) = self.pending_browse.get(&name) else {
            return false;
        };
        match (pending, self.profile_assignment(&name)) {
            (_, None) => true,
            (sel, Some(asg)) => sel_from_assignment(&asg).as_ref() != Some(sel),
        }
    }

    pub fn set_pending_browse(&mut self, profile: &str, sel: script::ScriptSel) {
        self.pending_browse.insert(profile.to_string(), sel);
    }

    fn upsert_profile_settings(
        &mut self,
        profile: &str,
        edit: impl FnOnce(&mut vault::ProfileSettings),
    ) -> bool {
        let Some(vault) = self.vault.as_mut() else {
            self.error = Some("script: vault locked".into());
            return false;
        };
        let Some(mut row) = vault.get(profile).cloned() else {
            self.error = Some(format!("script: no profile {profile}"));
            return false;
        };
        edit(&mut row.settings);
        match vault.upsert(row) {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(format!("script: {e}"));
                false
            }
        }
    }

    pub fn persist_successful_assignment(&mut self, profile: &str, assignment: ScriptAssignment) {
        self.upsert_profile_settings(profile, |settings| {
            settings.script_assignment = Some(assignment);
        });
    }

    pub fn mark_assignment_unavailable(&mut self, profile: &str, reason: impl Into<String>) {
        let reason = reason.into();
        self.upsert_profile_settings(profile, |settings| {
            if let Some(asg) = settings.script_assignment.as_mut() {
                asg.unavailable = Some(reason);
            }
        });
    }

    fn claim_legacy_for(&mut self, profile: &str, key: &str, card_name: &str) {
        let legacy = self.script_settings.overrides(
            script::parse_source_kind(key.split(':').next().unwrap_or("catalog"))
                .unwrap_or(script::ScriptSource::Catalog),
            card_name,
        );
        self.upsert_profile_settings(profile, |settings| {
            script::claim_legacy_overrides(settings, key, card_name, &legacy);
        });
    }

    pub fn profile_overrides(
        &mut self,
        profile: &str,
        key: &str,
        card_name: &str,
    ) -> Map<String, Value> {
        self.claim_legacy_for(profile, key, card_name);
        self.vault
            .as_ref()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_settings.get(key).cloned())
            .unwrap_or_default()
    }

    pub fn merged_profile_bag(
        &mut self,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        schema: &[script::SettingDef],
    ) -> Map<String, Value> {
        let key = script::card_identity_key(source, path, name);
        let overrides = self.profile_overrides(profile, &key, name);
        script::merge_bag(schema, &overrides, self.script_settings_inject.as_ref())
    }

    pub fn set_profile_setting(
        &mut self,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        id: &str,
        value: Value,
    ) -> bool {
        let key = script::card_identity_key(source, path, name);
        self.claim_legacy_for(profile, &key, name);
        let ok = self.upsert_profile_settings(profile, |settings| {
            let mut bag = settings
                .script_settings
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let migrated = script::migrate_legacy_setting_value(name, id, &value);
            bag.insert(id.to_string(), migrated);
            settings.script_settings.insert(key.clone(), bag);
        });
        if ok {
            self.push_live_settings(profile, source, name, path);
        }
        ok
    }

    pub fn push_live_settings(
        &mut self,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
    ) {
        let (identity, generation) = {
            let Some(play) = self.play.as_ref() else {
                return;
            };
            let Some(identity) = play.script_source_identity(profile) else {
                return;
            };
            let key = script::card_identity_key(source, path, name);
            if identity != key {
                return;
            }
            let Some(generation) = play.script_runtime_generation(profile) else {
                return;
            };
            (identity, generation)
        };
        let schema = self
            .js
            .get(source, &lookup_name(source, name, path))
            .map(|c| c.settings_schema.clone())
            .unwrap_or_default();
        let bag = self.merged_profile_bag(profile, source, name, path, &schema);
        if let Some(play) = self.play.as_ref() {
            let _ = play.script_post_settings_fenced(profile, &bag, &identity, generation);
        }
    }

    /// Start the given profile using last successful assignment only.
    pub fn script_start_profile(&mut self, profile: &str) -> Result<(), String> {
        self.script_start_profile_with(profile, false)
    }

    pub(crate) fn script_start_profile_with(
        &mut self,
        profile: &str,
        prefer_pending: bool,
    ) -> Result<(), String> {
        if script_active_name(self, profile) {
            return Ok(());
        }
        let sel = if prefer_pending {
            self.pending_browse
                .get(profile)
                .cloned()
                .or_else(|| self.script_sel.clone())
                .or_else(|| {
                    self.profile_assignment(profile)
                        .and_then(|a| sel_from_assignment(&a))
                })
        } else {
            self.profile_assignment(profile)
                .and_then(|a| sel_from_assignment(&a))
        }
        .ok_or_else(|| "no assignment".to_string())?;
        self.script_start_sel(profile, sel)
    }

    fn script_start_sel(&mut self, profile: &str, sel: script::ScriptSel) -> Result<(), String> {
        if self.play.is_none() {
            return Err("no play".into());
        }
        match sel {
            script::ScriptSel::Compiled(id) => {
                {
                    let play = self.play.as_ref().ok_or_else(|| "no play".to_string())?;
                    play.script_start(profile, id)?;
                    play.script_attach_identity(profile, script::compiled_identity_key(id));
                }
                self.persist_successful_assignment(profile, script::compiled_assignment(id));
                Ok(())
            }
            script::ScriptSel::Loaded(source, lookup) => {
                let card = self
                    .js
                    .get(source, &lookup)
                    .cloned()
                    .ok_or_else(|| match source {
                        script::ScriptSource::File => format!("missing file: {lookup}"),
                        _ => format!("unavailable: {lookup}"),
                    })?;
                if let Some(reason) = &card.unloadable {
                    return Err(format!("unloadable import: {reason}"));
                }
                self.js.ensure_js(source, &lookup)?;
                let card = self
                    .js
                    .get(source, &lookup)
                    .cloned()
                    .ok_or_else(|| format!("no loaded script: {lookup}"))?;
                let bag = self.merged_profile_bag(
                    profile,
                    source,
                    &card.name,
                    &card.path,
                    &card.settings_schema,
                );
                let bag = if bag.is_empty() { None } else { Some(bag) };
                let siblings = script::resolve_sibling_modules(
                    &card.path,
                    &card.origin,
                    self.js.cache(),
                    script::CacheMeta {
                        kind: card.kind,
                        source: card.source,
                        shape: None,
                    },
                )?;
                {
                    let play = self.play.as_ref().ok_or_else(|| "no play".to_string())?;
                    play.script_start_load(profile, card.js.clone(), card.shape, bag, siblings)?;
                    play.script_attach_identity(profile, card.identity_key());
                }
                self.persist_successful_assignment(profile, card.assignment());
                Ok(())
            }
        }
    }

    pub fn script_start_all(&mut self) {
        let members = self.wall.members.clone();
        if members.is_empty() {
            self.error = Some("Start all: no wall members".into());
            return;
        }
        let mut failures = Vec::new();
        let mut started = 0usize;
        let mut skipped = 0usize;
        for name in members {
            let state = self
                .play
                .as_ref()
                .map(|p| p.script_state(&name))
                .unwrap_or(script::RunState::Idle);
            match state {
                script::RunState::Running
                | script::RunState::Paused
                | script::RunState::Stopping => {
                    skipped += 1;
                    continue;
                }
                script::RunState::Idle | script::RunState::Error => {}
            }
            match self.script_start_profile(&name) {
                Ok(()) => started += 1,
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
        let report = format_bulk("Start all", started, skipped, &failures);
        if self.last_bulk_script_report.as_deref() == Some(report.as_str()) {
            return;
        }
        self.last_bulk_script_report = Some(report.clone());
        self.error = Some(report);
    }

    pub fn script_stop_all(&mut self) {
        let mut names: Vec<String> = self.wall.members.clone();
        for name in self.slots.keys() {
            if !names.iter().any(|n| n == name) {
                names.push(name.clone());
            }
        }
        let Some(play) = self.play.as_ref() else {
            return;
        };
        let mut stopped = 0usize;
        for name in names {
            let state = play.script_state(&name);
            if matches!(state, script::RunState::Running | script::RunState::Paused) {
                play.script_stop(&name);
                stopped += 1;
            }
        }
        self.reload_generation = self.reload_generation.wrapping_add(1);
        self.reload_warning = None;
        let report = format!("Stop all: stopped {stopped}");
        if self.last_bulk_script_report.as_deref() == Some(report.as_str()) {
            return;
        }
        self.last_bulk_script_report = Some(report.clone());
        if stopped > 0 {
            self.error = None;
        }
    }

    pub fn script_reload(&mut self, commit: bool) -> ReloadOutcome {
        let Some(name) = self.focused_name() else {
            return ReloadOutcome::Failed("no focused profile".into());
        };
        let (source, lookup) = match self
            .pending_browse
            .get(&name)
            .cloned()
            .or_else(|| self.script_sel.clone())
            .or_else(|| {
                self.profile_assignment(&name)
                    .and_then(|a| sel_from_assignment(&a))
            }) {
            Some(script::ScriptSel::Loaded(source, lookup)) => (source, lookup),
            Some(script::ScriptSel::Compiled(_)) => {
                return ReloadOutcome::Failed("compiled scripts have no file to reload".into());
            }
            None => return ReloadOutcome::Failed("no script to reload".into()),
        };
        match self.js.raw_source_changed(source, &lookup) {
            Ok(false) => {
                self.error = Some(script::NOTHING_CHANGED_RELOAD.into());
                self.reload_warning = None;
                return ReloadOutcome::NothingChanged;
            }
            Ok(true) => {}
            Err(e) => {
                self.error = Some(format!("reload: {e}"));
                return ReloadOutcome::Failed(e);
            }
        }
        let key = self
            .js
            .get(source, &lookup)
            .map(|c| c.identity_key())
            .unwrap_or_default();
        let (running, paused) = self.slots_with_identity(&key);
        let warning = ReloadWarning {
            identity_key: key.clone(),
            source,
            lookup: lookup.clone(),
            running,
            paused,
            paused_during_prep: Vec::new(),
        };
        self.reload_warning = Some(warning.clone());
        if !commit {
            self.error = Some(reload_warning_text(&warning));
            return ReloadOutcome::NeedsConfirm;
        }
        let prep_gen = self.reload_generation;
        let prepared = match self.js.prepare_card(source, &lookup) {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(format!("reload: {e}"));
                return ReloadOutcome::Failed(e);
            }
        };
        if self.reload_generation != prep_gen {
            return ReloadOutcome::Failed("reload cancelled".into());
        }
        let (running_now, paused_now) = self.slots_with_identity(&key);
        let paused_during: Vec<String> = paused_now
            .iter()
            .filter(|n| warning.running.iter().any(|r| r == *n))
            .cloned()
            .collect();
        if !paused_during.is_empty() {
            if let Some(w) = self.reload_warning.as_mut() {
                w.paused_during_prep = paused_during.clone();
                w.paused = paused_now.clone();
                w.running = running_now.clone();
            }
            if !commit {
                self.error = Some(reload_warning_text(self.reload_warning.as_ref().unwrap()));
                return ReloadOutcome::NeedsConfirm;
            }
        }
        if let Err(e) = self.js.commit_prepared(prepared.clone()) {
            self.error = Some(format!("reload: {e}"));
            return ReloadOutcome::Failed(e);
        }
        let mut restarted = 0usize;
        let mut stopped_paused = 0usize;
        let targets: Vec<String> = running_now
            .into_iter()
            .chain(paused_now.into_iter())
            .collect();
        for slot_name in targets {
            let Some(play) = self.play.as_ref() else {
                break;
            };
            if play.script_source_identity(&slot_name).as_deref() != Some(key.as_str()) {
                continue;
            }
            let state = play.script_state(&slot_name);
            match state {
                script::RunState::Running => {
                    play.script_stop(&slot_name);
                    match self.script_start_profile(&slot_name) {
                        Ok(()) => restarted += 1,
                        Err(e) => {
                            self.error = Some(format!("reload {slot_name}: {e}"));
                        }
                    }
                }
                script::RunState::Paused => {
                    play.script_stop(&slot_name);
                    stopped_paused += 1;
                }
                _ => {}
            }
        }
        self.reload_warning = None;
        self.error = None;
        ReloadOutcome::Applied {
            restarted,
            stopped_paused,
        }
    }

    fn slots_with_identity(&self, key: &str) -> (Vec<String>, Vec<String>) {
        let mut running = Vec::new();
        let mut paused = Vec::new();
        let Some(play) = self.play.as_ref() else {
            return (running, paused);
        };
        let mut names: Vec<String> = self.wall.members.clone();
        for name in self.slots.keys() {
            if !names.iter().any(|n| n == name) {
                names.push(name.clone());
            }
        }
        for name in names {
            if play.script_source_identity(&name).as_deref() != Some(key) {
                continue;
            }
            match play.script_state(&name) {
                script::RunState::Running => running.push(name),
                script::RunState::Paused => paused.push(name),
                _ => {}
            }
        }
        (running, paused)
    }

    /// Re-read the configured catalog. Does not rewrite the persisted path.
    pub fn refresh_catalog(&mut self) {
        let root = match self.catalog_root() {
            Ok(Some(root)) => root,
            Ok(None) => {
                self.error = Some("Refresh catalog: no catalog configured".into());
                return;
            }
            Err(e) => {
                self.error = Some(format!("Refresh catalog: {e}"));
                return;
            }
        };
        let diff = match self.js.diff_catalog(&root) {
            Ok(d) => d,
            Err(e) => {
                self.error = Some(format!("Refresh catalog: {e}"));
                return;
            }
        };
        if diff.is_noop() {
            self.catalog_refresh_report = Some(script::NOTHING_CHANGED_CATALOG.into());
            self.error = Some(script::NOTHING_CHANGED_CATALOG.into());
            return;
        }
        let changed = diff.changed.clone();
        let removed = diff.removed.clone();
        let added = diff.added.clone();
        for name in &added {
            self.enqueue_transpile(script::ScriptSource::Catalog, name.clone(), false);
        }
        let mut reload_keys = Vec::new();
        for name in &changed {
            match self.js.prepare_card(script::ScriptSource::Catalog, name) {
                Ok(prepared) => {
                    let key = prepared.card.identity_key();
                    if let Err(e) = self.js.commit_prepared(prepared) {
                        self.error = Some(format!("catalog {name}: {e}"));
                        continue;
                    }
                    reload_keys.push(key);
                }
                Err(e) => {
                    self.error = Some(format!("catalog {name}: {e}"));
                }
            }
        }
        let report = self.js.apply_catalog_diff(diff);
        for name in &removed {
            self.mark_removed_catalog_assignments(name);
        }
        for key in reload_keys {
            let (running, paused) = self.slots_with_identity(&key);
            if let Some(play) = self.play.as_ref() {
                for n in &paused {
                    play.script_stop(n);
                }
                for n in &running {
                    play.script_stop(n);
                }
            }
            for n in running {
                let _ = self.script_start_profile(&n);
            }
        }
        let summary = report.summary();
        self.catalog_refresh_report = Some(summary.clone());
        self.error = Some(format!("Refresh catalog: {summary}"));
    }

    fn mark_removed_catalog_assignments(&mut self, card_name: &str) {
        let key =
            script::card_identity_key(script::ScriptSource::Catalog, Path::new(""), card_name);
        let names: Vec<String> = self
            .vault
            .as_ref()
            .map(|v| v.profiles().map(|p| p.username.clone()).collect())
            .unwrap_or_default();
        for name in names {
            let matches = self
                .profile_assignment(&name)
                .is_some_and(|a| a.key() == key);
            if matches {
                self.mark_assignment_unavailable(&name, format!("catalog removed: {card_name}"));
            }
        }
    }
}

fn sel_from_assignment(asg: &ScriptAssignment) -> Option<script::ScriptSel> {
    if asg.source_kind == "compiled" {
        return script::compiled_ids()
            .iter()
            .copied()
            .find(|id| id.0 == asg.identity)
            .map(script::ScriptSel::Compiled);
    }
    let source = script::parse_source_kind(&asg.source_kind)?;
    Some(script::ScriptSel::Loaded(source, asg.identity.clone()))
}

fn lookup_name(source: script::ScriptSource, name: &str, path: &Path) -> String {
    match source {
        script::ScriptSource::File => path.to_string_lossy().into_owned(),
        _ => name.to_string(),
    }
}

fn script_active_name(session: &Session, name: &str) -> bool {
    matches!(
        session
            .play
            .as_ref()
            .map(|p| p.script_state(name))
            .unwrap_or(script::RunState::Idle),
        script::RunState::Running | script::RunState::Paused | script::RunState::Stopping
    )
}

fn format_bulk(op: &str, started: usize, skipped: usize, failures: &[String]) -> String {
    if failures.is_empty() {
        return format!("{op}: started {started}, skipped {skipped}");
    }
    let shown: Vec<&str> = failures.iter().take(6).map(String::as_str).collect();
    format!(
        "{op}: started {started}, skipped {skipped}, failed {}: {}",
        failures.len(),
        shown.join("; ")
    )
}

fn reload_warning_text(w: &ReloadWarning) -> String {
    let mut parts = vec![format!(
        "reload {} will replace running bots",
        w.identity_key
    )];
    if !w.running.is_empty() {
        parts.push(format!("running: {}", w.running.join(", ")));
    }
    if !w.paused.is_empty() {
        parts.push(format!(
            "paused bots will be stopped: {}",
            w.paused.join(", ")
        ));
    }
    if !w.paused_during_prep.is_empty() {
        parts.push(format!(
            "paused during prepare: {}",
            w.paused_during_prep.join(", ")
        ));
    }
    parts.join(" — ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};
    use vault::{Profile, ProfileSettings, Vault};

    fn tmp(name: &str) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "274bot-p2-{}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
            name
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn session_with_profiles(names: &[&str]) -> (Session, std::path::PathBuf) {
        script::IsolatedEnv::ensure_thread();
        let dir = tmp("vault");
        let path = dir.join("v.vault");
        let mut vault = Vault::create(&path, "bot").unwrap();
        for (i, name) in names.iter().enumerate() {
            vault
                .upsert(Profile {
                    username: (*name).into(),
                    password: "pw".into(),
                    uid: 1 + i as i32,
                    settings: ProfileSettings::default(),
                })
                .unwrap();
        }
        let mut s = Session::new();
        s.vault = Some(vault);
        s.persist_ui = false;
        (s, dir)
    }

    #[test]
    fn focus_restores_assignment_not_other_profile_pending() {
        let (mut s, _dir) = session_with_profiles(&["alice", "bob"]);
        s.set_pending_browse(
            "alice",
            script::ScriptSel::Loaded(script::ScriptSource::Catalog, "ChickenKiller".into()),
        );
        s.persist_successful_assignment(
            "bob",
            ScriptAssignment {
                source_kind: "catalog".into(),
                identity: "Alcher".into(),
                display_name: "Alcher".into(),
                unavailable: None,
            },
        );
        s.restore_script_heading("alice");
        assert_eq!(
            s.script_sel,
            Some(script::ScriptSel::Loaded(
                script::ScriptSource::Catalog,
                "ChickenKiller".into()
            ))
        );
        s.restore_script_heading("bob");
        assert_eq!(
            s.script_sel,
            Some(script::ScriptSel::Loaded(
                script::ScriptSource::Catalog,
                "Alcher".into()
            ))
        );
        s.focus.lock().unwrap().focused = Some("bob".into());
        assert!(!s.heading_is_pending());
        s.focus.lock().unwrap().focused = Some("alice".into());
        s.restore_script_heading("alice");
        assert!(s.heading_is_pending());
    }

    #[test]
    fn two_profiles_keep_isolated_settings_after_legacy_claim() {
        let (mut s, dir) = session_with_profiles(&["alice", "bob"]);
        let store = dir.join("script-settings.json");
        fs::write(&store, r#"{"catalog:SmithingBot":{"bar":"Adamant"}}"#).unwrap();
        s.script_settings = script::ScriptSettingsStore::at(store);
        let a = s.profile_overrides("alice", "catalog:SmithingBot", "SmithingBot");
        assert_eq!(a.get("bar"), Some(&Value::String("Adamantite".into())));
        s.set_profile_setting(
            "alice",
            script::ScriptSource::Catalog,
            "SmithingBot",
            Path::new(""),
            "bar",
            Value::String("Iron".into()),
        );
        let b = s.profile_overrides("bob", "catalog:SmithingBot", "SmithingBot");
        assert_eq!(b.get("bar"), Some(&Value::String("Adamantite".into())));
        let a = s.profile_overrides("alice", "catalog:SmithingBot", "SmithingBot");
        assert_eq!(a.get("bar"), Some(&Value::String("Iron".into())));
    }

    #[test]
    fn missing_file_assignment_is_kept_not_substituted() {
        let (mut s, _dir) = session_with_profiles(&["alice"]);
        s.persist_successful_assignment(
            "alice",
            script::missing_file_assignment(
                Path::new("/tmp/missing-bot.ts"),
                "bot",
                "missing file: /tmp/missing-bot.ts",
            ),
        );
        let asg = s.profile_assignment("alice").unwrap();
        assert_eq!(asg.identity, "/tmp/missing-bot.ts");
        assert!(asg.unavailable.is_some());
        assert_ne!(asg.display_name, asg.identity);
        s.restore_script_heading("alice");
        match &s.script_sel {
            Some(script::ScriptSel::Loaded(script::ScriptSource::File, id)) => {
                assert_eq!(id, "/tmp/missing-bot.ts");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn start_all_without_play_reports_per_profile_and_is_idempotent() {
        let (mut s, _dir) = session_with_profiles(&["alice", "bob"]);
        s.wall.load("alice");
        s.wall.load("bob");
        s.script_start_all();
        let first = s.error.clone().unwrap();
        assert!(first.contains("Start all"));
        assert!(first.contains("failed"));
        s.script_start_all();
        assert_eq!(s.error.as_deref(), Some(first.as_str()));
    }

    #[test]
    fn same_stem_files_are_distinct_and_hash_whitespace() {
        script::IsolatedEnv::ensure_thread();
        let dir = tmp("files");
        let a = dir.join("one");
        let b = dir.join("two");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let fa = a.join("bot.ts");
        let fb = b.join("bot.ts");
        let src = "export default class T extends LoopingBot { override loop() {} }\n";
        fs::write(&fa, src).unwrap();
        fs::write(&fb, src).unwrap();
        let mut lib = script::JsLibrary::with_cache(dir.join("store.json"), dir.join("cache"));
        let ca = lib.load(&fa).unwrap();
        let cb = lib.load(&fb).unwrap();
        assert_ne!(ca.identity_key(), cb.identity_key());
        assert_eq!(ca.name, cb.name);
        assert!(!lib
            .raw_source_changed(script::ScriptSource::File, &fa.to_string_lossy())
            .unwrap());
        fs::write(&fa, format!("{src} ")).unwrap();
        assert!(lib
            .raw_source_changed(script::ScriptSource::File, &fa.to_string_lossy())
            .unwrap());
        assert!(!lib
            .raw_source_changed(script::ScriptSource::File, &fb.to_string_lossy())
            .unwrap());
    }

    #[test]
    fn catalog_diff_noop_and_remove_keep_assignment() {
        let (mut s, _dir) = session_with_profiles(&["alice"]);
        s.persist_successful_assignment(
            "alice",
            ScriptAssignment {
                source_kind: "catalog".into(),
                identity: "GoneBot".into(),
                display_name: "GoneBot".into(),
                unavailable: None,
            },
        );
        s.mark_removed_catalog_assignments("GoneBot");
        let asg = s.profile_assignment("alice").unwrap();
        assert_eq!(asg.identity, "GoneBot");
        assert!(asg.unavailable.unwrap().contains("catalog removed"));
    }
}
