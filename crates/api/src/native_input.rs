//! Per-slot native input identity and eligibility.
//!
//! Shared between `SlotScript` (publisher / revoker) and host `SlotInput`
//! (consumer). Contains only the current identity, eligibility, and the
//! mutex that linearizes revoke against consume. It is not mouse policy
//! and not a global registry.
//!
//! Consume must hold [`NativeInputAuthority::lock`] across validation,
//! shell application, and `latch_click`, then drop it before `mainloop`
//! or an isolate join. If the consumer also locks a mouse queue, this
//! mutex is first.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Logical applet size, bound to the client constants so script/host agree
/// without a script→client import.
pub const APPLET_W: i32 = client::client::APPLET_W;
pub const APPLET_H: i32 = client::client::APPLET_H;

/// Snapshot of the live identity and whether script input may consume.
#[derive(Debug)]
pub struct NativeInputPermit {
    identity: u64,
    eligible: bool,
    keys: Option<Box<KeyState>>,
}

impl NativeInputPermit {
    pub fn identity(&self) -> u64 {
        self.identity
    }

    pub fn eligible(&self) -> bool {
        self.eligible
    }

    /// Cancel pending and held keys before a session is replaced.
    pub fn cancel_keys(&mut self) {
        if let Some(keys) = &mut self.keys {
            if keys.progress.phase.active() {
                keys.progress.phase = KeyPhase::Cancelled;
            }
        }
    }

    fn cancel_script_keys(&mut self) {
        if self
            .keys
            .as_ref()
            .is_some_and(|keys| keys.script_identity.is_some())
        {
            self.cancel_keys();
        }
    }

    /// The key awaiting this native frame, without changing its state.
    pub fn pending_key(&self) -> Option<(u64, GestureKey)> {
        let keys = self.keys.as_ref()?;
        keys.progress
            .phase
            .active()
            .then_some((keys.progress.ticket, keys.progress.key))
    }

    /// Advance one admitted gesture on the normal input consumer.
    pub fn consume_key(&mut self, now: Instant, allowed: bool) -> Option<(u64, GestureKey)> {
        let keys = self.keys.as_mut()?;
        if !keys.progress.phase.active() {
            return None;
        }
        if !allowed
            || keys
                .script_identity
                .is_some_and(|id| !self.eligible || id != self.identity)
        {
            keys.progress.phase = KeyPhase::Cancelled;
            return None;
        }
        let start = *keys.started.get_or_insert(now);
        keys.progress.held_ms = now
            .saturating_duration_since(start)
            .as_millis()
            .min(u128::from(u32::MAX)) as u32;
        if now.saturating_duration_since(start) >= keys.gesture.duration {
            keys.progress.phase = KeyPhase::Released;
            return None;
        }
        keys.progress.phase = KeyPhase::Held;
        keys.progress.applied_frames = keys.progress.applied_frames.saturating_add(1);
        Some((keys.progress.ticket, keys.progress.key))
    }
}

/// Slot-local authority for native script input.
pub struct NativeInputAuthority {
    inner: Mutex<NativeInputPermit>,
}

