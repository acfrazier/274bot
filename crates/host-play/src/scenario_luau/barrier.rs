//! Two-query ordered read barrier for File root closure (LD2 §5.1).
//!
//! Query 1 is queued behind the root on the same connection. Query 2 is sent
//! only after the client applied the fresh reply to query 1. Inputs decode in
//! order and one synchronous world cycle runs inputs, player interactions and
//! client output (engine `NetworkPlayer.ts:54-150`, `World.ts:344-412`), and
//! socket data only buffers between cycles (`server/tcp/TcpServer.ts:22-35`,
//! `server/tcp/TcpClientSocket.ts:15-17`). Query 2 therefore cannot decode in
//! the root/query-1 input cycle, and its applied reply follows that cycle's
//! output. The barrier proves processing and progress, not which branch ran.

use api::snapshot::GameSnapshot;
use client::client::Client;
use serde_json::{json, Value};

/// Observed ticks a query may wait for its fresh reply.
const QUERY_TICK_BUDGET: u64 = 20;
const ACCESS_REFUSAL: &str = "Please finish what you are doing first.";

#[derive(Debug)]
struct Query {
    sent_tick: u64,
    chat_baseline: Option<i32>,
    reply: Option<(i32, String)>,
}

/// One in-flight barrier for one root.
#[derive(Debug)]
pub(super) struct Barrier {
    varp: String,
    session: u64,
    queries: Vec<Query>,
}

impl Barrier {
    /// Send query 1. `varp` must be a plain ordinary-varp debug name with no
    /// varbit of the same name (getvar resolves varbits first,
    /// `ClientCheatHandler.ts:279-318`).
    pub(super) fn start(
        client: &mut Client,
        snapshot: &GameSnapshot,
        varp: &str,
    ) -> Result<Self, String> {
        let mut barrier = Self {
            varp: varp.to_owned(),
            session: snapshot.gens().session,
            queries: Vec::with_capacity(2),
        };
        barrier.send(client, snapshot)?;
        Ok(barrier)
    }

    fn send(&mut self, client: &mut Client, snapshot: &GameSnapshot) -> Result<(), String> {
        let command = format!("getvar {}", self.varp);
        let chat_baseline = snapshot.chat_lines().first().map(|line| line.sequence);
        match api::interact::cheat(client, &command) {
            client::CheatSend::Sent => {}
            client::CheatSend::Refused(reason) => {
                return Err(format!("barrier query send was refused: {reason:?}"))
            }
        }
        self.queries.push(Query {
            sent_tick: u64::from(snapshot.tick()),
            chat_baseline,
            reply: None,
        });
        Ok(())
    }

    /// Returns true once reply 2 was applied. Never retries a query.
    pub(super) fn poll(
        &mut self,
        client: &mut Client,
        snapshot: &GameSnapshot,
    ) -> Result<bool, String> {
        if snapshot.gens().session != self.session {
            return Err("barrier crossed a session change".into());
        }
        let prefix = format!("get {}: ", self.varp);
        let query = self.queries.last_mut().ok_or("barrier has no query")?;
        if query.reply.is_none() {
            let mut fresh = None;
            for line in snapshot.chat_lines() {
                if line.type_ != 0 || !after(line.sequence, query.chat_baseline) {
                    continue;
                }
                if line.text == ACCESS_REFUSAL {
                    return Err("barrier query was refused by the engine".into());
                }
                let Some(value) = line.text.strip_prefix(&prefix) else {
                    continue;
                };
                if value.parse::<i32>().is_err() {
                    return Err("barrier reply is not an ordinary integer varp value".into());
                }
                if fresh.is_some() {
                    return Err("barrier query has more than one fresh reply".into());
                }
                fresh = Some((line.sequence, line.text.clone()));
            }
            match fresh {
                Some(reply) => query.reply = Some(reply),
                None if u64::from(snapshot.tick()) >= query.sent_tick + QUERY_TICK_BUDGET => {
                    return Err(
                        "barrier query reply did not arrive within its observed tick budget".into(),
                    )
                }
                None => return Ok(false),
            }
        }
        if self.queries.len() == 1 {
            // The reply above is applied in this snapshot: only now queue query 2.
            self.send(client, snapshot)?;
            return Ok(false);
        }
        Ok(true)
    }

    /// Chat sequence of the final applied reply, the closing source boundary.
    pub(super) fn boundary(&self) -> Option<i32> {
        self.queries.get(1)?.reply.as_ref().map(|reply| reply.0)
    }

    /// Control receipts, labelled as controls and kept unchanged.
    pub(super) fn record(&self) -> Value {
        json!({
            "control": "ordered-read-barrier",
            "varp": self.varp,
            "session": self.session,
            "queries": self.queries.iter().map(|query| json!({
                "sent_tick": query.sent_tick,
                "chat_baseline": query.chat_baseline,
                "reply_sequence": query.reply.as_ref().map(|reply| reply.0),
                "reply": query.reply.as_ref().map(|reply| reply.1.as_str()),
            })).collect::<Vec<_>>(),
        })
    }

    /// True for a line that is one of this barrier's own control replies.
    pub(super) fn is_control(&self, sequence: i32) -> bool {
        self.queries.iter().any(|query| {
            query
                .reply
                .as_ref()
                .is_some_and(|reply| reply.0 == sequence)
        })
    }
}

fn after(sequence: i32, baseline: Option<i32>) -> bool {
    baseline.is_none_or(|baseline| sequence.wrapping_sub(baseline) > 0)
}

#[cfg(test)]
#[path = "barrier_tests.rs"]
mod tests;
