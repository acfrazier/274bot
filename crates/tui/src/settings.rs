//! Settings popup (spec `2026-09-01-headless-tui-design.md`): an overlay
//! (`o` on the Overview, or the palette) with the selected profile's
//! random-event toggle, per-event Guardian policy, and lamp reward settings,
//! plus durable global walk grants (teleports / wilderness / danger zones),
//! manual WalkTo bank fetch, and the remembered terrain-bake choice shared
//! with the panel. Profile settings return typed edits for the application
//! to stage through the command seam (`--live` stays ephemeral). Not crowding
//! the main view — a small centered box drawn after the panes.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, Widget};

use frontend_core::quester_paths::{QuesterPathsView, LOAD_PATHS_LABEL, RELOAD_PATHS_LABEL};
use frontend_core::{CurePoison, GuardianPolicy, Setting, SettingsDraft, GUARDIAN_EVENTS};
use frontend_core::{
    FormNotice, MapBakeChoice, MemoryNotice, NavPreference, BANK_FETCH_PERMISSION_SCOPE,
    GLOBAL_DANGER_WARNING, GLOBAL_PERMISSION_LABELS, NOTHING_SAVED, QUESTER_HEADING,
    QUEST_PATHS_HEADING, ROUTING_SCOPE_NOTE, SCRIPT_SCOPE_NOTICE,
};

use crate::app::SettingFeedback;

/// The popup's title while no profile is bound to it.
pub const TITLE: &str = "settings";
const EVENT_POLICY_UNAVAILABLE: &str =
    "Fight and Ignore are unavailable; the Guardian treats saved choices as Flee.";

/// The last line of a selected setting taller than the viewport: which of
/// its lines are hidden and the key that shows them.
const MORE_BELOW: &str = "↓ more of this setting below (Down)";
const MORE_ABOVE: &str = "↑ more of this setting above (Up)";
const MORE_ABOVE_AND_BELOW: &str = "↕ more of this setting above (Up) and below (Down)";
/// The first line of the notice area under the rows when the notices need
/// more lines than the popup leaves them: which notices are hidden and the
/// key that shows them.
const NOTICES_ABOVE: &str = "↑ more notices above (PgUp)";
const NOTICES_BELOW: &str = "↓ more notices below (PgDn)";
const NOTICES_ABOVE_AND_BELOW: &str = "↕ more notices above (PgUp) and below (PgDn)";
/// Drawn on the popup's bottom border when the popup leaves no line for any
/// notice under the rows.
const NOTICES_CUT: &str = " not every notice fits: make the terminal taller ";

/// Lamp skills the popup cycles, in display order.
pub const LAMP_SKILLS: [&str; 7] = [
    "attack",
    "strength",
    "defence",
    "hitpoints",
    "ranged",
    "prayer",
    "magic",
];

/// Mutable settings-pane state: whether the popup is open and which row
/// the operator is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsState {
    pub open: bool,
    /// 0=random events, 1=lamp skill, 2=lamp auto, 3=event, 4=event policy,
    /// 5=teleports, 6=wilderness, 7=bank fetch, 8=danger zones,
    /// 9=manual-walk pause, 10=map bake, 11=memory, 12=folder gate,
    /// 13=paths folder, 14=reload paths, 15=cure poison.
    pub row: usize,
    /// Selected per-event policy row.
    pub random_event_index: usize,
    /// Path folder text input is active.
    pub folder_editing: bool,
    /// Text changed during this folder edit and needs persistence when closed.
    pub folder_changed: bool,
    /// A process-owned shared-preferences save has not settled yet.
    pub preferences_pending: bool,
    /// Hit targets from the last drawn viewport, including wrapped rows.
    pub row_areas: [Rect; 16],
    /// The row whose own lines [`Self::detail_scroll`] scrolls.
    pub detail_row: usize,
    /// Lines of the selected setting scrolled out above the viewport when
    /// the setting (with its saved state or failure) is taller than the
    /// viewport. Up and Down scroll these lines before they move the row.
    pub detail_scroll: u16,
    /// The largest [`Self::detail_scroll`] the last drawn viewport allowed
    /// (0 when the selected setting fits).
    pub detail_max: u16,
    /// Notice lines hidden below the notice area under the rows, when the
    /// notices need more lines than the popup leaves them. PageUp and
    /// PageDown scroll them. The area first shows the last notices.
    pub notice_scroll: u16,
    /// The largest [`Self::notice_scroll`] the last drawn popup allowed (0
    /// when every notice fits).
    pub notice_max: u16,
    /// Lines one PageUp or PageDown scrolls: the height of the last drawn
    /// notice window.
    pub notice_page: u16,
}

impl SettingsState {
    pub(crate) fn row_at(&self, col: u16, row: u16) -> Option<usize> {
        self.row_areas.iter().position(|area| {
            col >= area.x && col < area.right() && row >= area.y && row < area.bottom()
        })
    }
}

/// One profile-setting value selected by the popup. The app stages it and
/// submits `EditProfileDraft(Settings)` in the same dispatch handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsEdit {
    RandomEvents(bool),
    LampSkill(&'static str),
    LampAuto(bool),
    RandomEventPolicy {
        event: &'static str,
        policy: GuardianPolicy,
    },
    Lowmem(bool),
    CurePoison(CurePoison),
}

/// The outcome of one settings key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsKey {
    /// A profile setting edit for the app to stage and submit.
    Changed(SettingsEdit),
    /// An unavailable policy gets a Flee edit and a reason to show.
    ChangedWithNotice(SettingsEdit, &'static str),
    /// The selected setting is unavailable; show the refusal reason.
    Refused(&'static str),
    /// The remembered map-bake choice changed; persist it to the shared
    /// prefs (`panel-ui.json`).
    MapBake,
    /// A durable global walk permission changed.
    WalkGlobal(NavPreference),
    /// The one-time script-scope notice was dismissed.
    ScriptScopeNoticeAck,
    /// The one-time danger-routing migration notice was dismissed.
    SurvivableRoutingNoticeAck,
    /// The global manual-walk pause preference changed and needs persistence.
    PauseScriptOnManualWalkAbort,
    /// The Quester folder settings changed and need shared persistence.
    QuesterPathsChanged,
    /// Reload the shared Path registry without starting a Path.
    ReloadPaths,
    /// Relog the bound member now to apply the entire queued memory mode.
    MemoryRelog,
    /// The key was consumed but nothing changed (navigation, Esc).
    Consumed,
    /// Not a settings key.
    Ignored,
}

/// The settings popup widget over a [`SettingsDraft`]. `title` names the
/// bound profile (for example `settings — alice`). `notice` is the bound
/// profile's save feedback, drawn under the rows: a refusal or a failed
/// write in red with [`NOTHING_SAVED`], or `Saved <name>.` in green once the
/// write is durable. `memory` is the bound slot's applied vs queued mode;
/// changes take effect only at the next login, with an explicit `r` offer.
/// The notices are drawn into the same buffer as the settings rows.
pub struct SettingsPane<'a> {
    pub settings: &'a SettingsDraft,
    pub nav: &'a mut frontend_core::WalkGlobals,
    pub map_bake: &'a mut MapBakeChoice,
    pub state: &'a mut SettingsState,
    pub pause_script_on_manual_walk_abort: Option<&'a mut bool>,
    pub script_scope_notice_ack: Option<&'a mut bool>,
    pub survivable_routing_notice_ack: Option<&'a mut bool>,
    pub quester_paths: Option<&'a mut QuesterPathsView>,
    pub quester_paths_notice: Option<&'a str>,
    pub quester_paths_notice_error: bool,
    pub quester_paths_reloading: bool,
    pub title: &'a str,
    pub notice: Option<&'a FormNotice>,
    pub memory: Option<MemoryNotice>,
    /// What the core shows beside each saved setting: pending, or the
    /// retained failure (copied from the core each pump).
    pub feedback: &'a [(Setting, SettingFeedback)],
}

impl<'a> SettingsPane<'a> {
    pub fn new(
        settings: &'a SettingsDraft,
        nav: &'a mut frontend_core::WalkGlobals,
        map_bake: &'a mut MapBakeChoice,
        state: &'a mut SettingsState,
    ) -> Self {
        Self {
            settings,
            nav,
            map_bake,
            state,
            title: TITLE,
            notice: None,
            memory: None,
            feedback: &[],
            pause_script_on_manual_walk_abort: None,
            script_scope_notice_ack: None,
            survivable_routing_notice_ack: None,
            quester_paths: None,
            quester_paths_notice: None,
            quester_paths_notice_error: false,
            quester_paths_reloading: false,
        }
    }
    /// Bind the shared script-scope acknowledgement.
    pub fn script_scope_notice_ack(mut self, value: &'a mut bool) -> Self {
        self.script_scope_notice_ack = Some(value);
        self
    }