impl NativeInputAuthority {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(NativeInputPermit {
                identity: 0,
                eligible: false,
                keys: None,
            }),
        })
    }

    /// Consume-side guard. Hold only across apply + latch.
    pub fn lock(&self) -> MutexGuard<'_, NativeInputPermit> {
        self.inner.lock().expect("native input authority")
    }

    /// After a live instance exists: new identity, eligible.
    pub fn publish_live(&self) -> u64 {
        let mut g = self.lock();
        g.cancel_script_keys();
        g.identity = g.identity.wrapping_add(1);
        g.eligible = true;
        g.identity
    }

    /// Pause / Stop / replacement / watchdog / logout / error.
    /// Bumps identity so Resume cannot replay queued events.
    pub fn revoke(&self) {
        let mut g = self.lock();
        g.cancel_script_keys();
        g.eligible = false;
        g.identity = g.identity.wrapping_add(1);
    }

    /// Resume after Pause: current (already bumped) identity becomes eligible.
    pub fn resume(&self) {
        let mut g = self.lock();
        g.eligible = true;
    }

    /// Observe / watchdog sync. Going live does not bump; leaving live
    /// revokes once so queued events cannot replay.
    pub fn sync_live(&self, live: bool) {
        let mut g = self.lock();
        if live {
            g.eligible = true;
        } else if g.eligible {
            g.cancel_script_keys();
            g.eligible = false;
            g.identity = g.identity.wrapping_add(1);
        }
    }

    /// Start one scenario-owned gesture. Dropping its lease cancels only that gesture.
    pub fn key_gesture(self: &Arc<Self>, gesture: KeyGesture) -> Result<KeyLease, KeyInputError> {
        let ticket = self.enqueue_key(gesture, None)?;
        Ok(KeyLease {
            authority: Arc::clone(self),
            ticket,
        })
    }

    /// Admit a script request stamped at production, never at consumption.
    pub fn script_key_gesture(
        &self,
        identity: u64,
        gesture: KeyGesture,
    ) -> Result<u64, KeyInputError> {
        self.enqueue_key(gesture, Some(identity))
    }

    fn enqueue_key(
        &self,
        gesture: KeyGesture,
        script_identity: Option<u64>,
    ) -> Result<u64, KeyInputError> {
        let mut permit = self.lock();
        if script_identity.is_some_and(|id| !permit.eligible || id != permit.identity) {
            return Err(KeyInputError::StaleOwner);
        }
        if permit.pending_key().is_some() {
            return Err(KeyInputError::Busy);
        }
        let ticket = permit
            .keys
            .as_ref()
            .map_or(1, |keys| keys.progress.ticket.wrapping_add(1));
        let state = KeyState {
            gesture,
            script_identity,
            started: None,
            progress: KeyProgress {
                ticket,
                key: gesture.key,
                duration_ms: gesture.duration.as_millis() as u32,
                phase: KeyPhase::Pending,
                applied_frames: 0,
                held_ms: 0,
            },
        };
        match &mut permit.keys {
            Some(keys) => **keys = state,
            None => permit.keys = Some(Box::new(state)),
        }
        Ok(ticket)
    }

    /// Read bounded native-input evidence without allocating.
    pub fn key_progress(&self, ticket: u64) -> Option<KeyProgress> {
        self.lock()
            .keys
            .as_ref()
            .filter(|keys| keys.progress.ticket == ticket)
            .map(|keys| keys.progress)
    }

    fn cancel_key(&self, ticket: u64) {
        let mut permit = self.lock();
        if permit
            .keys
            .as_ref()
            .is_some_and(|keys| keys.progress.ticket == ticket)
        {
            permit.cancel_keys();
        }
    }
}

/// An admitted camera key. Translation uses the client's ordinary key map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GestureKey {
    /// Rotate the orbit camera left.
    ArrowLeft,
    /// Rotate the orbit camera right.
    ArrowRight,
    /// Raise the orbit camera.
    ArrowUp,
    /// Lower the orbit camera.
    ArrowDown,
}

impl GestureKey {
    /// Parse the DOM name without admitting text, modifiers, or arbitrary key codes.
    pub fn parse(name: &str) -> Result<Self, KeyInputError> {
        match name {
            "ArrowLeft" => Ok(Self::ArrowLeft),
            "ArrowRight" => Ok(Self::ArrowRight),
            "ArrowUp" => Ok(Self::ArrowUp),
            "ArrowDown" => Ok(Self::ArrowDown),
            _ => Err(KeyInputError::InvalidKey),
        }
    }

    /// DOM key name.
    pub fn name(self) -> &'static str {
        match self {
            Self::ArrowLeft => "ArrowLeft",
            Self::ArrowRight => "ArrowRight",
            Self::ArrowUp => "ArrowUp",
            Self::ArrowDown => "ArrowDown",
        }
    }

    /// The same remapped key value that normal canvas input uses.
    pub fn shell_char(self) -> i32 {
        client::client::lookup(self.name())
            .expect("admitted client arrow key")
            .ch
    }
}

/// One bounded hold, measured from its first native consumer frame.
#[derive(Debug, Clone, Copy)]
pub struct KeyGesture {
    key: GestureKey,
    duration: Duration,
}

impl KeyGesture {
    /// Host admission ceiling, not a game timing constant.
    pub const MAX_DURATION_MS: u32 = 10_000;

