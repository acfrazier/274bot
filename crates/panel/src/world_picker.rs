//! The world picker: the one popup every Switch world entry of the panel
//! opens. A click on the world marker of a rail tile or a Grid cell, on the
//! World / login cell of the Fleet window, or the Fleet window's **Switch
//! world…** entry in the Marks menu, asks [`Session::open_world_picker`];
//! [`popup`] draws it once per frame from `ui_frame`, so there is never a
//! second picker.
//!
//! It lists the bound server's public worlds from the core's cached list
//! ([`frontend_core::OperatorSession::switch_world_choices`]: read when the
//! play started, nothing is fetched or read from disk) and ticks the bot's
//! own world. The labels are built when the picker opens, never per frame.
//! Picking the world the bot is on does nothing. Picking another world
//! plans the switch through the fleet plan flow
//! ([`Session::plan_world_switch`]); the plan's prompt then shows in the
//! Fleet window's plan row with **Confirm** and **Cancel**, and nothing runs
//! before Confirm.

use dear_imgui_rs::Ui;

use crate::session::Session;

/// The popup's ImGui id.
const POPUP: &str = "##world-picker";

/// One public world as the popup lists it.
#[derive(Debug)]
struct Entry {
    world: u16,
    /// `World 301`, built when the picker opened.
    label: String,
}

/// An open picker: what it moves and the rows it lists.
#[derive(Debug)]
struct Open {
    /// The bot it moves; `None`: the marked bots, or the loaded shown bots
    /// when none is marked.
    target: Option<String>,
    /// The target's world now: ticked, and picking it does nothing.
    current: Option<u16>,
    /// The line above the rows, naming what moves.
    heading: String,
    entries: Vec<Entry>,
    /// The popup has been opened in Dear ImGui.
    shown: bool,
}

/// The picker's state: closed, or open for one bot or the marks.
#[derive(Debug, Default)]
pub(crate) struct WorldPicker {
    open: Option<Open>,
}

impl WorldPicker {
    /// Whether the picker is open.
    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The bot the open picker moves (`None` while closed, or for the marks).
    #[cfg(test)]
    pub(crate) fn target(&self) -> Option<&str> {
        self.open.as_ref()?.target.as_deref()
    }

    /// The buffers of the open picker's labels, to see that a frame kept
    /// them (nothing is rebuilt per frame).
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> Vec<*const u8> {
        self.open
            .iter()
            .flat_map(|open| {
                open.entries
                    .iter()
                    .map(|entry| entry.label.as_ptr())
                    .chain([open.heading.as_ptr()])
            })
            .collect()
    }
}

/// One picker row as a test sees it.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct EntryRecord {
    pub world: u16,
    pub label: String,
    pub ticked: bool,
    pub rect: [f32; 4],
}

#[cfg(test)]
std::thread_local! {
    /// The rows the last [`popup`] call drew.
    static ENTRIES: std::cell::RefCell<Vec<EntryRecord>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The rows drawn by the last [`popup`] call, in order.
#[cfg(test)]
pub(crate) fn take_entries() -> Vec<EntryRecord> {
    ENTRIES.with_borrow_mut(std::mem::take)
}

impl Session {
    /// Whether the bound server has public worlds to switch to. A local
    /// server has one world: no marker, cell or entry offers a switch.
    pub(crate) fn can_switch_world(&self) -> bool {
        self.core
            .switch_world_choices()
            .is_some_and(|worlds| worlds.len() > 0)
    }

    /// Open the world picker for the profile `target`, or for the marked
    /// bots when it is `None` (the loaded shown bots when none is marked).
    /// Returns whether it opened: a server with one world has nothing to
    /// pick. A picker that is already open is replaced.
    pub(crate) fn open_world_picker(&mut self, target: Option<&str>) -> bool {
        use std::fmt::Write as _;
        let Some(worlds) = self
            .core
            .switch_world_choices()
            .filter(|worlds| worlds.len() > 0)
        else {
            return false;
        };
        let current = target
            .and_then(|name| {
                self.fleet_model
                    .rows()
                    .iter()
                    .find(|row| row.name() == name)
            })
            .and_then(|row| row.world());
        let mut heading = String::new();
        match target {
            Some(name) => {
                let _ = write!(heading, "Switch world of {name}");
            }
            None => {
                let counts = self.fleet_model.counts();
                let _ = if counts.marked() > 0 {
                    let plural = if counts.marked() == 1 { "" } else { "s" };
                    write!(
                        heading,
                        "Switch world of {} marked bot{plural}",
                        counts.marked()
                    )
                } else {
                    write!(
                        heading,
                        "Switch world of all {} shown",
                        counts.shown_loaded()
                    )
                };
            }
        }
        let entries = worlds
            .map(|world| {
                let mut label = String::new();
                let _ = write!(label, "World {world}");
                Entry { world, label }
            })
            .collect();
        self.world_picker.open = Some(Open {
            target: target.map(str::to_owned),
            current,
            heading,
            entries,
            shown: false,
        });
        true
    }
}

/// Draw the picker while it is open. Call it once per frame from the frame's
/// root (outside every window): the first call after the click that asked
/// for the picker opens the popup at the pointer, and a pick of a world
/// other than the bot's own plans the switch. A click away or Esc closes it
/// and changes nothing.
pub(crate) fn popup(ui: &Ui, session: &mut Session) {
    #[cfg(test)]
    ENTRIES.with_borrow_mut(Vec::clear);
    let Some(open) = session.world_picker.open.as_mut() else {
        return;
    };
    if !open.shown {
        ui.open_popup(POPUP);
        open.shown = true;
    }
    let mut picked = None;
    {
        let Some(_popup) = ui.begin_popup(POPUP) else {
            // Closed by Esc or a click away: nothing was picked.
            session.world_picker.open = None;
            return;
        };
        ui.text_disabled(&open.heading);
        for entry in &open.entries {
            let ticked = open.current == Some(entry.world);
            if ui.menu_item_enabled_selected_no_shortcut(&entry.label, ticked, true) {
                picked = Some(entry.world);
            }
            #[cfg(test)]
            {
                let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
                ENTRIES.with_borrow_mut(|entries| {
                    entries.push(EntryRecord {
                        world: entry.world,
                        label: entry.label.clone(),
                        ticked,
                        rect: [min[0], min[1], max[0], max[1]],
                    });
                });
            }
        }
    }
    let Some(world) = picked else {
        return;
    };
    // A pick closes the popup (Dear ImGui closes it on the menu item).
    if let Some(open) = session.world_picker.open.take() {
        if open.current != Some(world) {
            session.plan_world_switch(open.target.as_deref(), world);
        }
    }
}