    /// Bind the independent survivable-routing notice acknowledgement.
    pub fn survivable_routing_notice_ack(mut self, value: &'a mut bool) -> Self {
        self.survivable_routing_notice_ack = Some(value);
        self
    }

    /// Bind the shared global pause preference to the settings row.
    pub fn pause_script_on_manual_walk_abort(mut self, value: &'a mut bool) -> Self {
        self.pause_script_on_manual_walk_abort = Some(value);
        self
    }

    /// Bind shared Quester Path settings.
    pub fn quester_paths(mut self, value: &'a mut QuesterPathsView) -> Self {
        self.quester_paths = Some(value);
        self
    }

    /// One key while the popup is open. Up/Down move the row; when the
    /// selected setting (with its saved state or failure) is taller than the
    /// viewport they scroll its own lines first. PageUp/PageDown scroll the
    /// notices under the rows when they do not all fit. Enter/Space toggles
    /// settings, `d` dismisses the script-scope notice, and `n` dismisses the
    /// independent survivable-routing notice; `r` relogs the bound member
    /// only while its login mode differs and no relog is queued. Esc closes.
    pub fn on_key(&mut self, key: KeyEvent) -> SettingsKey {
        if self.state.folder_editing {
            return self.folder_key(key);
        }
        match key.code {
            KeyCode::Char('r') if self.memory.is_some_and(MemoryNotice::can_relog) => {
                SettingsKey::MemoryRelog
            }
            KeyCode::Char('r') => SettingsKey::Consumed,
            KeyCode::Char('d')
                if self
                    .script_scope_notice_ack
                    .as_deref()
                    .is_some_and(|ack| !*ack) =>
            {
                SettingsKey::ScriptScopeNoticeAck
            }
            KeyCode::Char('n')
                if self
                    .survivable_routing_notice_ack
                    .as_deref()
                    .is_some_and(|ack| !*ack) =>
            {
                SettingsKey::SurvivableRoutingNoticeAck
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.state.detail_row == self.state.row && self.state.detail_scroll > 0 {
                    self.state.detail_scroll -= 1;
                } else {
                    self.state.row = self.state.row.saturating_sub(1);
                    self.reset_detail();
                }
                SettingsKey::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.state.detail_row == self.state.row
                    && self.state.detail_scroll < self.state.detail_max
                {
                    self.state.detail_scroll += 1;
                } else {
                    self.state.row = (self.state.row + 1).min(15);
                    self.reset_detail();
                }
                SettingsKey::Consumed
            }
            KeyCode::PageUp => {
                self.state.notice_scroll = self
                    .state
                    .notice_scroll
                    .saturating_add(self.state.notice_page.max(1))
                    .min(self.state.notice_max);
                SettingsKey::Consumed
            }
            KeyCode::PageDown => {
                self.state.notice_scroll = self
                    .state
                    .notice_scroll
                    .saturating_sub(self.state.notice_page.max(1));
                SettingsKey::Consumed
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Char(' ') if self.state.row == 14 => SettingsKey::Consumed,
            KeyCode::Char(' ') => self.activate(),
            KeyCode::Esc => {
                self.state.open = false;
                SettingsKey::Consumed
            }
            _ => SettingsKey::Ignored,
        }
    }

    /// The selected row changed: its own lines start at its top.
    fn reset_detail(&mut self) {
        self.state.detail_row = self.state.row;
        self.state.detail_scroll = 0;
        self.state.detail_max = 0;
    }

    fn folder_key(&mut self, key: KeyEvent) -> SettingsKey {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.state.folder_editing = false;
                if std::mem::take(&mut self.state.folder_changed) {
                    SettingsKey::QuesterPathsChanged
                } else {
                    SettingsKey::Consumed
                }
            }
            KeyCode::Backspace => {
                let Some(paths) = self.quester_paths.as_deref_mut() else {
                    return SettingsKey::Consumed;
                };
                let mut folder = paths.folder().to_string_lossy().into_owned();
                if folder.pop().is_some() {
                    paths.set_folder(folder.into());
                    self.state.folder_changed = true;
                }
                SettingsKey::Consumed
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                if let Some(paths) = self.quester_paths.as_deref_mut() {
                    let mut folder = paths.folder().to_string_lossy().into_owned();
                    folder.push(character);
                    paths.set_folder(folder.into());
                    self.state.folder_changed = true;
                }
                SettingsKey::Consumed
            }
            _ => SettingsKey::Consumed,
        }
    }
    fn activate(&mut self) -> SettingsKey {
        match self.state.row {
            0 => SettingsKey::Changed(SettingsEdit::RandomEvents(!self.settings.random_events())),
            1 => {
                let next = LAMP_SKILLS
                    .iter()
                    .position(|skill| *skill == self.settings.lamp_skill())
                    .map(|index| (index + 1) % LAMP_SKILLS.len())
                    .unwrap_or(0);
                SettingsKey::Changed(SettingsEdit::LampSkill(LAMP_SKILLS[next]))
            }
            2 => SettingsKey::Changed(SettingsEdit::LampAuto(!self.settings.lamp_auto())),
            3 => {
                self.activate_value();
                SettingsKey::Consumed
            }
            4 => self.activate_event_policy(),
            5 => {
                self.activate_value();
                SettingsKey::WalkGlobal(NavPreference::AllowTeleports)
            }
            6 => {
                self.activate_value();
                SettingsKey::WalkGlobal(NavPreference::AllowWilderness)
            }
            7 => {
                self.activate_value();
                SettingsKey::WalkGlobal(NavPreference::AllowBankFetch)
            }
            8 => {
                self.activate_value();
                SettingsKey::WalkGlobal(NavPreference::SurvivableRouting)
            }
            9 if self.pause_script_on_manual_walk_abort.is_some() => {
                self.activate_value();
                SettingsKey::PauseScriptOnManualWalkAbort
            }
            10 => {
                self.activate_value();
                SettingsKey::MapBake
            }
            11 => SettingsKey::Changed(SettingsEdit::Lowmem(!self.settings.lowmem())),
            12 => {
                let Some(paths) = self.quester_paths.as_deref_mut() else {
                    return SettingsKey::Consumed;
                };
                paths.set_enabled(!paths.enabled());
                SettingsKey::QuesterPathsChanged
            }
            13 => {
                if self.quester_paths.is_none() {
                    return SettingsKey::Consumed;
                }
                self.state.folder_editing = true;
                self.state.folder_changed = false;
                SettingsKey::Consumed
            }
            14 => SettingsKey::ReloadPaths,
            15 => SettingsKey::Changed(SettingsEdit::CurePoison(
                self.settings.survival().cure_poison().next(),
            )),
            _ => SettingsKey::Consumed,
        }
    }
    fn activate_event_policy(&mut self) -> SettingsKey {
        let (event, _) = GUARDIAN_EVENTS[self.state.random_event_index % GUARDIAN_EVENTS.len()];
        if self.settings.random_event_policy(event) == GuardianPolicy::Flee {
            return SettingsKey::Refused(EVENT_POLICY_UNAVAILABLE);
        }
        SettingsKey::ChangedWithNotice(
            SettingsEdit::RandomEventPolicy {
                event,
                policy: GuardianPolicy::Flee,
            },
            EVENT_POLICY_UNAVAILABLE,
        )
    }

    fn activate_value(&mut self) {
        match self.state.row {
            3 => {
                self.state.random_event_index =
                    (self.state.random_event_index + 1) % GUARDIAN_EVENTS.len();
            }
            5 => {
                let nav = *self.nav;
                *self.nav = frontend_core::WalkGlobals::new(
                    !nav.allow_teleports(),
                    nav.allow_wilderness(),
                    nav.allow_bank_fetch(),
                    nav.allow_danger_zones(),
                    nav.survivable_routing(),
                );
            }
            6 => {
                let nav = *self.nav;
                *self.nav = frontend_core::WalkGlobals::new(
                    nav.allow_teleports(),
                    !nav.allow_wilderness(),
                    nav.allow_bank_fetch(),
                    nav.allow_danger_zones(),
                    nav.survivable_routing(),
                );
            }
            7 => {
                let nav = *self.nav;
                *self.nav = frontend_core::WalkGlobals::new(
                    nav.allow_teleports(),
                    nav.allow_wilderness(),
                    !nav.allow_bank_fetch(),
                    nav.allow_danger_zones(),
                    nav.survivable_routing(),
                );
            }
            8 => {
                let next = self.nav.danger_level().next();
                *self.nav = self.nav.with_danger_level(next);
            }
            9 => {
                if let Some(pause) = self.pause_script_on_manual_walk_abort.as_deref_mut() {
                    *pause = !*pause;
                }
            }
            10 => *self.map_bake = self.map_bake.toggled(),
            _ => {}
        }
    }
}