    /// Admit a nonzero hold of at most ten seconds.
    pub fn new(key: GestureKey, duration_ms: u32) -> Result<Self, KeyInputError> {
        if duration_ms == 0 || duration_ms > Self::MAX_DURATION_MS {
            return Err(KeyInputError::InvalidDuration);
        }
        Ok(Self {
            key,
            duration: Duration::from_millis(u64::from(duration_ms)),
        })
    }
}

/// A normal input admission or ownership refusal.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyInputError {
    /// Only camera arrows are admitted.
    #[error("key gesture requires an arrow key")]
    InvalidKey,
    /// Zero and unbounded holds are refused.
    #[error("key gesture duration must be 1..=10000 milliseconds")]
    InvalidDuration,
    /// A previous gesture still owns the input.
    #[error("another key gesture is active")]
    Busy,
    /// The producing script no longer owns native input.
    #[error("key gesture owner is stale or inactive")]
    StaleOwner,
}

/// Native consumer state, distinct from an accepted request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum KeyPhase {
    /// Admitted, not yet applied to a native frame.
    Pending,
    /// Applied and owned by the native consumer.
    Held,
    /// The requested native duration ended and its key was released.
    Released,
    /// Ownership or session ended before normal completion.
    Cancelled,
}

impl KeyPhase {
    fn active(self) -> bool {
        matches!(self, Self::Pending | Self::Held)
    }
}

/// Scalar evidence retained for one gesture only.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct KeyProgress {
    /// Slot-local gesture identity.
    pub ticket: u64,
    /// Admitted camera key.
    pub key: GestureKey,
    /// Requested native hold duration.
    pub duration_ms: u32,
    /// Latest consumer state.
    pub phase: KeyPhase,
    /// Number of native frames that applied this hold.
    pub applied_frames: u32,
    /// Elapsed native hold duration, including the release frame.
    pub held_ms: u32,
}

#[derive(Debug)]
struct KeyState {
    gesture: KeyGesture,
    script_identity: Option<u64>,
    started: Option<Instant>,
    progress: KeyProgress,
}

/// Scenario ownership. Cancellation never clears a later owner's gesture.
pub struct KeyLease {
    authority: Arc<NativeInputAuthority>,
    ticket: u64,
}

impl KeyLease {
    /// Read the native consumer's evidence.
    pub fn progress(&self) -> Option<KeyProgress> {
        self.authority.key_progress(self.ticket)
    }
}

impl Drop for KeyLease {
    fn drop(&mut self) {
        self.authority.cancel_key(self.ticket);
    }
}

#[cfg(test)]
mod tests {
    use super::NativeInputAuthority;

    #[test]
    fn publish_live_enables_a_fresh_identity() {
        let a = NativeInputAuthority::new();
        assert!(!a.lock().eligible());
        assert_eq!(a.lock().identity(), 0);
        let id = a.publish_live();
        assert!(id > 0);
        let p = a.lock();
        assert!(p.eligible());
        assert_eq!(p.identity(), id);
    }

    #[test]
    fn revoke_disables_and_bumps_so_resume_cannot_replay() {
        let a = NativeInputAuthority::new();
        let old = a.publish_live();
        a.revoke();
        {
            let p = a.lock();
            assert!(!p.eligible());
            assert_ne!(p.identity(), old);
        }
        a.resume();
        let p = a.lock();
        assert!(p.eligible());
        assert_ne!(p.identity(), old);
    }

    #[test]
    fn sync_live_false_bumps_once() {
        let a = NativeInputAuthority::new();
        let id = a.publish_live();
        a.sync_live(false);
        let after = a.lock().identity();
        assert_ne!(after, id);
        a.sync_live(false);
        assert_eq!(a.lock().identity(), after);
        a.sync_live(true);
        assert!(a.lock().eligible());
        assert_eq!(a.lock().identity(), after);
    }

    #[test]
    fn applet_constants_bind_client_source() {
        assert_eq!(super::APPLET_W, client::client::APPLET_W);
        assert_eq!(super::APPLET_H, client::client::APPLET_H);
        assert_eq!(super::APPLET_W, 765);
        assert_eq!(super::APPLET_H, 503);
    }
}
