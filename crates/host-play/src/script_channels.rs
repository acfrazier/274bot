//! One-Play, one-world BroadcastChannel broker for the frozen JiveKQ card.
//!
//! The browser API is only a shim endpoint. Membership, roster admission,
//! world and generation fencing, sequencing and lifecycle revocation live
//! here. A membership belongs to the script run, not to the connection: a
//! session boundary the isolate survives suspends it, and the relogged
//! session re-admits it only under the same run generation and world.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use script::shim::InteractReq;

const PREFIX: &str = "rs2b0t:kq:v1:";
const MAX_GROUPS: usize = 16;
const LEFT: &str = "party member left or changed world";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BrokerWorld {
    Local,
    Public(u16),
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Delivery {
    pub account: String,
    pub generation: u64,
    pub event: InteractReq,
}

/// The Play-wide broker. Slot threads reach it through their own
/// [`SlotChannels`].
#[derive(Clone, Default)]
pub(crate) struct ChannelBroker(Arc<Mutex<Broker>>);

/// One slot thread's handle. Its keys are normalized once, and whether the
/// broker holds anything for the account is one atomic load, so a slot with
/// no channel never takes the Play-wide lock. The flag is the handle's own:
/// only this account's calls change what the broker holds for it, and each
/// of them re-reads that under the lock, so the broker keeps nothing per
/// profile once the account leaves.
#[derive(Clone)]
pub(crate) struct SlotChannels {
    broker: ChannelBroker,
    slot: String,
    account: String,
    tracked: Arc<AtomicBool>,
}

#[derive(Default)]
struct Broker {
    groups: HashMap<String, Group>,
    /// Accounts with a membership (live or suspended), or a live run the
    /// broker already told about a refused post. Nothing else is kept.
    accounts: HashMap<String, Account>,
}

struct Account {
    lifetime: Lifetime,
    /// Post refusals already reported under this lifetime: a surviving
    /// sender hears each once, not on every heartbeat.
    reported: HashSet<(u64, &'static str)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lifetime {
    generation: u64,
    world: BrokerWorld,
    active: bool,
}

impl Lifetime {
    fn live(self) -> bool {
        self.active && self.world != BrokerWorld::Unavailable
    }
}

struct Group {
    roster: [String; 4],
    members: HashMap<String, Member>,
    seq: HashMap<String, u64>,
    diagnosed: HashSet<String>,
}

#[derive(Clone)]
struct Member {
    account: String,
    slot: String,
    generation: u64,
    world: BrokerWorld,
    channel_id: u64,
    active: bool,
}

impl ChannelBroker {
    /// The handle the slot thread of `slot` keeps for its whole life.
    pub(crate) fn slot(&self, slot: &str) -> SlotChannels {
        let account = normalize(slot);
        let held = self.0.lock().accounts.contains_key(&account);
        SlotChannels {
            broker: self.clone(),
            slot: slot.to_string(),
            account,
            tracked: Arc::new(AtomicBool::new(held)),
        }
    }

    #[cfg(test)]
    fn member_count(&self, channel: &str) -> usize {
        self.0
            .lock()
            .groups
            .get(channel)
            .map_or(0, |group| group.members.len())
    }

    /// Groups and accounts the broker holds.
    #[cfg(test)]
    fn held(&self) -> (usize, usize) {
        let broker = self.0.lock();
        (broker.groups.len(), broker.accounts.len())
    }
}

impl SlotChannels {
    /// The broker holds a membership (live or suspended) or refusal state
    /// for this account. Lock-free: every slot reads it every frame.
    pub(crate) fn tracks(&self) -> bool {
        self.tracked.load(Ordering::Acquire)
    }

    /// Publish this frame's script lifetime and world, then run the
    /// isolate's channel requests under it, in one lock. A change to a
    /// dead lifetime (Stop, no world) or a different run or world revokes
    /// the membership; the same run back in the same world resumes a
    /// suspended one.
    pub(crate) fn pump(
        &self,
        generation: u64,
        world: BrokerWorld,
        active: bool,
        reqs: Vec<InteractReq>,
    ) -> Vec<Delivery> {
        let now = Lifetime {
            generation,
            world,
            active,
        };
        let mut broker = self.broker.0.lock();
        let mut deliveries = broker.sync(&self.account, &self.slot, now);
        if active {
            for req in reqs {
                deliveries.extend(broker.handle(&self.account, &self.slot, now, req));
            }
        }
        self.settle(&mut broker);
        deliveries
    }

    /// A session boundary the isolate survives (a relog or a logout): the
    /// membership stays, and the group delivers nothing until this account
    /// resumes it under `generation` in its world.
    pub(crate) fn suspend(&self, generation: u64) -> Vec<Delivery> {
        let mut broker = self.broker.0.lock();
        let deliveries = broker.suspend(&self.account, generation);
        self.settle(&mut broker);
        deliveries
    }

    /// The run or the slot thread ended: leave every group now.
    pub(crate) fn leave(&self) -> Vec<Delivery> {
        let mut broker = self.broker.0.lock();
        let deliveries = broker.revoke(&self.account);
        broker.accounts.remove(&self.account);
        self.settle(&mut broker);
        deliveries
    }

    /// Mirror what the broker still holds for this account onto the flag,
    /// under the lock that changed it. Another account's leave can only
    /// drop this one's memberships, so the flag is never falsely clear:
    /// at worst this slot takes the lock once more and settles again.
    fn settle(&self, broker: &mut Broker) {
        self.tracked
            .store(broker.settle(&self.account), Ordering::Release);
    }
}

impl Broker {
    fn sync(&mut self, account: &str, slot: &str, now: Lifetime) -> Vec<Delivery> {
        let old = self.accounts.get(account).map(|state| state.lifetime);
        if old == Some(now) {
            return Vec::new();
        }
        let resumed = if now.live() {
            self.resume(account, slot, now)
        } else {
            None
        };
        let deliveries = resumed.unwrap_or_else(|| self.revoke(account));
        let state = self
            .accounts
            .entry(account.to_string())
            .or_insert_with(|| Account {
                lifetime: now,
                reported: HashSet::new(),
            });
        state.lifetime = now;
        state.reported.clear();
        deliveries
    }

    fn handle(
        &mut self,
        account: &str,
        slot: &str,
        lifetime: Lifetime,
        req: InteractReq,
    ) -> Vec<Delivery> {
        match req {
            InteractReq::ChannelOpen { channel_id, name } => {
                self.open(account, slot, lifetime, channel_id, &name)
            }
            InteractReq::ChannelPost {
                channel_id,
                name,
                data,
            } => self.post(account, slot, lifetime, channel_id, &name, data),
            InteractReq::ChannelClose { channel_id, name } => {
                self.close(account, lifetime.generation, channel_id, &name)
            }
            _ => Vec::new(),
        }
    }

    /// Keep an account only while it holds a membership, or while its live
    /// run holds reported refusals. True when the account is kept.
    fn settle(&mut self, account: &str) -> bool {
        let keep = self.accounts.get(account).is_some_and(|state| {
            self.is_member(account) || (state.lifetime.live() && !state.reported.is_empty())
        });
        if !keep {
            self.accounts.remove(account);
        }
        keep
    }

    fn is_member(&self, account: &str) -> bool {
        self.groups
            .values()
            .any(|group| group.members.contains_key(account))
    }

    fn open(
        &mut self,
        account: &str,
        slot: &str,
        lifetime: Lifetime,
        channel_id: u64,
        name: &str,
    ) -> Vec<Delivery> {
        let generation = lifetime.generation;
        if channel_id == 0 {
            return Vec::new();
        }
        let roster = match parse_channel(name) {
            Ok(roster) => roster,
            Err(message) => {
                return vec![status(slot, generation, channel_id, message)];
            }
        };
        if !roster.iter().any(|member| member == account) {
            return vec![status(
                slot,
                generation,
                channel_id,
                "account is not in the JiveKQ roster",
            )];
        }
        if lifetime.world == BrokerWorld::Unavailable {
            return vec![status(
                slot,
                generation,
                channel_id,
                "account has no active world",
            )];
        }
        if !self.groups.contains_key(name) && self.groups.len() >= MAX_GROUPS {
            return vec![status(
                slot,
                generation,
                channel_id,
                "BroadcastChannel group limit reached",
            )];
        }
        let group = self
            .groups
            .entry(name.to_string())
            .or_insert_with(|| Group {
                roster: roster.clone(),
                members: HashMap::new(),
                seq: HashMap::new(),
                diagnosed: HashSet::new(),
            });
        if group.roster != roster {
            return vec![status(
                slot,
                generation,
                channel_id,
                "JiveKQ roster identity mismatch",
            )];
        }
        group.members.insert(
            account.to_string(),
            Member {
                account: account.to_string(),
                slot: slot.to_string(),
                generation,
                world: lifetime.world,
                channel_id,
                active: true,
            },
        );
        group.diagnosed.remove(account);
        group.diagnostics()
    }

    fn post(
        &mut self,
        account: &str,
        slot: &str,
        lifetime: Lifetime,
        channel_id: u64,
        name: &str,
        data: Vec<u8>,
    ) -> Vec<Delivery> {
        let refused = if script::channel::decode(&data).is_err() {
            Some("BroadcastChannel message failed validation")
        } else {
            match self.groups.get(name) {
                None => Some("BroadcastChannel was not opened"),
                Some(group)
                    if !group.members.get(account).is_some_and(|member| {
                        member.generation == lifetime.generation
                            && member.world == lifetime.world
                            && member.channel_id == channel_id
                    }) =>
                {
                    Some("BroadcastChannel sender lifetime is stale")
                }
                Some(_) => None,
            }
        };
        if let Some(message) = refused {
            let first = self
                .accounts
                .get_mut(account)
                .is_none_or(|state| state.reported.insert((channel_id, message)));
            return if first {
                vec![status(slot, lifetime.generation, channel_id, message)]
            } else {
                Vec::new()
            };
        }
        let Some(group) = self.groups.get_mut(name) else {
            return Vec::new();
        };
        if let Some(reason) = group.refusal() {
            if group.diagnosed.insert(account.to_string()) {
                return vec![status(slot, lifetime.generation, channel_id, reason)];
            }
            return Vec::new();
        }
        let seq = group.seq.entry(account.to_string()).or_default();
        *seq = seq.wrapping_add(1).max(1);
        let seq = *seq;
        group
            .roster
            .iter()
            .filter(|target| target.as_str() != account)
            .filter_map(|target| group.members.get(target))
            .map(|target| Delivery {
                account: target.slot.clone(),
                generation: target.generation,
                event: InteractReq::ChannelMessage {
                    channel_id: target.channel_id,
                    sender: account.to_string(),
                    seq,
                    data: data.clone(),
                },
            })
            .collect()
    }

    fn close(
        &mut self,
        account: &str,
        generation: u64,
        channel_id: u64,
        name: &str,
    ) -> Vec<Delivery> {
        let Some(group) = self.groups.get_mut(name) else {
            return Vec::new();
        };
        let owned = group.members.get(account).is_some_and(|member| {
            member.generation == generation && member.channel_id == channel_id
        });
        if !owned {
            return Vec::new();
        }
        group.members.remove(account);
        group.diagnosed.clear();
        let deliveries = group.diagnostics();
        if group.members.is_empty() {
            self.groups.remove(name);
        }
        deliveries
    }

    /// Hold the account's memberships across a session boundary. Its
    /// lifetime becomes dead, so the relogged session's first live pump
    /// either resumes them (same run, same world) or revokes them.
    fn suspend(&mut self, account: &str, generation: u64) -> Vec<Delivery> {
        let Some(state) = self.accounts.get_mut(account) else {
            return Vec::new();
        };
        state.lifetime = Lifetime {
            generation,
            world: BrokerWorld::Unavailable,
            active: false,
        };
        state.reported.clear();
        let mut deliveries = Vec::new();
        for group in self.groups.values_mut() {
            let Some(member) = group.members.get_mut(account) else {
                continue;
            };
            if member.active {
                member.active = false;
                group.diagnosed.clear();
                deliveries.extend(group.diagnostics());
            }
        }
        deliveries
    }

    /// Re-admit the account's suspended memberships under `now`. `None`
    /// when it holds none, or when any was suspended under another run
    /// generation or world: the caller revokes them all.
    fn resume(&mut self, account: &str, slot: &str, now: Lifetime) -> Option<Vec<Delivery>> {
        let mut suspended = self
            .groups
            .values()
            .filter_map(|group| group.members.get(account))
            .filter(|member| !member.active)
            .peekable();
        suspended.peek()?;
        if !suspended.all(|member| member.generation == now.generation && member.world == now.world)
        {
            return None;
        }
        let mut deliveries = Vec::new();
        for group in self.groups.values_mut() {
            let Some(member) = group.members.get_mut(account) else {
                continue;
            };
            if !member.active {
                member.slot = slot.to_string();
                member.active = true;
                group.diagnosed.clear();
                deliveries.extend(group.diagnostics());
            }
        }
        Some(deliveries)
    }

    fn revoke(&mut self, account: &str) -> Vec<Delivery> {
        let mut deliveries = Vec::new();
        self.groups.retain(|_, group| {
            if group.members.remove(account).is_some() {
                group.diagnosed.clear();
                for member in group.members.values() {
                    deliveries.push(status(
                        &member.slot,
                        member.generation,
                        member.channel_id,
                        LEFT,
                    ));
                    group.diagnosed.insert(member.account.clone());
                }
            }
            !group.members.is_empty()
        });
        deliveries
    }
}

impl Group {
    fn refusal(&self) -> Option<&'static str> {
        if self.members.len() != 4
            || self
                .roster
                .iter()
                .any(|account| !self.members.contains_key(account))
            || self.members.values().any(|member| !member.active)
        {
            return Some("waiting for all four JiveKQ roster members");
        }
        let mut worlds = self.members.values().map(|member| member.world);
        let first = worlds.next()?;
        if first == BrokerWorld::Unavailable || worlds.any(|world| world != first) {
            return Some("mixed-world JiveKQ roster refused");
        }
        None
    }

    fn diagnostics(&mut self) -> Vec<Delivery> {
        let Some(reason) = self.refusal() else {
            self.diagnosed.clear();
            return Vec::new();
        };
        self.members
            .values()
            .filter(|member| member.active)
            .filter(|member| self.diagnosed.insert(member.account.clone()))
            .map(|member| status(&member.slot, member.generation, member.channel_id, reason))
            .collect()
    }
}

fn status(account: &str, generation: u64, channel_id: u64, message: impl Into<String>) -> Delivery {
    Delivery {
        account: account.to_string(),
        generation,
        event: InteractReq::ChannelStatus {
            channel_id,
            message: message.into(),
        },
    }
}

fn normalize(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .split(|character: char| character == '_' || character.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_channel(name: &str) -> Result<[String; 4], &'static str> {
    let roster = name
        .strip_prefix(PREFIX)
        .ok_or("only rs2b0t:kq:v1 channels are supported")?
        .split(',')
        .map(normalize)
        .collect::<Vec<_>>();
    if roster.len() != 4
        || roster.iter().any(|name| name.is_empty() || name.len() > 12)
        || roster.iter().collect::<HashSet<_>>().len() != 4
    {
        return Err("JiveKQ channel requires four distinct account names");
    }
    roster
        .try_into()
        .map_err(|_| "JiveKQ channel requires four account names")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn channel() -> String {
        format!("{PREFIX}a,b,c,d")
    }

    fn open(slot: &SlotChannels, generation: u64, world: BrokerWorld, id: u64) -> Vec<Delivery> {
        slot.pump(
            generation,
            world,
            true,
            vec![InteractReq::ChannelOpen {
                channel_id: id,
                name: channel(),
            }],
        )
    }

    fn post_as(
        slot: &SlotChannels,
        generation: u64,
        world: BrokerWorld,
        id: u64,
        value: serde_json::Value,
    ) -> Vec<Delivery> {
        slot.pump(
            generation,
            world,
            true,
            vec![InteractReq::ChannelPost {
                channel_id: id,
                name: channel(),
                data: script::channel::encode(&value).unwrap(),
            }],
        )
    }

    fn post(slot: &SlotChannels, id: u64, value: serde_json::Value) -> Vec<Delivery> {
        post_as(slot, 1, BrokerWorld::Local, id, value)
    }

    /// Four members opened in `world` under generation 1, channel ids 1–4.
    fn party(broker: &ChannelBroker, world: BrokerWorld) -> Vec<SlotChannels> {
        ["a", "b", "c", "d"]
            .into_iter()
            .enumerate()
            .map(|(index, account)| {
                let slot = broker.slot(account);
                open(&slot, 1, world, index as u64 + 1);
                slot
            })
            .collect()
    }

    fn statuses(deliveries: &[Delivery]) -> Vec<&str> {
        deliveries
            .iter()
            .filter_map(|delivery| match &delivery.event {
                InteractReq::ChannelStatus { message, .. } => Some(message.as_str()),
                _ => None,
            })
            .collect()
    }

    fn reaches(deliveries: &[Delivery], account: &str) -> bool {
        deliveries.iter().any(|delivery| {
            delivery.account == account
                && matches!(delivery.event, InteractReq::ChannelMessage { .. })
        })
    }

    #[test]
    fn ordered_delivery_has_no_self_echo() {
        let broker = ChannelBroker::default();
        let party = party(&broker, BrokerWorld::Local);
        let first = post(&party[0], 1, json!({"member":1}));
        let second = post(&party[0], 1, json!({"member":2}));
        assert_eq!(first.len(), 3);
        assert!(first.iter().all(|delivery| delivery.account != "a"));
        for (one, two) in first.iter().zip(&second) {
            assert_eq!(one.account, two.account);
            let InteractReq::ChannelMessage { seq: one, .. } = one.event else {
                panic!()
            };
            let InteractReq::ChannelMessage { seq: two, .. } = two.event else {
                panic!()
            };
            assert_eq!((one, two), (1, 2));
        }
    }

    #[test]
    fn mixed_world_roster_is_refused_explicitly() {
        let broker = ChannelBroker::default();
        let slots = ["a", "b", "c", "d"].map(|account| broker.slot(account));
        for (index, slot) in slots[..3].iter().enumerate() {
            open(slot, 1, BrokerWorld::Public(1), index as u64 + 1);
        }
        let diagnostics = open(&slots[3], 1, BrokerWorld::Public(2), 4);
        assert!(statuses(&diagnostics)
            .iter()
            .any(|message| message.contains("mixed-world")));
        assert!(post_as(&slots[0], 1, BrokerWorld::Public(1), 1, json!({"member":1})).is_empty());
    }

    #[test]
    fn member_leave_revokes_delivery_until_fresh_open() {
        let broker = ChannelBroker::default();
        let party = party(&broker, BrokerWorld::Local);
        let diagnostics = party[3].leave();
        assert_eq!(broker.member_count(&channel()), 3);
        assert_eq!(diagnostics.len(), 3);
        assert!(post(&party[0], 1, json!({"member":1})).is_empty());
        open(&party[3], 2, BrokerWorld::Local, 40);
        assert_eq!(post(&party[0], 1, json!({"member":2})).len(), 3);
    }

    #[test]
    fn a_suspended_member_resumes_only_under_its_run_and_world() {
        for (generation, world, resumed) in [
            (1, BrokerWorld::Public(289), true),
            // A Stop then Start during the relog window is another run.
            (2, BrokerWorld::Public(289), false),
            (1, BrokerWorld::Public(274), false),
        ] {
            let broker = ChannelBroker::default();
            let party = party(&broker, BrokerWorld::Public(289));
            let told = party[3].suspend(1);
            assert_eq!(broker.member_count(&channel()), 4);
            assert_eq!(statuses(&told).len(), 3, "the others wait for d");
            assert!(party[3].tracks(), "the suspended member stays tracked");

            party[3].pump(generation, world, true, Vec::new());
            let delivered = post_as(
                &party[0],
                1,
                BrokerWorld::Public(289),
                1,
                json!({"member":2}),
            );
            assert_eq!(
                reaches(&delivered, "d"),
                resumed,
                "generation {generation} in {world:?}"
            );
            assert_eq!(broker.member_count(&channel()), if resumed { 4 } else { 3 });
            assert_eq!(party[3].tracks(), resumed);
        }
    }

    #[test]
    fn a_stale_sender_hears_each_refusal_once_per_run() {
        let broker = ChannelBroker::default();
        let party = party(&broker, BrokerWorld::Public(289));
        party[3].suspend(1);
        // The surviving isolate comes back in another world: revoked.
        party[3].pump(1, BrokerWorld::Public(274), true, Vec::new());
        let heartbeats = (0..3)
            .map(|n| post_as(&party[3], 1, BrokerWorld::Public(274), 4, json!({ "n": n })))
            .collect::<Vec<_>>();
        assert_eq!(
            statuses(&heartbeats[0]),
            ["BroadcastChannel sender lifetime is stale"]
        );
        assert!(heartbeats[1..].iter().all(Vec::is_empty), "{heartbeats:?}");
        // A new run is told again.
        let restarted = post_as(&party[3], 2, BrokerWorld::Public(274), 4, json!({}));
        assert_eq!(
            statuses(&restarted),
            ["BroadcastChannel sender lifetime is stale"]
        );
    }

    #[test]
    fn the_broker_keeps_no_state_for_accounts_without_a_channel() {
        let broker = ChannelBroker::default();
        let bystander = broker.slot("x");
        let refused = open(&bystander, 1, BrokerWorld::Local, 1);
        assert!(statuses(&refused)[0].contains("not in"));
        assert!(!bystander.tracks());
        let party = party(&broker, BrokerWorld::Local);
        assert!(party.iter().all(SlotChannels::tracks));
        for slot in &party {
            slot.leave();
        }
        assert!(party.iter().all(|slot| !slot.tracks()));
        assert_eq!(broker.held(), (0, 0));
    }

    /// Profiles come and go for the life of a Play, each party with its own
    /// roster. Nothing a slot leaves behind outlives its handle.
    #[test]
    fn profile_churn_leaves_the_broker_nothing() {
        let broker = ChannelBroker::default();
        let mut flags = Vec::new();
        for party in 0..25 {
            let accounts = (0..4)
                .map(|member| format!("p{party}_{member}"))
                .collect::<Vec<_>>();
            let name = format!("{PREFIX}{}", accounts.join(","));
            let slots = accounts
                .iter()
                .map(|account| broker.slot(account))
                .collect::<Vec<_>>();
            for (index, slot) in slots.iter().enumerate() {
                slot.pump(1, BrokerWorld::Local, true, Vec::new());
                assert!(!slot.tracks(), "a frame with no channel settles untracked");
                let open = InteractReq::ChannelOpen {
                    channel_id: index as u64 + 1,
                    name: name.clone(),
                };
                slot.pump(1, BrokerWorld::Local, true, vec![open]);
                assert!(slot.tracks(), "an open channel is tracked again");
            }
            for slot in &slots {
                slot.leave();
                assert!(!slot.tracks());
                flags.push(Arc::downgrade(&slot.tracked));
            }
        }
        assert_eq!(broker.held(), (0, 0));
        assert!(
            flags.iter().all(|flag| flag.upgrade().is_none()),
            "the broker keeps a departed slot's flag"
        );
    }

    #[test]
    fn normalized_roster_delivers_to_original_slot_keys() {
        let broker = ChannelBroker::default();
        let channel = format!("{PREFIX}team_0,team_1,team_2,team_3");
        let slots = ["team_0", "team_1", "team_2", "team_3"].map(|account| broker.slot(account));
        for (index, slot) in slots.iter().enumerate() {
            slot.pump(
                1,
                BrokerWorld::Local,
                true,
                vec![InteractReq::ChannelOpen {
                    channel_id: index as u64 + 1,
                    name: channel.clone(),
                }],
            );
        }
        let deliveries = slots[0].pump(
            1,
            BrokerWorld::Local,
            true,
            vec![InteractReq::ChannelPost {
                channel_id: 1,
                name: channel,
                data: script::channel::encode(&json!({"member":1})).unwrap(),
            }],
        );
        let mut slots = deliveries
            .into_iter()
            .map(|delivery| delivery.account)
            .collect::<Vec<_>>();
        slots.sort();
        assert_eq!(slots, ["team_1", "team_2", "team_3"]);
    }
}
