//! Armed selected-actor records. Cached actor families retain their existing behavior.
use api::snapshot::{
    actor_observation, ActorChatStyle, ActorObservationView, ActorSelector, WorldTile,
};
use client::client::Client;
use serde::Serialize;

const MAX_RECORDS: usize = 1024;

/// Identity and clock of one native sample. A slot is not a persistent spawn ID.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ActorStamp {
    pub actor: ActorSelector,
    pub index: usize,
    pub session: u64,
    pub source_cycle: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActorRecord {
    ActorSample {
        #[serde(flatten)]
        stamp: ActorStamp,
        name: Option<String>,
        npc_type: Option<usize>,
        tile: WorldTile,
        network: WorldTile,
        animation: i32,
        animation_frame: i32,
        pose_animation: i32,
    },
    ActorStyle {
        #[serde(flatten)]
        stamp: ActorStamp,
        style: ActorChatStyle,
    },
    Overhead {
        #[serde(flatten)]
        stamp: ActorStamp,
        transition: &'static str,
        /// Only appearance/replacement owns text. Countdown records reference it.
        text: Option<String>,
        message_record: Option<usize>,
        remaining_cycles: i32,
    },
    Unavailable {
        actor: ActorSelector,
        session: u64,
        source_cycle: i32,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ActorEvidence {
    pub records: Vec<ActorRecord>,
    pub overflowed: bool,
}

#[derive(Clone, Copy)]
struct Current {
    stamp: ActorStamp,
    scalar_record: usize,
    message_record: Option<usize>,
    style: ActorChatStyle,
    remaining: i32,
    decreased: bool,
    ended: bool,
}

/// Absent until an ObserveActor step runs. Unchanged samples retain no new storage.
#[derive(Default)]
pub struct ActorObserver {
    evidence: ActorEvidence,
    selected: Option<ActorSelector>,
    current: Option<Current>,
    unavailable: bool,
}

impl ActorObserver {
    fn push(&mut self, record: ActorRecord) -> Result<usize, &'static str> {
        if self.evidence.records.len() == MAX_RECORDS {
            self.evidence.overflowed = true;
            self.current = None;
            return Err("actor-observation-overflow");
        }
        let index = self.evidence.records.len();
        self.evidence.records.push(record);
        Ok(index)
    }

    fn text(&self, record: Option<usize>) -> Option<&str> {
        match self.evidence.records.get(record?)? {
            ActorRecord::Overhead { text, .. } => text.as_deref(),
            _ => None,
        }
    }

    fn same_actor(&self, previous: Current, sample: ActorObservationView<'_>) -> bool {
        previous.stamp.actor == sample.actor
            && previous.stamp.index == sample.index
            && previous.stamp.session == sample.session
            && matches!(&self.evidence.records[previous.scalar_record],
                ActorRecord::ActorSample { name, npc_type, .. }
                if name.as_deref() == sample.name && *npc_type == sample.npc_type)
    }

    fn same_pose(&self, previous: Current, sample: ActorObservationView<'_>) -> bool {
        matches!(&self.evidence.records[previous.scalar_record],
            ActorRecord::ActorSample { tile, network, animation, animation_frame, pose_animation, .. }
            if *tile == sample.tile && *network == sample.network
                && *animation == sample.animation && *animation_frame == sample.animation_frame
                && *pose_animation == sample.pose_animation)
    }

    /// The optional owner check precedes the selected-client read and all recording.
    pub fn observe(
        owner: Option<&mut Self>,
        client: &Client,
        selected: Option<ActorSelector>,
    ) -> Result<(), &'static str> {
        let Some(owner) = owner else { return Ok(()) };
        if owner.selected != selected {
            owner.selected = selected;
            owner.current = None;
            owner.unavailable = false;
        }
        let Some(actor) = selected else { return Ok(()) };
        if owner.evidence.overflowed {
            return Err("actor-observation-overflow");
        }
        let Some(sample) = actor_observation(client, Some(actor)) else {
            owner.current = None;
            if !owner.unavailable {
                owner.push(ActorRecord::Unavailable {
                    actor,
                    session: client.gens.session,
                    source_cycle: client.loop_cycle,
                })?;
                owner.unavailable = true;
            }
            return Ok(());
        };
        owner.unavailable = false;
        owner.sample(sample)
    }

    fn sample(&mut self, sample: ActorObservationView<'_>) -> Result<(), &'static str> {
        let stamp = ActorStamp {
            actor: sample.actor,
            index: sample.index,
            session: sample.session,
            source_cycle: sample.source_cycle,
        };
        let previous = self.current.filter(|&old| self.same_actor(old, sample));
        let scalar_record = if let Some(old) = previous.filter(|&old| self.same_pose(old, sample)) {
            old.scalar_record
        } else {
            self.push(ActorRecord::ActorSample {
                stamp,
                name: sample.name.map(str::to_owned),
                npc_type: sample.npc_type,
                tile: sample.tile,
                network: sample.network,
                animation: sample.animation,
                animation_frame: sample.animation_frame,
                pose_animation: sample.pose_animation,
            })?
        };
        if previous.is_none_or(|old| old.style != sample.chat_style) {
            self.push(ActorRecord::ActorStyle {
                stamp,
                style: sample.chat_style,
            })?;
        }
        let active = sample.overhead_text.is_some() && sample.remaining_cycles > 0;
        let same_message = previous.is_some_and(|old| {
            !old.ended
                && self.text(old.message_record) == sample.overhead_text
                && sample.remaining_cycles <= old.remaining
        });
        let mut current = Current {
            stamp,
            scalar_record,
            message_record: None,
            style: sample.chat_style,
            remaining: sample.remaining_cycles,
            decreased: false,
            ended: false,
        };
        if active {
            if same_message {
                let old = previous.expect("same message requires a preceding sample");
                current.message_record = old.message_record;
                current.decreased = old.decreased
                    || (sample.remaining_cycles < old.remaining
                        && sample.source_cycle.wrapping_sub(old.stamp.source_cycle) > 0);
                if sample.remaining_cycles != old.remaining {
                    self.push(ActorRecord::Overhead {
                        stamp,
                        transition: "countdown",
                        text: None,
                        message_record: old.message_record,
                        remaining_cycles: sample.remaining_cycles,
                    })?;
                }
            } else {
                current.message_record = Some(self.push(ActorRecord::Overhead {
                    stamp,
                    transition: if previous.is_some_and(|old| old.message_record.is_some()) {
                        "replaced"
                    } else {
                        "appeared"
                    },
                    text: sample.overhead_text.map(str::to_owned),
                    message_record: None,
                    remaining_cycles: sample.remaining_cycles,
                })?);
            }
        } else if let Some(old) = previous.filter(|old| old.message_record.is_some()) {
            current.message_record = old.message_record;
            current.decreased = old.decreased;
            current.ended = old.ended
                || (old.decreased
                    && sample.remaining_cycles == 0
                    && sample.overhead_text.is_none()
                    && sample.source_cycle.wrapping_sub(old.stamp.source_cycle) > 0);
            if !old.ended {
                self.push(ActorRecord::Overhead {
                    stamp,
                    transition: if current.ended { "expired" } else { "cleared" },
                    text: None,
                    message_record: old.message_record,
                    remaining_cycles: sample.remaining_cycles,
                })?;
                if !current.ended {
                    current.message_record = None;
                }
            }
        }
        self.current = Some(current);
        Ok(())
    }

    pub(crate) fn holds(&self, proof: crate::Proof) -> bool {
        if self.evidence.overflowed {
            return false;
        }
        let Some(current) = self.current else {
            return false;
        };
        match proof {
            crate::Proof::ActorChatStyle {
                actor,
                colour,
                effect,
            } => {
                current.stamp.actor == actor
                    && current.remaining > 0
                    && !current.ended
                    && current.message_record.is_some()
                    && current.style == ActorChatStyle { colour, effect }
            }
            crate::Proof::OverheadText { actor, text } => {
                current.stamp.actor == actor
                    && current.remaining > 0
                    && !current.ended
                    && self.text(current.message_record) == Some(text)
            }
            crate::Proof::OverheadTextLifetime { actor, text } => {
                current.stamp.actor == actor
                    && current.ended
                    && self.text(current.message_record) == Some(text)
            }
            _ => false,
        }
    }

    pub fn into_evidence(self) -> ActorEvidence {
        self.evidence
    }
}
