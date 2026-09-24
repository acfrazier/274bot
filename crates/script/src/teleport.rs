//! Rust-owned spellbook teleport, a [`crate::machine`] family. Selected-
//! cache metadata names the non-target button; compact snapshot facts
//! observe Magic XP and tile change. JavaScript starts it with the caller
//! name and awaits the arrival verdict; Rust presses the if-button. A
//! queued click is not arrival.

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, Scene};
use crate::shim::InteractReq;
use serde::Deserialize;

/// Steps after the click before an unobserved arrival is a timeout.
pub const SETTLE_POLLS: u32 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

/// The posted facts this module decides from, read from the isolate scene.
struct NativeObservation {
    ingame: bool,
    here: Option<Tile>,
    magic_xp: Option<i32>,
    magic_level: Option<i32>,
}

impl NativeObservation {
    /// A logout forgets the session: only pages posted since login count.
    fn from_scene(scene: &Scene) -> Self {
        let session = scene.since_login();
        let magic = session.stats().and_then(|skills| skills.magic);
        Self {
            ingame: session.ingame().unwrap_or(false),
            here: session.here().map(|tile| Tile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            }),
            magic_xp: magic.map(|row| row.xp),
            magic_level: magic.map(|row| row.effective),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct TeleportArgs {
    name: String,
}

/// One cast: the click went out at begin; each step polls for arrival.
pub(crate) struct Teleport {
    polls_left: u32,
    start_here: Option<Tile>,
    start_xp: Option<i32>,
}

impl Family for Teleport {
    const NAME: &'static str = "teleport";
    /// A new cast replaces the one in flight.
    const EXCLUSIVE: bool = true;
    type Args = TeleportArgs;
    /// Arrival observed (Magic XP gained and tile changed).
    type Output = bool;

    fn begin(args: TeleportArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let Some(selected) = crate::supply_v2::selected_data() else {
            return Begin::Refuse("missing selected teleports".into());
        };
        if selected.teleports().is_empty() {
            return Begin::Refuse("missing selected teleports".into());
        }
        let Some(spell) = selected.teleport(&args.name) else {
            return Begin::Done(false);
        };
        if !spell.available() {
            return Begin::Refuse("absent control".into());
        }
        let obs = observed::with(NativeObservation::from_scene);
        if obs.magic_level.is_some_and(|level| level < spell.level) {
            return Begin::Done(false);
        }
        cx.emit(InteractReq::IfButton {
            component_id: spell.component_id,
        });
        Begin::Run(Self {
            polls_left: SETTLE_POLLS,
            start_here: obs.here,
            start_xp: obs.magic_xp,
        })
    }

    fn step(&mut self, _cx: &mut Cx<'_>) -> Step<bool> {
        let obs = observed::with(NativeObservation::from_scene);
        if !obs.ingame {
            return Step::Done(false);
        }
        if arrived(self.start_here, self.start_xp, obs.here, obs.magic_xp) {
            return Step::Done(true);
        }
        if self.polls_left == 0 {
            return Step::Done(false);
        }
        self.polls_left -= 1;
        Step::Wait
    }
}

fn arrived(
    start_here: Option<Tile>,
    start_xp: Option<i32>,
    here: Option<Tile>,
    xp: Option<i32>,
) -> bool {
    let (Some(before), Some(after), Some(xp_before), Some(xp_after)) =
        (start_here, here, start_xp, xp)
    else {
        return false;
    };
    after != before && xp_after > xp_before
}

#[cfg(test)]
mod tests {
    use crate::machine::{self, Outcome, Started};
    use client::io::ClientRevision;
    use serde_json::json;

    fn data(rev: ClientRevision) -> std::sync::Arc<api::game_data::SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    #[test]
    fn both_selected_caches_post_the_audited_teleports() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(rev);
            let varrock = data.teleport("Varrock").expect("Varrock");
            assert_eq!(varrock.component_id, 1164);
            assert_eq!(varrock.level, 25);
            assert_eq!(varrock.experience, 350);
            assert_eq!(varrock.x, 3213);
            assert_eq!(varrock.z, 3424);
            assert_eq!(
                varrock
                    .runes
                    .iter()
                    .map(|rune| (rune.name.as_str(), rune.count))
                    .collect::<Vec<_>>(),
                vec![("Fire rune", 1), ("Air rune", 3), ("Law rune", 1)]
            );
            let falador = data
                .teleport("Cast @gre@Falador teleport")
                .expect("Falador live label");
            assert_eq!(falador.component_id, 1170);
            assert_eq!(falador.level, 37);
            assert!(data.teleport("Wind Strike").is_none());
            assert!(data.teleport("High level alchemy").is_none());
            assert_eq!(data.teleports().len(), 7);
            assert_eq!(data.teleports()[6].name, "Trollheim");
        }
    }

    #[test]
    fn missing_controls_do_not_invent_a_button() {
        crate::supply_v2::configure(None);
        assert_eq!(
            machine::start("teleport", json!({ "name": "Varrock" }), Vec::new(), 0),
            Started::Refused("missing selected teleports".into())
        );
        crate::supply_v2::configure(Some(data(ClientRevision::R274)));
        assert_eq!(
            machine::start("teleport", json!({ "name": "Nowhere" }), Vec::new(), 0),
            Started::Settled(Outcome::Done(json!(false)))
        );
        assert!(machine::merge_ops(Vec::new()).is_empty());
    }
}
