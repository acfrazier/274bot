//! Servers popup: non-blocking SQLite server-roster CRUD (S9e, C6 parity).
//!
//! Mirrors the panel Servers window over the exact same `frontend-core`
//! commands (`AddServer`, `AddServerPreset`, `EditServer`, `RemoveServer`).
//! Keys return a [`ServersIntent`] that `TuiSession` submits asynchronously
//! without blocking the terminal loop on SQLite I/O.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use frontend_core::server_roster::{
    format_login_key_text, format_worlds_text, parse_login_key_text, parse_worlds_text,
    LaunchProfile, LaunchTransport, ServerRoster, StoreVersion,
};

/// Mutable presentation state for the TUI Servers overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServersState {
    pub open: bool,
    /// 0 selected server, 1 name, 2 vault, 3 revision, 4 transport, 5 worlds,
    /// 6 login_key, 7 members, 8 plaintext_offhost, 9 [save edit],
    /// 10 [add custom], 11 [remove], 12 [add shipped preset].
    pub row: usize,
    pub selected_index: usize,
    pub preset_index: usize,
    pub target_name: String,
    pub name_scratch: String,
    pub vault_scratch: String,
    pub revision: u16,
    pub transport: LaunchTransport,
    pub worlds_scratch: String,
    pub login_key_scratch: String,
    pub members: Option<bool>,
    pub allow_plaintext_offhost: bool,
    pub local_error: Option<String>,
    pub reopened: bool,
    pub synced_version: Option<StoreVersion>,
}

impl Default for ServersState {
    fn default() -> Self {
        Self {
            open: false,
            row: 0,
            selected_index: 0,
            preset_index: 0,
            target_name: String::new(),
            name_scratch: String::new(),
            vault_scratch: String::new(),
            revision: 289,
            transport: LaunchTransport::Tcp,
            worlds_scratch: "1:127.0.0.1:44594:1:127.0.0.1:1080".into(),
            login_key_scratch: "engine-dir".into(),
            members: None,
            allow_plaintext_offhost: false,
            local_error: None,
            reopened: false,
            synced_version: None,
        }
    }
}

/// What one key press in the Servers popup asks of `TuiSession`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServersIntent {
    None,
    SelectNext,
    SaveEdit,
    AddCustom,
    Remove,
    AddPreset,
    CyclePreset,
}

impl ServersState {
    pub fn reopen(&mut self) {
        self.open = true;
        self.reopened = true;
        self.row = 0;
        self.local_error = None;
    }

    pub fn fill_from_roster(&mut self, roster: &ServerRoster) {
        self.synced_version = Some(roster.version());
        let servers = &roster.servers().servers;
        if servers.is_empty() {
            self.selected_index = 0;
            self.target_name.clear();
            return;
        }
        if self.selected_index >= servers.len() {
            self.selected_index = 0;
        }
        let profile = &servers[self.selected_index];
        self.target_name.clone_from(&profile.name);
        self.name_scratch.clone_from(&profile.name);
        self.vault_scratch.clone_from(&profile.vault);
        self.revision = profile.revision;
        self.transport = profile.transport;
        self.worlds_scratch = format_worlds_text(&profile.worlds);
        self.login_key_scratch = format_login_key_text(&profile.login_key);
        self.members = profile.members;
        self.allow_plaintext_offhost = profile.allow_plaintext_offhost;
        self.local_error = None;
    }

    /// Reload only when reopened or when the durable roster version advances,
    /// keeping the operator's typed draft intact after a refused edit.
    pub fn sync_from_roster_if_changed(&mut self, roster: &ServerRoster) -> bool {
        let reopened = std::mem::take(&mut self.reopened);
        if !reopened && self.synced_version == Some(roster.version()) {
            return false;
        }
        self.fill_from_roster(roster);
        true
    }