impl Widget for SettingsPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.state.row_areas.fill(Rect::default());
        // The popup needs two lines inside its border: a line of the
        // selected setting and the line that says what of it is hidden.
        if !self.state.open || area.width < 3 || area.height < 5 {
            return;
        }
        let memory_note = self
            .memory
            .filter(|n| n.differs())
            .map_or(MemoryNotice::NEXT_LOGIN_NOTE, |n| n.notice_text());
        let show_scope_notice = self
            .script_scope_notice_ack
            .as_deref()
            .is_some_and(|ack| !*ack);
        let show_survivable_routing_notice = self
            .survivable_routing_notice_ack
            .as_deref()
            .is_some_and(|ack| !*ack);
        let danger_level = self.nav.danger_level();
        let selected_row = self.state.row.min(15);
        let danger_help = (selected_row == 8
            && danger_level == frontend_core::DangerLevel::WhenSurvivable)
            .then_some(frontend_core::walk_permissions::survivable_routing_tooltip());
        let path_folder = self
            .quester_paths
            .as_deref()
            .map(|paths| paths.folder().display().to_string())
            .unwrap_or_else(|| "unavailable".into());
        let paths_enabled = self
            .quester_paths
            .as_deref()
            .is_some_and(|paths| paths.enabled());
        let reload = if self.quester_paths_reloading {
            format!("{} (reloading…)", RELOAD_PATHS_LABEL)
        } else {
            RELOAD_PATHS_LABEL.to_string()
        };
        let folder_hint = if self.state.folder_editing {
            " [type path; Enter/Esc done]"
        } else {
            " [Enter edit]"
        };
        let event_index = self.state.random_event_index % GUARDIAN_EVENTS.len();
        let (event_key, event_label) = GUARDIAN_EVENTS[event_index];
        let event_policy = match self.settings.random_event_policy(event_key) {
            GuardianPolicy::Flee => "Flee",
            GuardianPolicy::Fight => "Fight (not available yet)",
            GuardianPolicy::Ignore => "Ignore (not available yet)",
        };
        let marker = |row| if selected_row == row { "> " } else { "  " };

        let rows = [
            format!(
                "{}random events: {}",
                marker(0),
                self.settings.random_events()
            ),
            format!("{}lamp skill: {}", marker(1), self.settings.lamp_skill()),
            format!("{}lamp auto: {}", marker(2), self.settings.lamp_auto()),
            format!("{}event: {}", marker(3), event_label),
            format!("{}event policy: {event_policy}", marker(4)),
            format!(
                "{}{}: {}",
                marker(5),
                GLOBAL_PERMISSION_LABELS[0].1,
                self.nav.allow_teleports()
            ),
            format!(
                "{}{}: {}",
                marker(6),
                GLOBAL_PERMISSION_LABELS[1].1,
                self.nav.allow_wilderness()
            ),
            format!(
                "{}{}: {} · {}",
                marker(7),
                GLOBAL_PERMISSION_LABELS[2].1,
                self.nav.allow_bank_fetch(),
                BANK_FETCH_PERMISSION_SCOPE
            ),
            if danger_level == frontend_core::DangerLevel::Always {
                format!(
                    "{}{}: {} — {}",
                    marker(8),
                    GLOBAL_PERMISSION_LABELS[3].1,
                    frontend_core::walk_permissions::danger_routing_label(danger_level),
                    GLOBAL_DANGER_WARNING
                )
            } else {
                format!(
                    "{}{}: {}",
                    marker(8),
                    GLOBAL_PERMISSION_LABELS[3].1,
                    frontend_core::walk_permissions::danger_routing_label(danger_level)
                )
            },
            format!(
                "{}Pause script on manual movement: {}",
                marker(9),
                self.pause_script_on_manual_walk_abort
                    .as_deref()
                    .copied()
                    .unwrap_or(true)
            ),
            format!("{}map bake: {}", marker(10), self.map_bake.as_str()),
            format!(
                "{}memory: {}",
                marker(11),
                MemoryNotice::status_text(self.settings.lowmem(), self.memory)
            ),
            format!("{}{}: {}", marker(12), LOAD_PATHS_LABEL, paths_enabled),
            format!("{}Paths folder: {}{}", marker(13), path_folder, folder_hint),
            format!("{}{}", marker(14), reload),
            format!(
                "{}cure poison: {}",
                marker(15),
                self.settings.survival().cure_poison().label()
            ),
        ];
        // What follows each row while its setting is pending or failed:
        // pieces borrowed from the retained feedback and drawn after the
        // row's own text, so a frame copies none of it.
        let row_feedback: [Feedback<'_>; 16] = std::array::from_fn(|index| {
            row_settings(index)
                .iter()
                .find_map(|setting| feedback_parts(self.feedback, *setting))
                .unwrap_or(NO_FEEDBACK)
        });
        let quester_heading = format!("{QUESTER_HEADING}: {QUEST_PATHS_HEADING}");
        let note_style = Style::default().fg(Color::DarkGray);
        let heading_style = Style::default().add_modifier(Modifier::BOLD);
        // Lead-ins sit above their first row. A lead-in adds to its row's block
        // height, but the row keeps its own index, hit target, and scroll slot.
        let headers: [Option<(&str, Style)>; 16] = [
            None,
            None,
            None,
            None,
            Some((
                "Fight/Ignore are unavailable; the Guardian treats saved choices as Flee.",
                note_style,
            )),
            Some((ROUTING_SCOPE_NOTE, note_style)),
            None,
            None,
            None,
            None,
            None,
            None,
            Some((quester_heading.as_str(), heading_style)),
            None,
            None,
            Some(("Survival and supplies", heading_style)),
        ];
        let width = area.width.min(100);
        let columns = usize::from(width - 2);
        let row_heights: [u16; 16] = std::array::from_fn(|index| {
            wrapped_rows_parts(
                [
                    rows[index].as_str(),
                    row_feedback[index][0],
                    row_feedback[index][1],
                ],
                columns,
            )
        });
        let header_heights =
            headers.map(|header| header.map_or(0, |(text, _)| wrapped_rows(text, columns)));
        let block_heights: [u16; 16] =
            std::array::from_fn(|index| header_heights[index] + row_heights[index]);
        let selected = self.state.row.min(rows.len() - 1);
        let memory_height = wrapped_rows(memory_note, columns);
        let scope_dismiss = dismiss_parts(
            "[d] dismiss this notice",
            self.feedback,
            Setting::Nav(frontend_core::NavPreference::ScriptScopeNoticeAck),
        );
        let migration_dismiss = dismiss_parts(
            "[n] dismiss this notice",
            self.feedback,
            Setting::Nav(frontend_core::NavPreference::SurvivableRoutingNoticeAck),
        );
        let scope_height = if show_scope_notice {
            wrapped_rows(SCRIPT_SCOPE_NOTICE, columns) + wrapped_rows_parts(scope_dismiss, columns)
        } else {
            0
        };
        // The dismiss line sits under its notice and carries the notice's
        // saving state or failure.
        let migration_notice_height = if show_survivable_routing_notice {
            wrapped_rows(
                frontend_core::walk_permissions::survivable_routing_notice(),
                columns,
            ) + wrapped_rows_parts(migration_dismiss, columns)
        } else {
            0
        };
        let help_height = danger_help.map_or(0, |help| wrapped_rows(help, columns));
        let notice_height = self.notice.map_or(0, |notice| {
            notice.error().map_or_else(
                || wrapped_rows(notice.text(), columns),
                |reason| wrapped_rows(reason, columns) + wrapped_rows(NOTHING_SAVED, columns),
            )
        });
        let path_notice_height = self
            .quester_paths_notice
            .map_or(0, |notice| wrapped_rows(notice, columns));
        let height = (block_heights.iter().sum::<u16>()
            + memory_height
            + help_height
            + migration_notice_height
            + scope_height
            + notice_height
            + path_notice_height
            + 2)
        .min(area.height - 1);
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        Clear.render(popup, buf);
        let mut block = Block::default().borders(Borders::ALL).title(self.title);
        if self.state.preferences_pending {
            block = block.title_bottom(" Saving preferences… ");
        }
        let inner = block.inner(popup);
        block.render(popup, buf);
        // The notices below the rows share the popup with at least two lines
        // of the rows: a window of the selected setting and its sign. When
        // the notices need more lines than that leaves, their area shows a
        // window of them (its newest lines, at the bottom, first) under a
        // sign that says which part is hidden and the key that shows it.
        // PageUp and PageDown scroll that window. Nothing is cut silently.
        let footer_need = memory_height
            + help_height
            + migration_notice_height
            + scope_height
            + notice_height
            + path_notice_height;
        let footer_height = footer_need.min(inner.height.saturating_sub(2));
        let notices_cut = footer_need > footer_height;
        // The sign takes the area's first line when the area has a line for
        // it and one more for the notices; else it goes on the bottom border.
        let sign_inside = notices_cut && footer_height >= 2;
        let notice_window = footer_height - u16::from(sign_inside);
        let notice_max = footer_need - notice_window;
        let notice_scroll = self.state.notice_scroll.min(notice_max);
        self.state.notice_scroll = notice_scroll;
        self.state.notice_max = notice_max;
        self.state.notice_page = notice_window.max(1);
        let rows_height = inner.height - footer_height;
        let rows_bottom = inner.y + rows_height;
        if notices_cut {
            let hidden_above = notice_max > notice_scroll;
            let sign = match (hidden_above, notice_scroll > 0) {
                (true, true) => NOTICES_ABOVE_AND_BELOW,
                (true, false) => NOTICES_ABOVE,
                (false, _) => NOTICES_BELOW,
            };
            let yellow = Style::default().fg(Color::Yellow);
            if sign_inside {
                buf.set_stringn(
                    inner.x,
                    rows_bottom,
                    sign,
                    usize::from(inner.width),
                    Style::default().fg(Color::DarkGray),
                );
            } else if footer_height == 1 {
                buf.set_stringn(
                    popup.x + 1,
                    popup.bottom() - 1,
                    sign,
                    usize::from(popup.width.saturating_sub(2)),
                    yellow,
                );
            } else {
                // No line is left for any notice (a popup with two lines
                // inside its border).
                let marker = Span::raw(NOTICES_CUT).width();
                let marker = u16::try_from(marker).unwrap_or(u16::MAX);
                buf.set_stringn(
                    popup.x + popup.width.saturating_sub(marker.saturating_add(1)).max(1),
                    popup.bottom() - 1,
                    NOTICES_CUT,
                    usize::from(popup.width.saturating_sub(2)),
                    yellow,
                );
            }
        }

        let row_style = |index: usize| {
            if index == 8 && danger_level == frontend_core::DangerLevel::Always {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            }
        };
        if self.state.detail_row != selected {
            self.state.detail_row = selected;
            self.state.detail_scroll = 0;
        }
        let selected_height = block_heights[selected];
        if selected_height > rows_height {
            // The selected setting (a long failure on a small terminal) is
            // taller than the viewport. The viewport shows a window of its
            // own lines, Up and Down scroll that window before they move the
            // row, and the last line says which part is hidden, so nothing
            // of the setting is lost without a sign. The rows always have a
            // line for the window and a line for the sign.
            let window = rows_height - 1;
            let max_scroll = selected_height - window;
            let scroll = self.state.detail_scroll.min(max_scroll);
            self.state.detail_scroll = scroll;
            self.state.detail_max = max_scroll;
            self.state.row_areas[selected] = Rect::new(inner.x, inner.y, inner.width, window);
            draw_scrolled_block(
                buf,
                inner,
                inner.y + window,
                headers[selected],
                [
                    rows[selected].as_str(),
                    row_feedback[selected][0],
                    row_feedback[selected][1],
                ],
                row_style(selected),
                scroll,
            );
            let hint = match (scroll > 0, scroll < max_scroll) {
                (true, true) => MORE_ABOVE_AND_BELOW,
                (true, false) => MORE_ABOVE,
                (false, _) => MORE_BELOW,
            };
            buf.set_stringn(
                inner.x,
                rows_bottom - 1,
                hint,
                usize::from(inner.width),
                Style::default().fg(Color::DarkGray),
            );
        } else {
            self.state.detail_scroll = 0;
            self.state.detail_max = 0;
            let mut first = 0;
            let mut through_selected = block_heights[..=selected].iter().sum::<u16>();
            while first < selected && through_selected > rows_height {
                through_selected -= block_heights[first];
                first += 1;
            }
            let mut y = inner.y;
            for (index, text) in rows.iter().enumerate().skip(first) {
                if y >= rows_bottom {
                    break;
                }
                if let Some((header, style)) = headers[index] {
                    y = draw_wrapped(buf, inner, y, rows_bottom, header, style);
                    if y >= rows_bottom {
                        break;
                    }
                }
                let bottom = (y + row_heights[index]).min(rows_bottom);
                self.state.row_areas[index] = Rect::new(inner.x, y, inner.width, bottom - y);
                y = draw_wrapped_parts(
                    buf,
                    inner,
                    y,
                    bottom,
                    [
                        text.as_str(),
                        row_feedback[index][0],
                        row_feedback[index][1],
                    ],
                    row_style(index),
                );
            }
        }

        // The notices, in order, as one column of lines of which the area
        // shows the window `notice_scroll` lines above its end.
        let yellow = Style::default().fg(Color::Yellow);
        let mut notices = NoticeArea {
            x: inner.x,
            width: usize::from(inner.width).max(1),
            top: rows_bottom + u16::from(sign_inside),
            bottom: inner.bottom(),
            first: notice_max - notice_scroll,
            line: 0,
        };
        notices.text(buf, memory_note, yellow);
        if let Some(help) = danger_help {
            notices.text(buf, help, yellow);
        }
        if show_survivable_routing_notice {
            notices.text(
                buf,
                frontend_core::walk_permissions::survivable_routing_notice(),
                yellow,
            );
            notices.parts(buf, migration_dismiss, yellow);
        }
        if show_scope_notice {
            notices.text(buf, SCRIPT_SCOPE_NOTICE, yellow);
            notices.parts(buf, scope_dismiss, yellow);
        }
        if let Some(notice) = self.notice {
            match notice.error() {
                Some(reason) => {
                    let red = Style::default().fg(Color::Red);
                    notices.text(buf, reason, red);
                    notices.text(buf, NOTHING_SAVED, red);
                }
                None => notices.text(buf, notice.text(), Style::default().fg(Color::Green)),
            }
        }
        if let Some(notice) = self.quester_paths_notice {
            let color = if self.quester_paths_notice_error {
                Color::Red
            } else {
                Color::Green
            };
            notices.text(buf, notice, Style::default().fg(color));
        }
    }
}

