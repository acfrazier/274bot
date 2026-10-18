//! The profile values the panel and the TUI edit and show, owned by the
//! core (T3.3): a profile form's staged draft ([`ProfileDraft`], its
//! settings in [`SettingsDraft`]), a read view of a saved profile
//! ([`ProfileView`]), the slot spawn profile a surface adjusts
//! ([`crate::surface::SpawnProfile`]) and the guardian vocabulary
//! ([`GuardianPolicy`], [`GUARDIAN_EVENTS`]). The vault types stay behind
//! these accessors; front ends never name them.

use vault::{Profile, ProfileSettings, RandomEventPolicy, Secret};

use crate::command::RasterMode;

/// One hostile random's response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GuardianPolicy {
    /// Walk away (the only supported response; saved Fight and Ignore act
    /// as Flee).
    #[default]
    Flee,
    /// Fight it (not available yet).
    Fight,
    /// Ignore it (not available yet).
    Ignore,
}

impl GuardianPolicy {
    pub(crate) fn from_vault(policy: RandomEventPolicy) -> Self {
        match policy {
            RandomEventPolicy::Flee => Self::Flee,
            RandomEventPolicy::Fight => Self::Fight,
            RandomEventPolicy::Ignore => Self::Ignore,
        }
    }

    pub(crate) fn to_vault(self) -> RandomEventPolicy {
        match self {
            Self::Flee => RandomEventPolicy::Flee,
            Self::Fight => RandomEventPolicy::Fight,
            Self::Ignore => RandomEventPolicy::Ignore,
        }
    }
}

/// The hostile randoms with a per-event policy: (stable key, label), in
/// display order.
pub const GUARDIAN_EVENTS: [(&str, &str); 8] = vault::RANDOM_EVENT_POLICY_EVENTS;

impl RasterMode {
    pub(crate) fn from_vault(mode: vault::RasterMode) -> Self {
        match mode {
            vault::RasterMode::Off => Self::Off,
            vault::RasterMode::Gpu => Self::Gpu,
            vault::RasterMode::Cpu => Self::Cpu,
        }
    }

    pub(crate) fn to_vault(self) -> vault::RasterMode {
        match self {
            Self::Off => vault::RasterMode::Off,
            Self::Gpu => vault::RasterMode::Gpu,
            Self::Cpu => vault::RasterMode::Cpu,
        }
    }
}

/// A staged copy of one profile's settings. The `_mut` accessors are the
/// edit buffers a toolkit widget writes into; nothing is saved until a
/// command commits the draft.
#[derive(Debug, PartialEq, Default)]
pub struct SettingsDraft {
    settings: ProfileSettings,
}

impl SettingsDraft {
    pub(crate) fn from_settings(settings: ProfileSettings) -> Self {
        Self { settings }
    }

    pub(crate) fn settings(&self) -> &ProfileSettings {
        &self.settings
    }

    /// Low-memory handshake for the next login.
    pub fn lowmem(&self) -> bool {
        self.settings.lowmem
    }

    /// Edit buffer of [`Self::lowmem`].
    pub fn lowmem_mut(&mut self) -> &mut bool {
        &mut self.settings.lowmem
    }

    /// Auto-login.
    pub fn auto_login(&self) -> bool {
        self.settings.auto_login
    }

    /// Edit buffer of [`Self::auto_login`].
    pub fn auto_login_mut(&mut self) -> &mut bool {
        &mut self.settings.auto_login
    }

    /// The public world, or `None` for automatic selection.
    pub fn world(&self) -> Option<u16> {
        self.settings.world
    }

    /// Edit buffer of [`Self::world`].
    pub fn world_mut(&mut self) -> &mut Option<u16> {
        &mut self.settings.world
    }

    /// Incoming random events.
    pub fn random_events(&self) -> bool {
        self.settings.random_events
    }

    /// Edit buffer of [`Self::random_events`].
    pub fn random_events_mut(&mut self) -> &mut bool {
        &mut self.settings.random_events
    }

    /// The lamp reward skill.
    pub fn lamp_skill(&self) -> &str {
        &self.settings.lamp_skill
    }

    /// Edit buffer of [`Self::lamp_skill`].
    pub fn lamp_skill_mut(&mut self) -> &mut String {
        &mut self.settings.lamp_skill
    }

    /// Lamp auto-use.
    pub fn lamp_auto(&self) -> bool {
        self.settings.lamp_auto
    }

    /// Edit buffer of [`Self::lamp_auto`].
    pub fn lamp_auto_mut(&mut self) -> &mut bool {
        &mut self.settings.lamp_auto
    }

    /// Clue 3554's Duel Arena partner.
    pub fn clue_duel_partner(&self) -> &str {
        &self.settings.clue_duel_partner
    }