    pub fn build_profile(&self) -> Result<LaunchProfile, String> {
        let name = self.name_scratch.trim().to_string();
        if name.is_empty() {
            return Err("server name is required".into());
        }
        let vault = self.vault_scratch.trim().to_string();
        if vault.is_empty() {
            return Err("vault component is required".into());
        }
        let worlds = parse_worlds_text(&self.worlds_scratch)?;
        let login_key = parse_login_key_text(&self.login_key_scratch)?;
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

    pub fn on_key(
        &mut self,
        key: KeyEvent,
        server_count: usize,
        preset_count: usize,
    ) -> ServersIntent {
        let is_text_row = matches!(self.row, 1 | 2 | 5 | 6);
        match key.code {
            KeyCode::Esc => {
                self.open = false;
                ServersIntent::None
            }
            KeyCode::Up => {
                self.row = self.row.saturating_sub(1);
                ServersIntent::None
            }
            KeyCode::Down | KeyCode::Tab => {
                self.row = (self.row + 1).min(12);
                ServersIntent::None
            }
            KeyCode::BackTab => {
                self.row = self.row.saturating_sub(1);
                ServersIntent::None
            }
            KeyCode::Char('k') if !is_text_row => {
                self.row = self.row.saturating_sub(1);
                ServersIntent::None
            }
            KeyCode::Char('j') if !is_text_row => {
                self.row = (self.row + 1).min(12);
                ServersIntent::None
            }
            KeyCode::Right | KeyCode::Left if self.row == 12 && preset_count > 0 => {
                self.preset_index = (self.preset_index + 1) % preset_count;
                ServersIntent::CyclePreset
            }
            KeyCode::Enter | KeyCode::Char(' ') if !is_text_row || key.code == KeyCode::Enter => {
                match self.row {
                    0 if server_count > 0 => {
                        self.selected_index = (self.selected_index + 1) % server_count;
                        ServersIntent::SelectNext
                    }
                    3 => {
                        self.revision = match self.revision {
                            274 => 289,
                            289 => 377,
                            _ => 274,
                        };
                        ServersIntent::None
                    }
                    4 => {
                        self.transport = match self.transport {
                            LaunchTransport::Tcp => {
                                self.allow_plaintext_offhost = false;
                                LaunchTransport::Wss
                            }
                            LaunchTransport::Wss => LaunchTransport::Tcp,
                        };
                        ServersIntent::None
                    }
                    7 => {
                        self.members = match self.members {
                            None => Some(true),
                            Some(true) => Some(false),
                            Some(false) => None,
                        };
                        ServersIntent::None
                    }
                    8 => {
                        if self.transport == LaunchTransport::Tcp {
                            self.allow_plaintext_offhost = !self.allow_plaintext_offhost;
                        }
                        ServersIntent::None
                    }
                    1 | 2 | 5 | 6 => {
                        self.row = (self.row + 1).min(12);
                        ServersIntent::None
                    }
                    9 => ServersIntent::SaveEdit,
                    10 => ServersIntent::AddCustom,
                    11 => ServersIntent::Remove,
                    12 if preset_count > 0 => ServersIntent::AddPreset,
                    _ => ServersIntent::None,
                }
            }
            KeyCode::Char(c) if is_text_row => {
                match self.row {
                    1 => self.name_scratch.push(c),
                    2 => self.vault_scratch.push(c),
                    5 => self.worlds_scratch.push(c),
                    6 => self.login_key_scratch.push(c),
                    _ => {}
                }
                ServersIntent::None
            }
            KeyCode::Backspace if is_text_row => {
                match self.row {
                    1 => {
                        self.name_scratch.pop();
                    }
                    2 => {
                        self.vault_scratch.pop();
                    }
                    5 => {
                        self.worlds_scratch.pop();
                    }
                    6 => {
                        self.login_key_scratch.pop();
                    }
                    _ => {}
                }
                ServersIntent::None
            }
            _ => ServersIntent::None,
        }
    }
}

/// Drawable TUI overlay for the shared durable server roster.
pub struct ServersPane<'a> {
    pub roster: Option<&'a ServerRoster>,
    pub available_presets: &'a [LaunchProfile],
    pub pending: bool,
    pub status: Option<&'a str>,
    pub error: Option<&'a str>,
    pub state: &'a ServersState,
}

