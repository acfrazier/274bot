//! Native server-roster editor window (S9e, C6 parity).
//!
//! Every mutation stages its input on [`frontend_core::OperatorSession`] and
//! submits `AddServer`, `AddServerPreset`, `EditServer`, or `RemoveServer`
//! through the command seam. The write runs asynchronously on the single
//! SQLite writer thread and settles in `OperatorSession::poll`, so this window
//! never blocks a panel frame.

use dear_imgui_rs::{Condition, Ui};
use frontend_core::server_roster::{
    format_login_key_text, format_worlds_text, parse_login_key_text, parse_worlds_text,
    LaunchProfile, LaunchTransport,
};
use frontend_core::{OperatorCommand, Targets};

use crate::session::{PanelWindow, Session};

/// Presentation-only form buffers for the panel Servers window.
#[derive(Debug, Clone)]
pub struct ServerEditorUi {
    pub selected: Option<String>,
    pub name: String,
    pub revision: u16,
    pub transport: LaunchTransport,
    pub vault: String,
    pub worlds_text: String,
    pub login_key_text: String,
    pub members: Option<bool>,
    pub allow_plaintext_offhost: bool,
    pub form_error: Option<String>,
    pub synced_generation: u64,
}

impl Default for ServerEditorUi {
    fn default() -> Self {
        Self {
            selected: None,
            name: String::new(),
            revision: 289,
            transport: LaunchTransport::Tcp,
            vault: String::new(),
            worlds_text: "1:127.0.0.1:44594:1:127.0.0.1:1080".into(),
            login_key_text: "engine-dir".into(),
            members: None,
            allow_plaintext_offhost: false,
            form_error: None,
            synced_generation: 0,
        }
    }
}

impl ServerEditorUi {
    pub fn load_profile(&mut self, profile: &LaunchProfile) {
        self.selected = Some(profile.name.clone());
        self.name.clone_from(&profile.name);
        self.revision = profile.revision;
        self.transport = profile.transport;
        self.vault.clone_from(&profile.vault);
        self.worlds_text = format_worlds_text(&profile.worlds);
        self.login_key_text = format_login_key_text(&profile.login_key);
        self.members = profile.members;
        self.allow_plaintext_offhost = profile.allow_plaintext_offhost;
        self.form_error = None;
    }

    pub fn build_profile(&self) -> Result<LaunchProfile, String> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err("server name is required".into());
        }
        let vault = self.vault.trim().to_string();
        if vault.is_empty() {
            return Err("vault component is required".into());
        }
        let worlds = parse_worlds_text(&self.worlds_text)?;
        let login_key = parse_login_key_text(&self.login_key_text)?;
        Ok(LaunchProfile {
            name,
            revision: self.revision,
            transport: self.transport,
            worlds,
            login_key,
            vault,
            members: self.members,
            allow_plaintext_offhost: self.allow_plaintext_offhost,
        })
    }
}

/// Sync the editor form from the latest durable snapshot when opened or settled.
pub fn sync_from_roster(session: &mut Session) {
    let generation = session.core.server_roster_generation();
    let need_initial =
        session.servers_ui.selected.is_none() && session.core.server_roster().is_some();
    if !need_initial && session.servers_ui.synced_generation == generation {
        return;
    }
    session.servers_ui.synced_generation = generation;
    let chosen = session.core.server_roster().and_then(|roster| {
        let current = session.servers_ui.selected.as_deref();
        current
            .and_then(|name| {
                roster
                    .servers()
                    .servers
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(name))
            })
            .or_else(|| roster.servers().servers.first())
            .cloned()
    });
    if let Some(profile) = chosen {
        session.servers_ui.load_profile(&profile);
    }
}

/// Select `name` from the saved roster into the panel form buffers.
pub fn select(session: &mut Session, name: &str) {
    let found = session.core.server_roster().and_then(|roster| {
        roster
            .servers()
            .servers
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .cloned()
    });
    if let Some(profile) = found {
        session.servers_ui.load_profile(&profile);
    }
}

/// Queue adding a custom server profile through the shared command seam.
pub fn add_custom(session: &mut Session, profile: LaunchProfile) -> bool {
    session.core.stage_add_server(profile);
    session
        .submit(OperatorCommand::AddServer, Targets::Application)
        .is_some()
}

/// Queue adding a shipped preset by name through the shared command seam.
pub fn add_preset(session: &mut Session, preset: &str) -> bool {
    session.core.stage_add_server_preset(preset);
    session
        .submit(OperatorCommand::AddServerPreset, Targets::Application)
        .is_some()
}

/// Queue replacing `target` with `profile` through the shared command seam.
pub fn edit_server(session: &mut Session, target: &str, profile: LaunchProfile) -> bool {
    session.core.stage_edit_server(target, profile);
    session
        .submit(OperatorCommand::EditServer, Targets::Application)
        .is_some()
}

/// Queue removing `target` from the saved roster through the shared command seam.
pub fn remove_server(session: &mut Session, target: &str) -> bool {
    session.core.stage_remove_server(target);
    session
        .submit(OperatorCommand::RemoveServer, Targets::Application)
        .is_some()
}