/// The notices under the popup's rows as one column of wrapped lines, of
/// which the lines `first..` show from `top` down to `bottom`. Each piece of
/// text follows the one before it. Borrows the text: allocates nothing.
struct NoticeArea {
    x: u16,
    width: usize,
    top: u16,
    bottom: u16,
    /// The first line of the column the area shows.
    first: u16,
    /// The column line the next piece of text starts on.
    line: u16,
}

impl NoticeArea {
    /// The row `y` the column line `line` shows on, if it shows.
    fn row(&self, line: u16) -> Option<u16> {
        let y = self.top.checked_add(line.checked_sub(self.first)?)?;
        (y < self.bottom).then_some(y)
    }

    fn text(&mut self, buf: &mut Buffer, text: &str, style: Style) {
        for line in wrapped_lines(text, self.width) {
            if let Some(y) = self.row(self.line) {
                buf.set_stringn(self.x, y, line, self.width, style);
            }
            self.line = self.line.saturating_add(1);
        }
    }

    fn parts<const N: usize>(&mut self, buf: &mut Buffer, parts: [&str; N], style: Style) {
        for row in wrapped_parts(parts, self.width) {
            if let Some(y) = self.row(self.line) {
                draw_slices(buf, self.x, y, self.width, row, style);
            }
            self.line = self.line.saturating_add(1);
        }
    }
}

