//! Scenario adapter for the ordinary, ownership-aware native key input.

use std::sync::Arc;

use api::native_input::{KeyGesture, KeyLease, KeyPhase, KeyProgress, NativeInputAuthority};
use api::snapshot::GameSnapshot;
use serde::Serialize;

/// One native key hold and its existing snapshot camera observations.
#[derive(Debug, Clone, Serialize)]
pub struct KeyEvidence {
    /// Client session that admitted the gesture.
    pub session: u64,
    /// Native consumer progress, not merely request admission.
    pub input: KeyProgress,
    /// Orbit yaw and pitch before the gesture.
    pub camera_before: [i32; 2],
    /// Orbit yaw and pitch when the gesture ended.
    pub camera_after: [i32; 2],
}

#[derive(Default)]
pub(crate) struct KeyRun {
    pub input: Option<Arc<NativeInputAuthority>>,
    lease: Option<KeyLease>,
    session: u64,
    observed_session: u64,
    before: [i32; 2],
    after: [i32; 2],
    pub records: Vec<KeyEvidence>,
}

impl KeyRun {
    pub fn begin(
        &mut self,
        gesture: KeyGesture,
        snapshot: &GameSnapshot,
        session: u64,
    ) -> Result<(), String> {
        if self.records.len() >= 1024 {
            return Err("key evidence limit reached".into());
        }
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return Err("key gesture requires a ready scene".into());
        }
        let input = self
            .input
            .as_ref()
            .ok_or("native key input is unavailable")?;
        self.lease = Some(
            input
                .key_gesture(gesture)
                .map_err(|error| error.to_string())?,
        );
        self.session = session;
        self.before = camera(snapshot);
        self.after = self.before;
        Ok(())
    }

    pub fn observe(&mut self, snapshot: &GameSnapshot, session: u64) -> Result<(), &'static str> {
        self.observed_session = session;
        if self.lease.is_none() {
            return Ok(());
        }
        self.after = camera(snapshot);
        if !snapshot.ingame() || self.session != session {
            self.cancel();
            return Err("key gesture session ended");
        }
        let progress = self.lease.as_ref().and_then(KeyLease::progress);
        match progress.map(|progress| progress.phase) {
            Some(KeyPhase::Released) => self.complete(),
            Some(KeyPhase::Cancelled) | None => {
                self.cancel();
                return Err("key gesture lost input ownership");
            }
            _ => {}
        }
        Ok(())
    }

    pub fn released(&self) -> bool {
        self.lease.is_none()
            && self.records.last().is_some_and(|record| {
                record.session == self.observed_session && record.input.phase == KeyPhase::Released
            })
    }

    pub fn camera_delta(&self, snapshot: &GameSnapshot) -> bool {
        self.lease.is_none()
            && self.records.last().is_some_and(|record| {
                record.session == self.observed_session
                    && record.input.phase == KeyPhase::Released
                    && record.input.applied_frames > 0
                    && record.input.held_ms >= record.input.duration_ms
                    && record.camera_before != record.camera_after
                    && snapshot.ingame()
            })
    }

    fn complete(&mut self) {
        if let Some(lease) = self.lease.take() {
            if let Some(input) = lease.progress() {
                self.records.push(KeyEvidence {
                    session: self.session,
                    input,
                    camera_before: self.before,
                    camera_after: self.after,
                });
            }
        }
    }

    pub fn cancel(&mut self) {
        if let Some(lease) = &self.lease {
            let input = Arc::clone(self.input.as_ref().expect("a key lease has its input"));
            let ticket = lease.progress().map(|progress| progress.ticket);
            drop(self.lease.take());
            if let Some(input) = ticket.and_then(|ticket| input.key_progress(ticket)) {
                self.records.push(KeyEvidence {
                    session: self.session,
                    input,
                    camera_before: self.before,
                    camera_after: self.after,
                });
            }
        }
    }
}

fn camera(snapshot: &GameSnapshot) -> [i32; 2] {
    let camera = snapshot.camera();
    [camera.orbit_yaw, camera.orbit_pitch]
}
