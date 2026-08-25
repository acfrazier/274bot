//! Focused-slot speaker gate: at most one cpal speaker, owned by the
//! focused slot while its Music/SFX toggle is on. `lowmem` (toggle off)
//! never opens one. The gate is generic over the speaker handle so the
//! open/close decision is unit-testable without a real audio device; the
//! panel instantiates it with `client::sound::output::AudioOut`.

use std::collections::HashMap;
use std::sync::Mutex;

/// What a frame's reconcile did to the speaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChange {
    None,
    Opened,
    Closed,
}

/// The policy: the speaker belongs to the focused slot when its Music/SFX
/// toggle is on (highmem). A `lowmem` slot (toggle off) never opens cpal.
fn should_own(name: &str, focused: Option<&str>, music_on: bool) -> bool {
    music_on && focused == Some(name)
}

/// Shared per-slot audio policy. Slot threads reconcile on their frame
/// loop via [`AudioGate::frame`] (the open closure runs on the slot that
/// owns the `Client`, feeding the speaker from its shared midi/waves/fade
/// state); the panel flips the Music/SFX toggle via [`AudioGate::set_music`]
/// and releases a stopped slot's speaker via [`AudioGate::release`].
pub struct AudioGate<S> {
    inner: Mutex<GateInner<S>>,
}

struct GateInner<S> {
    /// The slot that owns the open speaker.
    owner: Option<String>,
    /// Music/SFX per slot (mirrors the vault profile's lowmem).
    music_on: HashMap<String, bool>,
    /// The open speaker; dropping it stops the device callback.
    speaker: Option<S>,
}