/// The width in columns of one character, as [`Span`] measures it.
fn char_width(text: &str, at: usize, ch: char) -> usize {
    Span::raw(&text[at..at + ch.len_utf8()]).width()
}

/// Borrow character-wrapped rows without copying the text.
pub(crate) fn wrapped_lines(text: &str, width: usize) -> impl Iterator<Item = &str> {
    let mut remaining = text;
    let width = width.max(1);
    std::iter::from_fn(move || {
        if remaining.is_empty() {
            return None;
        }
        let mut used = 0;
        let mut end = remaining.len();
        for (at, ch) in remaining.char_indices() {
            let columns = char_width(remaining, at, ch);
            if at > 0 && used + columns > width {
                end = at;
                break;
            }
            used += columns;
        }
        let (line, rest) = remaining.split_at(end);
        remaining = rest;
        Some(line)
    })
}

/// How many character-wrapped rows `text` takes. Empty text takes none.
pub(crate) fn wrapped_rows(text: &str, width: usize) -> u16 {
    u16::try_from(wrapped_lines(text, width).count()).unwrap_or(u16::MAX)
}

/// The saved settings a popup row changes (the core shows their pending
/// and failed state beside the row).
fn row_settings(row: usize) -> &'static [Setting] {
    use NavPreference as N;
    match row {
        5 => &[Setting::Nav(N::AllowTeleports)],
        6 => &[Setting::Nav(N::AllowWilderness)],
        7 => &[Setting::Nav(N::AllowBankFetch)],
        8 => &[Setting::DangerRouting],
        9 => &[Setting::Nav(N::PauseScriptOnManualWalkAbort)],
        10 => &[Setting::MapBake],
        12 | 13 => &[Setting::QuesterPaths],
        _ => &[],
    }
}

/// What follows a row's text: nothing, or the separator and the state of the
/// row's setting (pending, or its retained failure).
type Feedback<'a> = [&'a str; 2];

const NO_FEEDBACK: Feedback<'static> = ["", ""];

/// The pieces that follow a row's text when its setting is pending or
/// failed, borrowed from the retained feedback.
fn feedback_parts(
    feedback: &[(Setting, SettingFeedback)],
    setting: Setting,
) -> Option<Feedback<'_>> {
    feedback
        .iter()
        .find(|(shown, _)| *shown == setting)
        .map(|(_, state)| match state {
            SettingFeedback::Pending => [" — ", "saving…"],
            SettingFeedback::Failed(text) => [" — ", text.as_str()],
        })
}

/// A notice's dismiss line and its setting's pending or failed state, as
/// the pieces one wrapped line is made of.
fn dismiss_parts<'a>(
    text: &'a str,
    feedback: &'a [(Setting, SettingFeedback)],
    setting: Setting,
) -> [&'a str; 3] {
    let [separator, state] = feedback_parts(feedback, setting).unwrap_or(NO_FEEDBACK);
    [text, separator, state]
}

/// Borrow character-wrapped rows of the text `parts` make up in order,
/// without copying any of it. Each row holds, per part, the slice of it
/// that row shows (empty when none).
pub(crate) fn wrapped_parts<const N: usize>(
    parts: [&str; N],
    width: usize,
) -> impl Iterator<Item = [&str; N]> {
    let mut rest = parts;
    let width = width.max(1);
    std::iter::from_fn(move || {
        if rest.iter().all(|part| part.is_empty()) {
            return None;
        }
        let mut row = [""; N];
        let mut used = 0;
        let mut full = false;
        for (shown, part) in row.iter_mut().zip(rest.iter_mut()) {
            if full {
                break;
            }
            let mut end = part.len();
            for (at, ch) in part.char_indices() {
                let columns = char_width(part, at, ch);
                if used > 0 && used + columns > width {
                    end = at;
                    full = true;
                    break;
                }
                used += columns;
            }
            let (head, tail) = part.split_at(end);
            *shown = head;
            *part = tail;
        }
        Some(row)
    })
}

/// How many wrapped rows the text `parts` make up take.
fn wrapped_rows_parts<const N: usize>(parts: [&str; N], width: usize) -> u16 {
    u16::try_from(wrapped_parts(parts, width).count()).unwrap_or(u16::MAX)
}

/// [`draw_wrapped`] for text made of pieces: each wrapped row draws its
/// slices one after the other. Allocates nothing.
fn draw_wrapped_parts<const N: usize>(
    buf: &mut Buffer,
    inner: Rect,
    mut y: u16,
    bottom: u16,
    parts: [&str; N],
    style: Style,
) -> u16 {
    let width = usize::from(inner.width).max(1);
    for row in wrapped_parts(parts, width) {
        if y < bottom {
            draw_slices(buf, inner.x, y, width, row, style);
        }
        y += 1;
    }
    y
}

/// Draw the slices of one wrapped row one after the other from `x`, at
/// most `width` columns.
fn draw_slices<const N: usize>(
    buf: &mut Buffer,
    mut x: u16,
    y: u16,
    width: usize,
    row: [&str; N],
    style: Style,
) {
    let mut left = width;
    for slice in row {
        if slice.is_empty() || left == 0 {
            continue;
        }
        let (next, _) = buf.set_stringn(x, y, slice, left, style);
        left = left.saturating_sub(usize::from(next.saturating_sub(x)));
        x = next;
    }
}

/// Draw one setting's block (its lead-in `header`, then its row text and
/// feedback `parts`) from the top of `inner` to `bottom`, with its first
/// `skip` wrapped lines scrolled out above. Allocates nothing.
fn draw_scrolled_block(
    buf: &mut Buffer,
    inner: Rect,
    bottom: u16,
    header: Option<(&str, Style)>,
    parts: [&str; 3],
    style: Style,
    skip: u16,
) {
    let width = usize::from(inner.width).max(1);
    let mut skip = usize::from(skip);
    let mut y = inner.y;
    if let Some((text, header_style)) = header {
        for line in wrapped_lines(text, width) {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            if y >= bottom {
                return;
            }
            buf.set_stringn(inner.x, y, line, width, header_style);
            y += 1;
        }
    }
    for row in wrapped_parts(parts, width).skip(skip) {
        if y >= bottom {
            return;
        }
        draw_slices(buf, inner.x, y, width, row, style);
        y += 1;
    }
}

