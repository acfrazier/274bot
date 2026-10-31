use std::sync::{LazyLock, Mutex};

use api::hostlog::{self, Record, Sink};
use client::client::Client;
use host_play::{SlotStatus, StartupPhase};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSessionObservation {
    pub host_frame: u64,
    pub session: u64,
    pub ingame: bool,
    pub scene_state: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotObservation {
    pub host_frame: u64,
    pub connected: bool,
    pub ingame: bool,
    pub scene_state: i32,
    pub startup_phase: StartupPhase,
    pub stream_epoch: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLogObservation {
    pub slot: Option<String>,
    pub tick: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct DropHistory {
    pub client_sessions: Vec<ClientSessionObservation>,
    pub slot_status: Vec<SlotObservation>,
    pub host_log: Vec<HostLogObservation>,
}

impl DropHistory {
    pub fn observe_client(&mut self, client: &Client, host_frame: u64) {
        let observation = ClientSessionObservation {
            host_frame,
            session: client.gens.session,
            ingame: client.ingame,
            scene_state: client.scene_state,
        };
        if self.client_sessions.last().is_none_or(|previous| {
            previous.session != observation.session
                || previous.ingame != observation.ingame
                || previous.scene_state != observation.scene_state
        }) {
            self.client_sessions.push(observation);
        }
    }

    pub fn observe_slot(&mut self, status: &SlotStatus, host_frame: u64) {
        let observation = SlotObservation {
            host_frame,
            connected: status.connected,
            ingame: status.ingame,
            scene_state: status.scene_state,
            startup_phase: status.startup_phase,
            stream_epoch: status.stream_epoch,
        };
        if self.slot_status.last().is_none_or(|previous| {
            previous.connected != observation.connected
                || previous.ingame != observation.ingame
                || previous.scene_state != observation.scene_state
                || previous.startup_phase != observation.startup_phase
                || previous.stream_epoch != observation.stream_epoch
        }) {
            self.slot_status.push(observation);
        }
    }

    pub fn replace_host_log(&mut self, records: Vec<HostLogObservation>) {
        self.host_log = records;
    }

    pub fn json(&self) -> Value {
        json!({
            "client_sessions": self.client_sessions.iter().map(|observation| json!({
                "host_frame": observation.host_frame,
                "session": observation.session,
                "ingame": observation.ingame,
                "scene_state": observation.scene_state,
            })).collect::<Vec<_>>(),
            "slot_status": self.slot_status.iter().map(|observation| json!({
                "host_frame": observation.host_frame,
                "connected": observation.connected,
                "ingame": observation.ingame,
                "scene_state": observation.scene_state,
                "startup_phase": format!("{:?}", observation.startup_phase),
                "stream_epoch": observation.stream_epoch,
            })).collect::<Vec<_>>(),
            "host_log": self.host_log.iter().map(|observation| json!({
                "slot": observation.slot,
                "tick": observation.tick,
                "message": observation.message,
            })).collect::<Vec<_>>(),
        })
    }
}

struct Capture(Mutex<Vec<HostLogObservation>>);

impl Sink for Capture {
    fn record(&self, record: &Record<'_>) {
        let message = record.message;
        if message.starts_with("session exit")
            || message.starts_with("relog decision")
            || message.starts_with("unexpected logout")
            || message.starts_with("handshake ok")
            || message.starts_with("login code")
            || message.starts_with("scene ")
        {
            if let Ok(mut rows) = self.0.lock() {
                rows.push(HostLogObservation {
                    slot: record.slot.map(str::to_owned),
                    tick: record.tick,
                    message: message.to_owned(),
                });
            }
        }
    }
}

static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));
static INSTALLED: LazyLock<bool> = LazyLock::new(|| hostlog::install_sink(&CAPTURE));

pub fn begin_case() -> Result<(), String> {
    if !*INSTALLED {
        return Err(
            "another host log sink is already installed in the tick-engine test process".into(),
        );
    }
    CAPTURE
        .0
        .lock()
        .map_err(|_| "drop lifecycle host log capture poisoned".to_owned())?
        .clear();
    Ok(())
}

pub fn clear_host_log() -> Result<(), String> {
    CAPTURE
        .0
        .lock()
        .map_err(|_| "drop lifecycle host log capture poisoned".to_owned())?
        .clear();
    Ok(())
}

pub fn host_log_for(slot: &str) -> Vec<HostLogObservation> {
    CAPTURE
        .0
        .lock()
        .expect("drop lifecycle host log capture poisoned")
        .iter()
        .filter(|observation| observation.slot.as_deref() == Some(slot))
        .cloned()
        .collect()
}