impl<S> Default for AudioGate<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> AudioGate<S> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(GateInner {
                owner: None,
                music_on: HashMap::new(),
                speaker: None,
            }),
        }
    }

    /// The slot that currently owns the speaker, if any.
    pub fn owner(&self) -> Option<String> {
        self.inner.lock().unwrap().owner.clone()
    }

    /// Set/clear `name`'s Music/SFX toggle (the panel's checkbox, mirrored
    /// from the vault profile's lowmem).
    pub fn set_music(&self, name: &str, on: bool) {
        self.inner
            .lock()
            .unwrap()
            .music_on
            .insert(name.to_string(), on);
    }

    /// Whether `name`'s Music/SFX toggle is on.
    pub fn music_on(&self, name: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .music_on
            .get(name)
            .copied()
            .unwrap_or(false)
    }

    /// Tear down the speaker if `name` owns it (a slot stopped/removed).
    pub fn release(&self, name: &str) -> AudioChange {
        let mut g = self.inner.lock().unwrap();
        if g.owner.as_deref() == Some(name) {
            g.speaker = None;
            g.owner = None;
            return AudioChange::Closed;
        }
        AudioChange::None
    }

    /// Reconcile this slot's speaker with the policy. `open` runs once on
    /// the slot thread that owns the `Client` when this slot is the one
    /// that should hold the speaker; a failed open stays silent (spec:
    /// audio device failure is not fatal).
    pub fn frame(
        &self,
        name: &str,
        focused: Option<&str>,
        open: impl FnOnce() -> Option<S>,
    ) -> AudioChange {
        let mut g = self.inner.lock().unwrap();
        let music = g.music_on.get(name).copied().unwrap_or(false);
        if should_own(name, focused, music) {
            if g.owner.as_deref() != Some(name) {
                // Retarget: any open speaker belonged to a slot that lost
                // the focus (or a failed open); replace it with ours.
                g.speaker = None;
                g.owner = None;
                if let Some(s) = open() {
                    g.speaker = Some(s);
                    g.owner = Some(name.to_string());
                    return AudioChange::Opened;
                }
            }
            return AudioChange::None;
        }
        if g.owner.as_deref() == Some(name) {
            g.speaker = None;
            g.owner = None;
            return AudioChange::Closed;
        }
        AudioChange::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fake speaker: the device seam. Tests count opens via the open
    /// closure instead of touching a real cpal device.
    struct Fake;

    #[test]
    fn lowmem_never_opens_the_speaker() {
        let gate = AudioGate::<Fake>::new();
        let opened = AtomicU32::new(0);
        let change = gate.frame("alice", Some("alice"), || {
            opened.fetch_add(1, Ordering::Relaxed);
            Some(Fake)
        });
        assert_eq!(change, AudioChange::None);
        assert_eq!(
            opened.load(Ordering::Relaxed),
            0,
            "lowmem must not open cpal"
        );
        assert_eq!(gate.owner(), None);
    }

    #[test]
    fn unfocused_music_slot_never_opens() {
        let gate = AudioGate::<Fake>::new();
        gate.set_music("bob", true);
        assert!(gate.music_on("bob"));
        let change = gate.frame("bob", Some("alice"), || Some(Fake));
        assert_eq!(change, AudioChange::None);
        assert_eq!(gate.owner(), None);
    }

    #[test]
    fn focused_highmem_opens_and_defocus_tears_down() {
        let gate = AudioGate::<Fake>::new();
        gate.set_music("alice", true);
        assert_eq!(
            gate.frame("alice", Some("alice"), || Some(Fake)),
            AudioChange::Opened
        );
        assert_eq!(gate.owner(), Some("alice".to_string()));
        // The owner's own frame stays open (no re-open).
        assert_eq!(
            gate.frame("alice", Some("alice"), || Some(Fake)),
            AudioChange::None
        );
        assert_eq!(gate.owner(), Some("alice".to_string()));
        // Focus moves away: the speaker tears down.
        assert_eq!(
            gate.frame("alice", None, || Some(Fake)),
            AudioChange::Closed
        );
        assert_eq!(gate.owner(), None);
    }

    #[test]
    fn focus_retarget_replaces_the_speaker() {
        let gate = AudioGate::<Fake>::new();
        gate.set_music("alice", true);
        gate.set_music("bob", true);
        assert_eq!(
            gate.frame("alice", Some("alice"), || Some(Fake)),
            AudioChange::Opened
        );
        // Focus flips to bob while alice's thread is still parked: bob's
        // frame retargets immediately, dropping alice's speaker.
        assert_eq!(
            gate.frame("bob", Some("bob"), || Some(Fake)),
            AudioChange::Opened
        );
        assert_eq!(gate.owner(), Some("bob".to_string()));
        // The defocused alice's next frame must not tear down bob's speaker.
        assert_eq!(
            gate.frame("alice", Some("bob"), || Some(Fake)),
            AudioChange::None
        );
        assert_eq!(gate.owner(), Some("bob".to_string()));
    }

    #[test]
    fn toggle_off_tears_down_live() {
        let gate = AudioGate::<Fake>::new();
        gate.set_music("alice", true);
        gate.frame("alice", Some("alice"), || Some(Fake));
        // Music/SFX toggle off (lowmem): the focused slot's speaker closes.
        gate.set_music("alice", false);
        assert_eq!(
            gate.frame("alice", Some("alice"), || Some(Fake)),
            AudioChange::Closed
        );
        assert_eq!(gate.owner(), None);
    }

    #[test]
    fn release_closes_a_stopped_slot() {
        let gate = AudioGate::<Fake>::new();
        gate.set_music("alice", true);
        gate.frame("alice", Some("alice"), || Some(Fake));
        assert_eq!(gate.release("alice"), AudioChange::Closed);
        assert_eq!(gate.owner(), None);
        assert_eq!(gate.release("alice"), AudioChange::None);
        // A non-owner release never touches the open speaker.
        gate.set_music("bob", true);
        gate.frame("bob", Some("bob"), || Some(Fake));
        assert_eq!(gate.release("alice"), AudioChange::None);
        assert_eq!(gate.owner(), Some("bob".to_string()));
    }

    #[test]
    fn failed_open_stays_silent_and_can_retry() {
        let gate = AudioGate::<Fake>::new();
        gate.set_music("alice", true);
        assert_eq!(
            gate.frame("alice", Some("alice"), || None),
            AudioChange::None
        );
        assert_eq!(gate.owner(), None);
        // A later frame can still open (device recovered / next focus).
        assert_eq!(
            gate.frame("alice", Some("alice"), || Some(Fake)),
            AudioChange::Opened
        );
        assert_eq!(gate.owner(), Some("alice".to_string()));
    }

    #[test]
    fn should_own_policy_table() {
        assert!(should_own("a", Some("a"), true));
        assert!(!should_own("a", Some("b"), true));
        assert!(!should_own("a", None, true));
        assert!(!should_own("a", Some("a"), false));
        assert!(!should_own("a", Some("b"), false));
    }
}
