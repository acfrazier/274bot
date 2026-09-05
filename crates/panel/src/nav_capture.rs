//! Panel GPU readback for opt-in navigation checkpoints. Captures can switch
//! focus, so this mode is diagnostic evidence, not a memory baseline.
use crate::{session::Session, window::ShotState};
use host_play::nav_capture::{self, Checkpoint};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Active {
    checkpoint: Checkpoint,
    started: Instant,
    submitted: bool,
}
#[derive(Default)]
pub struct Captures {
    active: Option<Active>,
    terminal: Option<(Instant, String)>,
}
impl Captures {
    pub fn failed(&mut self, error: String) {
        if self.terminal.is_none() {
            nav_capture::request_terminal(&error);
            self.terminal = Some((Instant::now(), error));
        }
    }
    pub fn has_terminal(&self) -> bool {
        self.terminal.is_some()
    }
    pub fn busy(&self) -> bool {
        self.active.is_some() || nav_capture::pending()
    }
    pub fn written(&mut self, label: &str) {
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.submitted && a.checkpoint.label == label)
        {
            self.active = None;
        }
    }
    /// Return the original failure after evidence has drained, or after a
    /// bounded capture timeout. An unrelated/manual shot cannot acknowledge it.
    pub fn terminal_ready(&self) -> Option<&str> {
        let (started, error) = self.terminal.as_ref()?;
        if !self.busy() || started.elapsed() > Duration::from_secs(25) {
            Some(error)
        } else {
            None
        }
    }
    pub fn tick(&mut self, session: &mut Session, shots: &Arc<Mutex<ShotState>>) {
        if self.active.is_none() {
            if let Some(checkpoint) = nav_capture::take() {
                session.select(&checkpoint.name);
                self.active = Some(Active {
                    checkpoint,
                    started: Instant::now(),
                    submitted: false,
                });
            }
        }
        let Some(active) = self.active.as_mut() else {
            return;
        };
        // Reapply one head, even if operator focus changed during settling.
        session.select(&active.checkpoint.name);
        {
            let mut focus = session.focus.lock().unwrap();
            focus.renderer = true;
            focus.game_pane_open = true;
            focus.only_render_selected = true;
        }
        if active.started.elapsed() > Duration::from_secs(20) {
            eprintln!(
                "[nav-capture] TIMEOUT {} submitted={}",
                active.checkpoint.label, active.submitted
            );
            let error = format!("diagnostic capture timed out: {}", active.checkpoint.label);
            self.active = None;
            self.failed(error);
            return;
        }
        if active.submitted {
            return;
        }
        let view = nav_capture::view(&active.checkpoint.name);
        if active.started.elapsed() < Duration::from_secs(2)
            || view["scene_state"] != 2
            || view["ingame"] != true
            || view["drawing"] != true
        {
            return;
        }
        let evidence = json!({"event":active.checkpoint.evidence,"capture_request_state":view,
            "capture_delay_ms":active.started.elapsed().as_millis(),
            "pixel_state_note":"GPU readback is later than event snapshot; capture_request_state is the latest observed native state, not an asserted source tick of asynchronously rendered pixels."});
        shots.lock().unwrap().requests.push((
            active.checkpoint.label.clone(),
            serde_json::to_string_pretty(&evidence).expect("checkpoint JSON"),
        ));
        eprintln!("[nav-capture] submitted {}", active.checkpoint.label);
        active.submitted = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_matching_written_checkpoint_releases_pending_capture() {
        let mut c = Captures {
            active: Some(Active {
                checkpoint: Checkpoint {
                    name: "bot".into(),
                    label: "nav-failure".into(),
                    evidence: json!({"event_tick":12}),
                },
                started: Instant::now(),
                submitted: true,
            }),
            terminal: None,
        };
        c.written("manual-shot");
        assert!(c.active.is_some());
        c.written("nav-failure");
        assert!(c.active.is_none());
    }
    #[test]
    fn failure_drain_has_a_deadline_and_keeps_original_error() {
        let c = Captures {
            active: Some(Active {
                checkpoint: Checkpoint {
                    name: "bot".into(),
                    label: "failure".into(),
                    evidence: json!({}),
                },
                started: Instant::now(),
                submitted: true,
            }),
            terminal: Some((
                Instant::now() - Duration::from_secs(30),
                "original failure".into(),
            )),
        };
        assert_eq!(c.terminal_ready(), Some("original failure"));
    }
}