/// Draw the non-modal Servers editor window. Never blocks on SQLite I/O.
pub fn window(ui: &Ui, session: &mut Session) {
    if !session.servers_open {
        return;
    }
    sync_from_roster(session);
    crate::app::focus_if_requested(ui, session, PanelWindow::Servers);
    let mut open = session.servers_open;
    ui.window(PanelWindow::Servers.title())
        .opened(&mut open)
        .size([520.0, 440.0], Condition::FirstUseEver)
        .build(|| {
            let pending = session.core.server_roster_pending();
            match session.core.server_roster() {
                Some(roster) => {
                    ui.text(format!(
                        "Saved roster v{} ({} servers) — edits apply on next selection.",
                        roster.version().get(),
                        roster.servers().servers.len()
                    ));
                }
                None => {
                    ui.text("Server profiles are still loading...");
                }
            }
            if pending {
                ui.text("Saving server roster...");
            }
            if let Some(status) = session.core.server_roster_status() {
                ui.text_wrapped(status);
            }
            if let Some(error) = session
                .servers_ui
                .form_error
                .as_deref()
                .or_else(|| session.core.server_roster_error())
            {
                ui.text_wrapped(format!("Error: {error}"));
            }
            ui.separator();

            let names: Vec<String> = session
                .core
                .server_roster()
                .map(|roster| {
                    roster
                        .servers()
                        .servers
                        .iter()
                        .map(|p| p.name.clone())
                        .collect()
                })
                .unwrap_or_default();
            let mut pick = None;
            for name in &names {
                let selected = session
                    .servers_ui
                    .selected
                    .as_deref()
                    .is_some_and(|cur| cur.eq_ignore_ascii_case(name));
                if ui.selectable_config(name).selected(selected).build() {
                    pick = Some(name.clone());
                }
            }
            if let Some(name) = pick {
                select(session, &name);
            }

            ui.separator();
            ui.input_text("Name", &mut session.servers_ui.name).build();
            ui.input_text("Vault", &mut session.servers_ui.vault)
                .build();
            ui.text("Revision:");
            for rev in [274u16, 289, 377] {
                ui.same_line();
                if ui.radio_button_bool(format!("{rev}"), session.servers_ui.revision == rev) {
                    session.servers_ui.revision = rev;
                }
            }
            ui.text("Transport:");
            ui.same_line();
            if ui.radio_button_bool("tcp", session.servers_ui.transport == LaunchTransport::Tcp) {
                session.servers_ui.transport = LaunchTransport::Tcp;
            }
            ui.same_line();
            if ui.radio_button_bool("wss", session.servers_ui.transport == LaunchTransport::Wss) {
                session.servers_ui.transport = LaunchTransport::Wss;
                session.servers_ui.allow_plaintext_offhost = false;
            }
            ui.input_text("Worlds", &mut session.servers_ui.worlds_text)
                .build();
            ui.input_text("Login key", &mut session.servers_ui.login_key_text)
                .build();
            ui.text("Members:");
            for (label, value) in [("auto", None), ("true", Some(true)), ("false", Some(false))] {
                ui.same_line();
                if ui.radio_button_bool(label, session.servers_ui.members == value) {
                    session.servers_ui.members = value;
                }
            }
            if session.servers_ui.transport == LaunchTransport::Tcp {
                ui.checkbox(
                    "Allow plaintext offhost",
                    &mut session.servers_ui.allow_plaintext_offhost,
                );
            }

            ui.separator();
            let disabled = ui.begin_disabled_with_cond(pending);
            if ui.button("Save edit") {
                match (
                    session.servers_ui.selected.clone(),
                    session.servers_ui.build_profile(),
                ) {
                    (Some(target), Ok(profile)) => {
                        session.servers_ui.form_error = None;
                        edit_server(session, &target, profile);
                    }
                    (None, _) => {
                        session.servers_ui.form_error =
                            Some("select a saved server to edit".into());
                    }
                    (_, Err(error)) => {
                        session.servers_ui.form_error = Some(error);
                    }
                }
            }
            ui.same_line();
            if ui.button("Add custom") {
                match session.servers_ui.build_profile() {
                    Ok(profile) => {
                        session.servers_ui.form_error = None;
                        add_custom(session, profile);
                    }
                    Err(error) => {
                        session.servers_ui.form_error = Some(error);
                    }
                }
            }
            ui.same_line();
            if ui.button("Remove") {
                if let Some(target) = session.servers_ui.selected.clone() {
                    session.servers_ui.form_error = None;
                    remove_server(session, &target);
                } else {
                    session.servers_ui.form_error = Some("select a saved server to remove".into());
                }
            }

            let presets = session.core.server_roster_available_presets();
            if !presets.is_empty() {
                ui.separator();
                ui.text("Add shipped preset:");
                for preset in presets {
                    if ui.button(format!("Add preset: {}", preset.name)) {
                        session.servers_ui.form_error = None;
                        add_preset(session, &preset.name);
                    }
                }
            }
            drop(disabled);
        });
    session.servers_open = open;
}