impl Widget for ServersPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup = centered(area, 76, 22);
        Clear.render(popup, buf);
        let mark = |row| if self.state.row == row { "> " } else { "  " };
        let header = match self.roster {
            Some(roster) => format!(
                "Saved roster v{} ({} servers) — edits apply on next selection",
                roster.version().get(),
                roster.servers().servers.len()
            ),
            None => "Server profiles are still loading...".into(),
        };
        let selected_label = if self.state.target_name.is_empty() {
            "<none>".into()
        } else {
            self.state.target_name.clone()
        };
        let transport = match self.state.transport {
            LaunchTransport::Tcp => "tcp",
            LaunchTransport::Wss => "wss",
        };
        let members = match self.state.members {
            None => "auto",
            Some(true) => "true",
            Some(false) => "false",
        };
        let preset_label = self
            .available_presets
            .get(self.state.preset_index)
            .or_else(|| self.available_presets.first())
            .map_or_else(
                || "none available".to_string(),
                |preset| format!("{} (Left/Right to cycle)", preset.name),
            );
        let mut lines = vec![
            Line::from(header),
            Line::from(format!("{}server:    {}", mark(0), selected_label)),
            Line::from(format!("{}name:      {}", mark(1), self.state.name_scratch)),
            Line::from(format!(
                "{}vault:     {}",
                mark(2),
                self.state.vault_scratch
            )),
            Line::from(format!("{}revision:  {}", mark(3), self.state.revision)),
            Line::from(format!("{}transport: {}", mark(4), transport)),
            Line::from(format!(
                "{}worlds:    {}",
                mark(5),
                self.state.worlds_scratch
            )),
            Line::from(format!(
                "{}login key: {}",
                mark(6),
                self.state.login_key_scratch
            )),
            Line::from(format!("{}members:   {}", mark(7), members)),
            Line::from(format!(
                "{}plaintext: {}",
                mark(8),
                self.state.allow_plaintext_offhost
            )),
            Line::from(format!("{}[save edit]", mark(9))),
            Line::from(format!("{}[add custom]", mark(10))),
            Line::from(format!("{}[remove selected]", mark(11))),
            Line::from(format!("{}[add preset: {}]", mark(12), preset_label)),
        ];
        if self.pending {
            lines.push(Line::from("Saving server roster..."));
        }
        if let Some(status) = self.status {
            lines.push(Line::from(status.to_string()));
        }
        if let Some(error) = self.state.local_error.as_deref().or(self.error) {
            lines.push(Line::from(format!("Error: {error}")));
        }
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Servers (Up/Down, Enter, Esc) ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false })
            .render(popup, buf);
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2)).max(20);
    let h = height.min(area.height.saturating_sub(2)).max(10);
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn tui_servers_state_supports_add_edit_remove_and_preset_intents() {
        let mut state = ServersState::default();
        state.reopen();
        assert!(state.open);
        assert_eq!(
            state.on_key(key(KeyCode::Enter), 2, 1),
            ServersIntent::SelectNext
        );
        // Pressing Enter on text fields advances to the next row instead of saving over the selected server.
        for text_row in [1, 2, 5, 6] {
            state.row = text_row;
            assert_eq!(state.on_key(key(KeyCode::Enter), 2, 1), ServersIntent::None);
            assert_eq!(state.row, text_row + 1);
        }
        state.row = 1;
        state.name_scratch = "tui-custom".into();
        state.vault_scratch = "vault-tui-custom".into();
        let built = state.build_profile().unwrap();
        assert_eq!(built.name, "tui-custom");
        assert_eq!(built.vault, "vault-tui-custom");

        state.row = 9;
        assert_eq!(
            state.on_key(key(KeyCode::Enter), 2, 1),
            ServersIntent::SaveEdit
        );
        state.row = 10;
        assert_eq!(
            state.on_key(key(KeyCode::Enter), 2, 1),
            ServersIntent::AddCustom
        );
        state.row = 11;
        assert_eq!(
            state.on_key(key(KeyCode::Enter), 2, 1),
            ServersIntent::Remove
        );
        state.row = 12;
        assert_eq!(
            state.on_key(key(KeyCode::Enter), 2, 1),
            ServersIntent::AddPreset
        );
        assert_eq!(state.on_key(key(KeyCode::Esc), 2, 1), ServersIntent::None);
        assert!(!state.open);
    }

    #[test]
    fn tui_servers_sync_preserves_draft_while_durable_version_is_unchanged() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "274bot-tui-servers-draft-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let root = host_play::local_state::StateRoot::new(dir.clone());
        let service = host_play::local_state::LocalStateService::open_process(root);
        let roster_v1 = service
            .wait_server_roster(std::time::Duration::from_secs(5))
            .unwrap();
        let mut state = ServersState::default();
        state.reopen();
        assert!(state.sync_from_roster_if_changed(&roster_v1));
        assert_eq!(state.synced_version, Some(roster_v1.version()));

        // Operator types a draft; a refused edit keeps the same durable version.
        state.name_scratch = "draft-tui-srv".into();
        state.vault_scratch = "vault-draft-tui".into();
        assert!(!state.sync_from_roster_if_changed(&roster_v1));
        assert_eq!(state.name_scratch, "draft-tui-srv");
        assert_eq!(state.vault_scratch, "vault-draft-tui");

        // Advancing the durable version reloads from the committed roster.
        let mut custom = roster_v1.servers().servers[0].clone();
        custom.name = "committed-tui-srv".into();
        custom.vault = "vault-committed-tui".into();
        let roster_v2 = service
            .add_server(
                roster_v1.version(),
                custom,
                std::time::Duration::from_secs(5),
            )
            .unwrap();
        assert!(state.sync_from_roster_if_changed(&roster_v2));
        assert_eq!(state.synced_version, Some(roster_v2.version()));
        assert_ne!(state.name_scratch, "draft-tui-srv");
        drop(service);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