    /// Edit buffer of [`Self::clue_duel_partner`].
    pub fn clue_duel_partner_mut(&mut self) -> &mut String {
        &mut self.settings.clue_duel_partner
    }

    /// The response to the hostile random `event` (a [`GUARDIAN_EVENTS`] key).
    pub fn random_event_policy(&self, event: &str) -> GuardianPolicy {
        GuardianPolicy::from_vault(self.settings.random_event_policy(event))
    }

    /// Stage the response to `event`.
    pub fn set_random_event_policy(&mut self, event: &str, policy: GuardianPolicy) {
        self.settings
            .set_random_event_policy(event, policy.to_vault());
    }

    /// The game image raster mode.
    pub fn raster(&self) -> RasterMode {
        RasterMode::from_vault(self.settings.raster)
    }

    /// Cached TutSkip result (`None` until read).
    pub fn tutorial_skipped(&self) -> Option<bool> {
        self.settings.tutorial_skipped
    }
}

/// A profile form's draft: username, password and settings.
#[derive(Debug, PartialEq)]
pub struct ProfileDraft {
    username: String,
    password: Secret,
    settings: SettingsDraft,
}

impl Default for ProfileDraft {
    fn default() -> Self {
        Self {
            username: String::new(),
            // Room for a normal password, so typing does not reallocate and
            // leave an unzeroed copy behind.
            password: Secret::with_capacity(256),
            settings: SettingsDraft::default(),
        }
    }
}

impl ProfileDraft {
    pub(crate) fn from_profile(profile: &Profile) -> Self {
        let mut password = Secret::with_capacity(256.max(profile.password.len()));
        password.push_str(&profile.password);
        Self {
            username: profile.username.clone(),
            password,
            settings: SettingsDraft::from_settings(profile.settings.clone()),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_parts(username: &str, password: &str, settings: ProfileSettings) -> Self {
        Self {
            username: username.to_string(),
            password: Secret::from(password),
            settings: SettingsDraft::from_settings(settings),
        }
    }

    pub(crate) fn password_secret(&self) -> &Secret {
        &self.password
    }

    /// The username field.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Edit buffer of [`Self::username`].
    pub fn username_mut(&mut self) -> &mut String {
        &mut self.username
    }

    /// Fixture seam: the password field's text.
    #[cfg(any(test, feature = "test-support"))]
    pub fn password_for_test(&self) -> &str {
        self.password.as_str()
    }

    /// Whether the password field is empty (the password is never shown).
    pub fn password_is_empty(&self) -> bool {
        self.password.is_empty()
    }

    /// Edit buffer of the password field. The text is zeroed when the draft
    /// is dropped or cleared.
    pub fn password_mut(&mut self) -> &mut String {
        &mut self.password
    }

    /// Empty the username and password fields.
    pub fn clear_credentials(&mut self) {
        self.username.clear();
        self.password.clear();
    }

    /// The draft's settings.
    pub fn settings(&self) -> &SettingsDraft {
        &self.settings
    }

    /// The draft's settings, for editing.
    pub fn settings_mut(&mut self) -> &mut SettingsDraft {
        &mut self.settings
    }
}

/// A read view of one saved profile.
#[derive(Debug, Clone, Copy)]
pub struct ProfileView<'a> {
    profile: &'a Profile,
}

impl<'a> ProfileView<'a> {
    pub(crate) fn new(profile: &'a Profile) -> Self {
        Self { profile }
    }

    /// The username (the vault key).
    pub fn username(&self) -> &'a str {
        &self.profile.username
    }

    /// The account uid.
    pub fn uid(&self) -> i32 {
        self.profile.uid
    }

    /// Low-memory handshake for the next login.
    pub fn lowmem(&self) -> bool {
        self.profile.settings.lowmem
    }

    /// Auto-login.
    pub fn auto_login(&self) -> bool {
        self.profile.settings.auto_login
    }

    /// The game image raster mode.
    pub fn raster(&self) -> RasterMode {
        RasterMode::from_vault(self.profile.settings.raster)
    }

    /// Incoming random events.
    pub fn random_events(&self) -> bool {
        self.profile.settings.random_events
    }

    /// The response to the hostile random `event`.
    pub fn random_event_policy(&self, event: &str) -> GuardianPolicy {
        GuardianPolicy::from_vault(self.profile.settings.random_event_policy(event))
    }

    /// Cached TutSkip result (`None` until read).
    pub fn tutorial_skipped(&self) -> Option<bool> {
        self.profile.settings.tutorial_skipped
    }

    /// A staged copy of this profile's settings.
    pub fn settings_draft(&self) -> SettingsDraft {
        SettingsDraft::from_settings(self.profile.settings.clone())
    }

    /// A staged copy of the whole profile.
    pub fn draft(&self) -> ProfileDraft {
        ProfileDraft::from_profile(self.profile)
    }
}
