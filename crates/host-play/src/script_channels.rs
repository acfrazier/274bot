//! One-Play, one-world BroadcastChannel broker for the frozen JiveKQ card.
//!
//! The browser API is only a shim endpoint. Membership, roster admission,
//! world fencing, sequencing and lifecycle revocation live here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;
use script::shim::InteractReq;

const PREFIX: &str = "rs2b0t:kq:v1:";
const MAX_GROUPS: usize = 16;

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

#[derive(Clone, Default)]
pub(crate) struct ChannelBroker(Arc<Mutex<Broker>>);

#[derive(Default)]
struct Broker {
    groups: HashMap<String, Group>,
    lifetimes: HashMap<String, Lifetime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lifetime {
    generation: u64,
    world: BrokerWorld,
    active: bool,
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
    /// Publish the current script lifetime/world. A suspended member from the
    /// same world follows its surviving isolate onto the new generation.
    /// Stops, live generation changes, and world changes revoke membership.
    pub(crate) fn sync(
        &self,
        account: &str,
        generation: u64,
        world: BrokerWorld,
        active: bool,
    ) -> Vec<Delivery> {
        let slot = account;
        let account = normalize(account);
        let mut broker = self.0.lock();
        let now = Lifetime {
            generation,
            world,
            active,
        };
        let old = broker.lifetimes.get(&account).copied();
        let deliveries = if !active || world == BrokerWorld::Unavailable {
            broker.revoke(&account, "party member left or changed world")
        } else if old == Some(now) {
            Vec::new()
        } else {
            match broker.resume(&account, slot, generation, world) {
                Ok(Some(deliveries)) => deliveries,
                Ok(None) if old.is_none() => Vec::new(),
                Ok(None) | Err(_) => broker.revoke(&account, "party member left or changed world"),
            }
        };
        broker.lifetimes.insert(account, now);
        deliveries
    }

    /// Pause channel admission across a reconnect while retaining enough
    /// membership to re-admit the same surviving isolate in the same world.
    pub(crate) fn suspend(&self, account: &str, generation: u64) -> Vec<Delivery> {
        let account = normalize(account);
        let mut broker = self.0.lock();
        broker.lifetimes.insert(
            account.clone(),
            Lifetime {
                generation,
                world: BrokerWorld::Unavailable,
                active: false,
            },
        );
        broker.suspend(&account)
    }

    /// The account owns a live or reconnect-suspended channel. This cheap
    /// gate keeps ordinary slots off the broker's sync path.
    pub(crate) fn tracks(&self, account: &str) -> bool {
        self.0
            .lock()
            .groups
            .values()
            .any(|group| group.members.values().any(|member| member.slot == account))
    }

    pub(crate) fn handle(
        &self,
        account: &str,
        generation: u64,
        world: BrokerWorld,
        req: InteractReq,
    ) -> Vec<Delivery> {
        let slot = account.to_string();
        let account = normalize(account);
        let mut broker = self.0.lock();
        let lifetime = Lifetime {
            generation,
            world,
            active: true,
        };
        if broker.lifetimes.get(&account).copied() != Some(lifetime) {
            return Vec::new();
        }
        match req {
            InteractReq::ChannelOpen { channel_id, name } => {
                broker.open(&account, &slot, generation, world, channel_id, &name)
            }
            InteractReq::ChannelPost {
                channel_id,
                name,
                data,
            } => broker.post(&account, &slot, lifetime, channel_id, &name, data),
            InteractReq::ChannelClose { channel_id, name } => {
                broker.close(&account, generation, channel_id, &name)
            }
            _ => Vec::new(),
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
}

impl Broker {
    fn open(
        &mut self,
        account: &str,
        slot: &str,
        generation: u64,
        world: BrokerWorld,
        channel_id: u64,
        name: &str,
    ) -> Vec<Delivery> {
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
        if world == BrokerWorld::Unavailable {
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
                world,
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
        if script::channel::decode(&data).is_err() {
            return vec![status(
                slot,
                lifetime.generation,
                channel_id,
                "BroadcastChannel message failed validation",
            )];
        }
        let Some(group) = self.groups.get_mut(name) else {
            return vec![status(
                slot,
                lifetime.generation,
                channel_id,
                "BroadcastChannel was not opened",
            )];
        };
        let authenticated = group.members.get(account).is_some_and(|member| {
            member.generation == lifetime.generation
                && member.world == lifetime.world
                && member.channel_id == channel_id
        });
        if !authenticated {
            return vec![status(
                slot,
                lifetime.generation,
                channel_id,
                "BroadcastChannel sender lifetime is stale",
            )];
        }
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

    fn suspend(&mut self, account: &str) -> Vec<Delivery> {
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

    fn resume(
        &mut self,
        account: &str,
        slot: &str,
        generation: u64,
        world: BrokerWorld,
    ) -> Result<Option<Vec<Delivery>>, BrokerWorld> {
        let mut found = false;
        for member in self
            .groups
            .values()
            .filter_map(|group| group.members.get(account))
            .filter(|member| !member.active)
        {
            found = true;
            if member.world != world {
                return Err(member.world);
            }
        }
        if !found {
            return Ok(None);
        }
        let mut deliveries = Vec::new();
        for group in self.groups.values_mut() {
            let Some(member) = group.members.get_mut(account) else {
                continue;
            };
            if !member.active {
                member.slot = slot.to_string();
                member.generation = generation;
                member.active = true;
                group.diagnosed.clear();
                deliveries.extend(group.diagnostics());
            }
        }
        Ok(Some(deliveries))
    }

    fn revoke(&mut self, account: &str, reason: &str) -> Vec<Delivery> {
        let mut deliveries = Vec::new();
        self.groups.retain(|_, group| {
            if group.members.remove(account).is_some() {
                group.diagnosed.clear();
                for member in group.members.values() {
                    deliveries.push(status(
                        &member.slot,
                        member.generation,
                        member.channel_id,
                        reason,
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

    fn open(
        broker: &ChannelBroker,
        account: &str,
        generation: u64,
        world: BrokerWorld,
        id: u64,
    ) -> Vec<Delivery> {
        broker.sync(account, generation, world, true);
        broker.handle(
            account,
            generation,
            world,
            InteractReq::ChannelOpen {
                channel_id: id,
                name: channel(),
            },
        )
    }

    fn post(
        broker: &ChannelBroker,
        account: &str,
        id: u64,
        value: serde_json::Value,
    ) -> Vec<Delivery> {
        broker.handle(
            account,
            1,
            BrokerWorld::Local,
            InteractReq::ChannelPost {
                channel_id: id,
                name: channel(),
                data: script::channel::encode(&value).unwrap(),
            },
        )
    }

    #[test]
    fn ordered_delivery_has_no_self_echo() {
        let broker = ChannelBroker::default();
        for (index, account) in ["a", "b", "c", "d"].into_iter().enumerate() {
            open(&broker, account, 1, BrokerWorld::Local, index as u64 + 1);
        }
        let first = post(&broker, "a", 1, json!({"member":1}));
        let second = post(&broker, "a", 1, json!({"member":2}));
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
        for (index, account) in ["a", "b", "c"].into_iter().enumerate() {
            open(
                &broker,
                account,
                1,
                BrokerWorld::Public(1),
                index as u64 + 1,
            );
        }
        let diagnostics = open(&broker, "d", 1, BrokerWorld::Public(2), 4);
        assert!(diagnostics.iter().any(|delivery| matches!(
            &delivery.event,
            InteractReq::ChannelStatus { message, .. } if message.contains("mixed-world")
        )));
        assert!(post(&broker, "a", 1, json!({"member":1})).is_empty());
    }

    #[test]
    fn member_leave_revokes_delivery_until_fresh_open() {
        let broker = ChannelBroker::default();
        for (index, account) in ["a", "b", "c", "d"].into_iter().enumerate() {
            open(&broker, account, 1, BrokerWorld::Local, index as u64 + 1);
        }
        let diagnostics = broker.sync("d", 1, BrokerWorld::Unavailable, false);
        assert_eq!(broker.member_count(&channel()), 3);
        assert_eq!(diagnostics.len(), 3);
        assert!(post(&broker, "a", 1, json!({"member":1})).is_empty());
        open(&broker, "d", 2, BrokerWorld::Local, 40);
        assert_eq!(post(&broker, "a", 1, json!({"member":2})).len(), 3);
    }

    #[test]
    fn suspended_member_follows_surviving_isolate_session_in_same_world() {
        let broker = ChannelBroker::default();
        for (index, account) in ["a", "b", "c", "d"].into_iter().enumerate() {
            open(
                &broker,
                account,
                1,
                BrokerWorld::Public(289),
                index as u64 + 1,
            );
        }
        let diagnostics = broker.suspend("d", 1);
        assert_eq!(broker.member_count(&channel()), 4);
        assert_eq!(diagnostics.len(), 3);
        assert!(broker.tracks("d"));

        broker.sync("d", 1, BrokerWorld::Public(289), true);
        let deliveries = broker.handle(
            "a",
            1,
            BrokerWorld::Public(289),
            InteractReq::ChannelPost {
                channel_id: 1,
                name: channel(),
                data: script::channel::encode(&json!({"member":2})).unwrap(),
            },
        );
        assert_eq!(deliveries.len(), 3);
        assert!(deliveries.iter().any(|delivery| {
            delivery.account == "d"
                && delivery.generation == 1
                && matches!(
                    delivery.event,
                    InteractReq::ChannelMessage { channel_id: 4, .. }
                )
        }));
    }

    #[test]
    fn suspended_member_is_revoked_after_world_change() {
        let broker = ChannelBroker::default();
        for (index, account) in ["a", "b", "c", "d"].into_iter().enumerate() {
            open(
                &broker,
                account,
                1,
                BrokerWorld::Public(289),
                index as u64 + 1,
            );
        }
        broker.suspend("d", 1);
        broker.sync("d", 2, BrokerWorld::Public(274), true);
        assert_eq!(broker.member_count(&channel()), 3);
        assert!(!broker.tracks("d"));
    }

    #[test]
    fn sender_must_belong_to_exact_roster() {
        let broker = ChannelBroker::default();
        let diagnostics = open(&broker, "x", 1, BrokerWorld::Local, 1);
        assert!(matches!(
            &diagnostics[0].event,
            InteractReq::ChannelStatus { message, .. } if message.contains("not in")
        ));
    }

    #[test]
    fn normalized_roster_delivers_to_original_slot_keys() {
        let broker = ChannelBroker::default();
        let channel = format!("{PREFIX}team_0,team_1,team_2,team_3");
        for (index, account) in ["team_0", "team_1", "team_2", "team_3"]
            .into_iter()
            .enumerate()
        {
            broker.sync(account, 1, BrokerWorld::Local, true);
            broker.handle(
                account,
                1,
                BrokerWorld::Local,
                InteractReq::ChannelOpen {
                    channel_id: index as u64 + 1,
                    name: channel.clone(),
                },
            );
        }
        let deliveries = broker.handle(
            "team_0",
            1,
            BrokerWorld::Local,
            InteractReq::ChannelPost {
                channel_id: 1,
                name: channel,
                data: script::channel::encode(&json!({"member":1})).unwrap(),
            },
        );
        let mut slots = deliveries
            .into_iter()
            .map(|delivery| delivery.account)
            .collect::<Vec<_>>();
        slots.sort();
        assert_eq!(slots, ["team_1", "team_2", "team_3"]);
    }
}