/// Draw `text` into `buf` inside `inner` from row `y`, wrapped at the
/// popup's width, stopping at `bottom`. Returns the next free row. Slices
/// the borrowed text, so it allocates nothing.
pub(crate) fn draw_wrapped(
    buf: &mut Buffer,
    inner: Rect,
    mut y: u16,
    bottom: u16,
    text: &str,
    style: Style,
) -> u16 {
    let width = usize::from(inner.width).max(1);
    for line in wrapped_lines(text, width) {
        if y < bottom {
            buf.set_stringn(inner.x, y, line, width, style);
        }
        y += 1;
    }
    y
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use frontend_core::{CurePoison, GuardianPolicy, SettingsDraft};
    use frontend_core::{MapBakeChoice, MemoryNotice, NavPreference};

    use frontend_core::WalkGlobals;

    use super::{
        SettingsEdit, SettingsKey, SettingsPane, SettingsState, EVENT_POLICY_UNAVAILABLE,
        LAMP_SKILLS,
    };

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn render(pane: SettingsPane<'_>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(pane, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// The same render as [`render`], as one line per terminal row, so a
    /// wrapped row's text stays contiguous inside its terminal lines.
    fn render_lines(pane: SettingsPane<'_>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(pane, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn cure_poison_row_is_visible_at_common_sizes() {
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let settings = SettingsDraft::default();
            let mut nav = WalkGlobals::default();
            let mut bake = MapBakeChoice::Ask;
            let mut state = SettingsState {
                open: true,
                row: 15,
                ..Default::default()
            };
            let text = render_lines(
                SettingsPane::new(&settings, &mut nav, &mut bake, &mut state),
                w,
                h,
            );
            assert!(
                text.contains("Survival and supplies"),
                "{w}x{h}: the group heading paints:\n{text}"
            );
            assert!(
                text.contains(
                    "> cure poison: When needed: drink for health in combat or protected walks"
                ),
                "{w}x{h}: the selected cure poison row paints:\n{text}"
            );
        }
    }

    /// The popup returns the selected profile edit for its caller to stage.
    #[test]
    fn popup_returns_random_events_edits_without_mutating_the_draft() {
        let mut settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            ..Default::default()
        };
        assert!(settings.random_events(), "default random events on");
        let first = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(
            first,
            SettingsKey::Changed(SettingsEdit::RandomEvents(false))
        );
        assert!(
            settings.random_events(),
            "the pane returns the edit without mutating the draft"
        );

        *settings.random_events_mut() = false;
        let second = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(
            second,
            SettingsKey::Changed(SettingsEdit::RandomEvents(true))
        );
        assert!(
            !settings.random_events(),
            "the caller applies the first edit"
        );
    }

    #[test]
    fn popup_cycles_cure_poison_without_mutating_the_draft() {
        let mut settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 15,
            ..Default::default()
        };
        assert_eq!(
            settings.survival().cure_poison(),
            CurePoison::WhenNeeded,
            "default: cure when needed"
        );
        for want in [
            CurePoison::Always,
            CurePoison::Never,
            CurePoison::WhenNeeded,
        ] {
            let got = {
                let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
                pane.on_key(key(KeyCode::Enter))
            };
            assert_eq!(got, SettingsKey::Changed(SettingsEdit::CurePoison(want)));
            settings.survival_mut().set_cure_poison(want);
        }
    }

    #[test]
    fn popup_returns_supported_policy_reset_and_refuses_repeated_choice() {
        let mut settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 4,
            ..Default::default()
        };

        for policy in [GuardianPolicy::Fight, GuardianPolicy::Ignore] {
            settings.set_random_event_policy("river troll", policy);
            let changed = {
                let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
                pane.on_key(key(KeyCode::Enter))
            };
            assert_eq!(
                changed,
                SettingsKey::ChangedWithNotice(
                    SettingsEdit::RandomEventPolicy {
                        event: "river troll",
                        policy: GuardianPolicy::Flee,
                    },
                    EVENT_POLICY_UNAVAILABLE,
                )
            );
            assert_eq!(
                settings.random_event_policy("river troll"),
                policy,
                "the caller applies the supported reset"
            );
            settings.set_random_event_policy("river troll", GuardianPolicy::Flee);

            let refused = {
                let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
                pane.on_key(key(KeyCode::Enter))
            };
            assert_eq!(refused, SettingsKey::Refused(EVENT_POLICY_UNAVAILABLE));
            assert_eq!(
                settings.random_event_policy("river troll"),
                GuardianPolicy::Flee,
                "selecting unavailable Fight or Ignore is refused"
            );
        }
    }

    #[test]
    fn popup_returns_lamp_skill_and_auto_edits_without_mutating_the_draft() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 1,
            ..Default::default()
        };
        let skill = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(
            skill,
            SettingsKey::Changed(SettingsEdit::LampSkill(LAMP_SKILLS[2]))
        );
        assert_eq!(
            settings.lamp_skill(),
            LAMP_SKILLS[1],
            "the caller applies the selected skill"
        );

        let flipped = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            pane.state.row = 2;
            pane.on_key(key(KeyCode::Char(' ')))
        };
        assert_eq!(flipped, SettingsKey::Changed(SettingsEdit::LampAuto(false)));
        assert!(settings.lamp_auto(), "space returns the edit");
    }

    #[test]
    fn popup_toggles_bank_fetch_and_requests_single_key_persistence() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 7,
            ..Default::default()
        };
        let key = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(key, SettingsKey::WalkGlobal(NavPreference::AllowBankFetch));
        assert!(nav.allow_bank_fetch(), "bank fetch toggles on");
    }

    #[test]
    fn each_permission_row_requests_its_global_preference() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        for (row, preference) in [
            (5, NavPreference::AllowTeleports),
            (6, NavPreference::AllowWilderness),
            (7, NavPreference::AllowBankFetch),
            (8, NavPreference::SurvivableRouting),
        ] {
            let mut state = SettingsState {
                open: true,
                row,
                ..Default::default()
            };
            let outcome = {
                let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
                pane.on_key(key(KeyCode::Enter))
            };
            assert_eq!(outcome, SettingsKey::WalkGlobal(preference));
        }
        assert!(nav.allow_teleports());
        assert!(nav.allow_wilderness());
        assert!(nav.allow_bank_fetch());
        assert!(nav.allow_danger_zones());
        assert_eq!(nav.danger_level(), frontend_core::DangerLevel::Always);
    }

    #[test]
    fn script_scope_notice_is_dismissible_and_stays_hidden_after_ack() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let mut acknowledged = false;
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut acknowledged),
            100,
            24,
        );
        assert!(
            text.contains("Teleports and wilderness in Nav config"),
            "the unacknowledged notice is shown: {text:?}"
        );
        let outcome = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut acknowledged);
            pane.on_key(key(KeyCode::Char('d')))
        };
        assert_eq!(outcome, SettingsKey::ScriptScopeNoticeAck);
        acknowledged = true;
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut acknowledged),
            100,
            24,
        );
        assert!(!text.contains("Teleports and wilderness in Nav config"));
    }

    #[test]
    fn survivable_routing_notice_uses_an_independent_acknowledgement() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let mut old_acknowledged = true;
        let mut acknowledged = false;
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut old_acknowledged)
                .survivable_routing_notice_ack(&mut acknowledged),
            100,
            24,
        );
        assert!(
            text.contains("Danger routing now has three levels"),
            "the migration notice remains visible after the old notice was dismissed: {text:?}"
        );
        let outcome = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut old_acknowledged)
                .survivable_routing_notice_ack(&mut acknowledged);
            pane.on_key(key(KeyCode::Char('n')))
        };
        assert_eq!(outcome, SettingsKey::SurvivableRoutingNoticeAck);
        acknowledged = true;
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                .script_scope_notice_ack(&mut old_acknowledged)
                .survivable_routing_notice_ack(&mut acknowledged),
            100,
            24,
        );
        assert!(!text.contains("Danger routing now has three levels"));
    }

    #[test]
    fn held_survivable_setting_and_notice_are_truthful_in_the_popup() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 8,
            ..Default::default()
        };
        let mut acknowledged = false;
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                .survivable_routing_notice_ack(&mut acknowledged),
            120,
            40,
        );
        assert!(text.contains(frontend_core::walk_permissions::WHEN_SURVIVABLE_HELD_LABEL));
        assert!(text.contains("Effective level: Never"));
        assert!(text.contains("not available yet"));
        // The held wording itself must not promise behaviours the held
        // routing does not have. Scope the absence checks to that
        // wording: the popup also carries the cure poison row, whose
        // label legitimately mentions the eating line.
        let held = format!(
            "{} {}",
            frontend_core::walk_permissions::survivable_routing_tooltip(),
            frontend_core::walk_permissions::survivable_routing_notice()
        );
        assert!(!held.contains("274V17"));
        assert!(!held.contains("rebake"));
        assert!(!held.contains("turn back"));
        assert!(!held.contains("eating"));
        assert!(!held.contains("praying"));
    }

    #[test]
    fn survivable_routing_notice_renders_in_full_with_its_dismiss_row() {
        let notice = frontend_core::walk_permissions::survivable_routing_notice();
        let squeeze = |text: &str| -> String {
            text.chars()
                .filter(|ch| !ch.is_whitespace() && *ch != '│')
                .collect()
        };
        for (w, h) in [(120u16, 50u16), (80, 50), (60, 60)] {
            let settings = SettingsDraft::default();
            let mut nav = WalkGlobals::default();
            let mut bake = MapBakeChoice::Ask;
            let mut state = SettingsState {
                open: true,
                row: 0,
                ..Default::default()
            };
            let mut acknowledged = false;
            let text = render(
                SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                    .survivable_routing_notice_ack(&mut acknowledged),
                w,
                h,
            );
            // The pane wraps by character, so drop borders and whitespace
            // and compare the whole notice, its last words included.
            let painted = squeeze(&text);
            assert!(
                painted.contains(&format!("{}[n]dismissthisnotice", squeeze(notice))),
                "{w}x{h}: the full notice and its dismiss row paint: {text:?}"
            );
        }
    }

    #[test]
    fn up_and_down_move_the_row_and_esc_closes() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let rows = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            let down = pane.on_key(key(KeyCode::Down));
            let at = pane.state.row;
            let up = pane.on_key(key(KeyCode::Up));
            (down, at, up, pane.on_key(key(KeyCode::Esc)))
        };
        assert_eq!(rows.0, SettingsKey::Consumed);
        assert_eq!(rows.1, 1, "Down moves the row");
        assert_eq!(rows.2, SettingsKey::Consumed);
        assert_eq!(rows.3, SettingsKey::Consumed);
        assert!(!state.open, "Esc closes the popup");
    }

    #[test]
    fn popup_draws_the_rows_while_open_and_nothing_when_closed() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state),
            60,
            14,
        );
        assert!(text.contains("random events"), "row paints: {text:?}");

        state.row = 7;
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state),
            60,
            14,
        );
        assert!(text.contains("bank fetch"), "nav row paints: {text:?}");
        state.open = false;
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state),
            60,
            14,
        );
        assert!(
            !text.contains("random events"),
            "closed popup paints nothing: {text:?}"
        );
    }

    #[test]
    fn manual_movement_pause_row_is_reachable_persistent_and_visible_at_common_sizes() {
        for (width, height) in [(120, 40), (80, 24)] {
            let settings = SettingsDraft::default();
            let mut nav = WalkGlobals::default();
            let mut bake = MapBakeChoice::Ask;
            let mut pause = true;
            let mut state = SettingsState {
                open: true,
                row: 0,
                ..Default::default()
            };
            {
                let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                    .pause_script_on_manual_walk_abort(&mut pause);
                for _ in 0..9 {
                    assert_eq!(pane.on_key(key(KeyCode::Down)), SettingsKey::Consumed);
                }
                assert_eq!(pane.state.row, 9);
                assert_eq!(
                    pane.on_key(key(KeyCode::Enter)),
                    SettingsKey::PauseScriptOnManualWalkAbort
                );
            }
            assert!(!pause, "activation flips the saved global toggle off");
            let text = render(
                SettingsPane::new(&settings, &mut nav, &mut bake, &mut state)
                    .pause_script_on_manual_walk_abort(&mut pause),
                width,
                height,
            );
            assert!(
                text.contains("Pause script on manual movement: false"),
                "{width}x{height} must render the reachable row: {text:?}"
            );
        }
    }
    #[test]
    fn memory_row_returns_lowmem_edit_for_the_caller_to_persist() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 11,
            ..Default::default()
        };
        let outcome = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(outcome, SettingsKey::Changed(SettingsEdit::Lowmem(false)));
        assert!(
            settings.lowmem(),
            "the caller applies the edit before persistence"
        );
    }

    #[test]
    fn r_requests_relog_only_while_the_server_mode_differs_and_none_is_queued() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 0,
            ..Default::default()
        };
        let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
        assert_eq!(
            pane.on_key(key(KeyCode::Char('r'))),
            SettingsKey::Consumed,
            "no divergence means no relog"
        );
        pane.memory = Some(MemoryNotice {
            login_lowmem: false,
            desired_lowmem: true,
            relog_pending: false,
            connected: true,
            login_applies_memory: true,
        });
        assert_eq!(
            pane.on_key(key(KeyCode::Char('r'))),
            SettingsKey::MemoryRelog
        );
        pane.memory.as_mut().unwrap().relog_pending = true;
        assert_eq!(
            pane.on_key(key(KeyCode::Char('r'))),
            SettingsKey::Consumed,
            "a queued relog cannot be restarted"
        );
        assert!(settings.lowmem(), "r never flips the setting itself");
    }

    #[test]
    fn popup_shows_the_login_mode_and_relog_hint_while_it_differs() {
        let mut settings = SettingsDraft::default();
        *settings.lowmem_mut() = false;
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 11,
            ..Default::default()
        };
        let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
        pane.memory = Some(MemoryNotice {
            login_lowmem: true,
            desired_lowmem: false,
            relog_pending: false,
            connected: true,
            login_applies_memory: true,
        });
        let text = render(pane, 60, 14);
        assert!(
            text.contains("memory: lowmem (next login highmem)"),
            "{text:?}"
        );
        assert!(text.contains("Relog now"), "{text:?}");
    }

    #[test]
    fn map_bake_row_flips_the_shared_choice_and_asks_the_binary_to_persist_it() {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut state = SettingsState {
            open: true,
            row: 9,
            ..Default::default()
        };
        let (moved, flipped) = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            let moved = pane.on_key(key(KeyCode::Down));
            (moved, pane.on_key(key(KeyCode::Enter)))
        };
        assert_eq!(moved, SettingsKey::Consumed);
        assert_eq!(state.row, 10, "the map-bake row follows the pause toggle");
        assert_eq!(flipped, SettingsKey::MapBake);
        assert_eq!(bake, MapBakeChoice::Always);
        assert!(!nav.allow_bank_fetch(), "the bank row is untouched");
        let text = render(
            SettingsPane::new(&settings, &mut nav, &mut bake, &mut state),
            60,
            14,
        );
        assert!(text.contains("map bake: always"), "row paints: {text:?}");
        let back = {
            let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, &mut state);
            pane.on_key(key(KeyCode::Char(' ')))
        };
        assert_eq!(back, SettingsKey::MapBake);
        assert_eq!(bake, MapBakeChoice::Ask);
    }

    /// One drawn frame of the popup with the scope and routing notices up
    /// and `feedback` beside its settings, one string per terminal line.
    fn draw_with_notices(
        state: &mut SettingsState,
        feedback: &[(frontend_core::Setting, crate::app::SettingFeedback)],
        w: u16,
        h: u16,
    ) -> Vec<String> {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        let mut scope_ack = false;
        let mut survivable_ack = false;
        let mut pane = SettingsPane::new(&settings, &mut nav, &mut bake, state)
            .script_scope_notice_ack(&mut scope_ack)
            .survivable_routing_notice_ack(&mut survivable_ack);
        pane.feedback = feedback;
        render_lines(pane, w, h)
            .lines()
            .map(|line| line.trim_matches(|c| c == '│' || c == ' ').to_string())
            .collect()
    }

    fn press(state: &mut SettingsState, code: KeyCode) -> SettingsKey {
        let settings = SettingsDraft::default();
        let mut nav = WalkGlobals::default();
        let mut bake = MapBakeChoice::Ask;
        SettingsPane::new(&settings, &mut nav, &mut bake, state).on_key(key(code))
    }

    /// The selected setting's whole failure can be read at 80x24 and
    /// 120x40. A permission failure with a long path and its remedy is
    /// taller than the compact viewport: the viewport shows a window of the
    /// setting and a line that says more is hidden, and Down scrolls through
    /// every line of it to the end of the remedy before it moves to the next
    /// row. Up scrolls back before it moves the row.
    #[test]
    fn settings_feedback_a_long_failure_at_the_selected_setting_is_readable_to_its_end() {
        use crate::app::SettingFeedback;
        let folders: String = (0..42).map(|i| format!("/directory-{i:02}")).collect();
        let file = format!("/home/alice{folders}/panel-ui.json");
        assert!(file.len() < 1024);
        let failure = format!(
            "Not saved: {file}: Permission denied (os error 13). Make the file writable: \
             chmod 600 {file} END-OF-DIAGNOSTIC"
        );
        let feedback = [(
            frontend_core::Setting::Nav(NavPreference::AllowTeleports),
            SettingFeedback::Failed(failure.clone()),
        )];
        let row_text = format!("> {}: false", frontend_core::GLOBAL_PERMISSION_LABELS[0].1);
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let columns = usize::from(w.min(100) - 2);
            let expected: Vec<String> =
                super::wrapped_parts([row_text.as_str(), " — ", failure.as_str()], columns)
                    .map(|[a, b, c]| format!("{a}{b}{c}").trim_end().to_string())
                    .collect();
            let mut state = SettingsState {
                open: true,
                row: 5,
                ..Default::default()
            };
            let mut seen: Vec<String> = Vec::new();
            let mut screens = 0;
            loop {
                let screen = draw_with_notices(&mut state, &feedback, w, h);
                if screens == 0 && w == 80 {
                    assert!(
                        screen.iter().any(|line| line == super::MORE_BELOW),
                        "{w}x{h}: the hidden part is announced:\n{}",
                        screen.join("\n")
                    );
                }
                let at_end = screen.iter().any(|line| line.contains("END-OF-DIAGNOSTIC"));
                seen.extend(screen);
                screens += 1;
                if at_end {
                    break;
                }
                assert!(screens < 80, "{w}x{h}: the end of the failure never showed");
                assert_eq!(press(&mut state, KeyCode::Down), SettingsKey::Consumed);
                assert_eq!(state.row, 5, "{w}x{h}: Down scrolls the setting first");
            }
            for line in &expected {
                assert!(
                    seen.iter().any(|shown| shown == line),
                    "{w}x{h}: the line {line:?} of the setting never showed"
                );
            }
            if w == 80 {
                assert!(screens > 1, "80x24 needs the scroll");
                let last = draw_with_notices(&mut state, &feedback, w, h);
                assert!(last.iter().any(|line| line == super::MORE_ABOVE));
                assert_eq!(press(&mut state, KeyCode::Up), SettingsKey::Consumed);
                assert_eq!(state.row, 5, "Up scrolls back first");
                let back = draw_with_notices(&mut state, &feedback, w, h);
                assert!(back.iter().any(|line| line == super::MORE_ABOVE_AND_BELOW));
                assert_eq!(press(&mut state, KeyCode::Down), SettingsKey::Consumed);
            }
            press(&mut state, KeyCode::Down);
            assert_eq!(state.row, 6, "{w}x{h}: past the end, Down moves the row");
        }
    }

    /// Notices under the rows that need more lines than the popup leaves
    /// them are never cut silently: the first line of their area says which
    /// notices are hidden and the key that shows them. When the popup leaves
    /// no line for any notice, the bottom border says so. When every notice
    /// fits, nothing says so.
    #[test]
    fn settings_feedback_notices_that_do_not_fit_say_so() {
        use crate::app::SettingFeedback;
        use frontend_core::Setting;
        let long = format!("Not saved: {}", "the disk is full; free space ".repeat(12));
        let failed = [
            (
                Setting::Nav(NavPreference::ScriptScopeNoticeAck),
                SettingFeedback::Failed(long.clone()),
            ),
            (
                Setting::Nav(NavPreference::SurvivableRoutingNoticeAck),
                SettingFeedback::Failed(long),
            ),
        ];
        let has = |screen: &[String], text: &str| screen.iter().any(|line| line.contains(text));
        let mut state = SettingsState {
            open: true,
            ..Default::default()
        };
        let small = draw_with_notices(&mut state, &failed, 80, 18);
        assert!(
            small
                .iter()
                .any(|line| line == "↑ more notices above (PgUp)"),
            "80x18:\n{}",
            small.join("\n")
        );
        let tiny = draw_with_notices(&mut state, &failed, 80, 5);
        assert!(
            has(&tiny, super::NOTICES_CUT.trim()),
            "80x5:\n{}",
            tiny.join("\n")
        );
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let fits = draw_with_notices(&mut state, &[], w, h);
            assert!(
                !has(&fits, "more notices") && !has(&fits, super::NOTICES_CUT.trim()),
                "{w}x{h}:\n{}",
                fits.join("\n")
            );
        }
    }

    /// Every line of a failed notice acknowledgement can be read at 80x24
    /// and 120x40, to the end of its remedy. Both acknowledgements fail with
    /// a long path: the notices need more lines than the popup has, so
    /// their area shows the last ones under a sign, and PageUp and PageDown
    /// scroll through all of them without moving the row.
    #[test]
    fn settings_feedback_notice_failures_are_readable_to_their_end_at_supported_sizes() {
        use crate::app::SettingFeedback;
        use frontend_core::Setting;
        let folders: String = (0..42).map(|i| format!("/directory-{i:02}")).collect();
        let file = format!("/home/alice{folders}/panel-ui.json");
        assert!(file.len() < 1024);
        let failure = |end: &str| {
            format!(
                "Not saved: {file}: Permission denied (os error 13). Make the file writable: \
                 chmod 600 {file} {end}"
            )
        };
        let scope = failure("END-SCOPE");
        let routing = failure("END-ROUTING");
        let feedback = [
            (
                Setting::Nav(NavPreference::ScriptScopeNoticeAck),
                SettingFeedback::Failed(scope.clone()),
            ),
            (
                Setting::Nav(NavPreference::SurvivableRoutingNoticeAck),
                SettingFeedback::Failed(routing.clone()),
            ),
        ];
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let columns = usize::from(w.min(100) - 2);
            let expected: Vec<String> = [
                ("[d] dismiss this notice", scope.as_str()),
                ("[n] dismiss this notice", routing.as_str()),
            ]
            .into_iter()
            .flat_map(|(dismiss, failure)| {
                super::wrapped_parts([dismiss, " — ", failure], columns)
                    .map(|[a, b, c]| format!("{a}{b}{c}").trim().to_string())
                    .collect::<Vec<_>>()
            })
            .collect();
            let mut state = SettingsState {
                open: true,
                ..Default::default()
            };
            let first = draw_with_notices(&mut state, &feedback, w, h);
            let mut seen = first.clone();
            let mut top = Vec::new();
            let mut keys = Vec::new();
            for _ in 0..40 {
                keys.push(press(&mut state, KeyCode::PageUp));
                top = draw_with_notices(&mut state, &feedback, w, h);
                seen.extend(top.iter().cloned());
            }
            for _ in 0..40 {
                keys.push(press(&mut state, KeyCode::PageDown));
                seen.extend(draw_with_notices(&mut state, &feedback, w, h));
            }
            for line in &expected {
                assert!(
                    seen.iter().any(|shown| shown == line),
                    "{w}x{h}: the notice line {line:?} never showed"
                );
            }
            assert!(keys.iter().all(|key| *key == SettingsKey::Consumed));
            assert_eq!(state.row, 0, "{w}x{h}: PageUp and PageDown keep the row");
            if w == 80 {
                assert!(
                    first
                        .iter()
                        .any(|line| line == "↑ more notices above (PgUp)"),
                    "{w}x{h}: the hidden notices are announced:\n{}",
                    first.join("\n")
                );
                assert!(
                    top.iter().any(|line| line == "↓ more notices below (PgDn)"),
                    "{w}x{h}: at the first notice the rest is below:\n{}",
                    top.join("\n")
                );
            }
        }
    }

    /// However few lines the notices leave the rows, a selected setting
    /// taller than them says that more of it is hidden, on every screen
    /// that does not show its end, and Down reaches its end.
    #[test]
    fn settings_feedback_a_cut_selected_setting_always_says_more_is_hidden() {
        use crate::app::SettingFeedback;
        use frontend_core::Setting;
        let long = |end: &str| {
            format!(
                "Not saved: {}{end}",
                "the store is busy; try again later ".repeat(8)
            )
        };
        let feedback = [
            (
                Setting::Nav(NavPreference::AllowTeleports),
                SettingFeedback::Failed(long("END-OF-TELEPORTS")),
            ),
            (
                Setting::Nav(NavPreference::ScriptScopeNoticeAck),
                SettingFeedback::Failed(long("END-SCOPE")),
            ),
            (
                Setting::Nav(NavPreference::SurvivableRoutingNoticeAck),
                SettingFeedback::Failed(long("END-ROUTING")),
            ),
        ];
        // Grok's case first (80x18: the notices leave the rows one line on
        // the old popup), then the supported sizes, then every height.
        let sizes = [(80u16, 18u16), (80, 24), (120, 40)]
            .into_iter()
            .chain((6u16..=40).map(|h| (80u16, h)));
        for (w, h) in sizes {
            let mut state = SettingsState {
                open: true,
                row: 5,
                ..Default::default()
            };
            let mut presses = 0;
            loop {
                let screen = draw_with_notices(&mut state, &feedback, w, h);
                if screen.iter().any(|line| line.contains("END-OF-TELEPORTS")) {
                    break;
                }
                assert!(
                    screen.iter().any(|line| {
                        line == super::MORE_BELOW || line == super::MORE_ABOVE_AND_BELOW
                    }),
                    "{w}x{h}: part of the selected setting is hidden without a sign \
                     (after {presses} Down):\n{}",
                    screen.join("\n")
                );
                assert!(
                    presses < 200,
                    "{w}x{h}: the end of the setting never showed"
                );
                assert_eq!(press(&mut state, KeyCode::Down), SettingsKey::Consumed);
                assert_eq!(state.row, 5, "{w}x{h}: Down scrolls the setting first");
                presses += 1;
            }
        }
    }
}
