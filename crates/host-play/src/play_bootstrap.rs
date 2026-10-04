use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use api::snapshot::WorldTile;
use client::client::{Client, ClientConfig};
use client::config::{Cache, IfType, IfTypeMut};
use client::io::JagFile;
use host::login_queue::LoginQueue;
use nav::world::NavWorld;
use parking_lot::Mutex as QueueMutex;
use rand_core::{OsRng, RngCore};
use vault::{Profile, Vault, VaultError};

use super::{
    catalog_core, paired_core, progress, scatter, FrameBuf, Play, ServerProfile, SlotInput,
};

/// Mint a login password accepted by the game protocol.
pub fn mint_game_password() -> String {
    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut password = String::with_capacity(20);
    let mut random = [0u8; 32];
    while password.len() < 20 {
        OsRng.fill_bytes(&mut random);
        for byte in random {
            if byte >= 252 {
                continue;
            }
            password.push(ALPHABET[(byte % 36) as usize] as char);
            if password.len() == 20 {
                break;
            }
        }
    }
    password
}

/// Mint a high-entropy passphrase for an ephemeral vault.
pub fn mint_vault_passphrase() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    let mut passphrase = String::with_capacity(48);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(passphrase, "{byte:02x}").expect("writing to String");
    }
    passphrase
}

/// Environment override for the minted-name prefix (default `live`), so
/// concurrent harness owners can tell their accounts apart in engine logs.
pub(crate) const LIVE_NAME_PREFIX_ENV: &str = "BOT_LIVE_NAME_PREFIX";

const DEFAULT_LIVE_NAME_PREFIX: &str = "live";

/// Check a minted-name prefix's length: 1–4 bytes. Capping it at the
/// default's length keeps the random token at least as long as the
/// default's, so a long prefix can't shrink the token until runs collide
/// and silently reuse an old save. Characters aren't checked here: the
/// engine rejects a name it can't use at login, which fails the run loudly.
pub(crate) fn parse_live_name_prefix(raw: Option<&str>) -> Result<&str, String> {
    let Some(prefix) = raw else {
        return Ok(DEFAULT_LIVE_NAME_PREFIX);
    };
    if (1..=DEFAULT_LIVE_NAME_PREFIX.len()).contains(&prefix.len()) {
        Ok(prefix)
    } else {
        Err(format!(
            "{LIVE_NAME_PREFIX_ENV} must be 1-4 characters, got {prefix:?}"
        ))
    }
}

/// Mint `n` per-run usernames for a live boot (`<prefix><token>_<i>`, prefix
/// `live` unless `BOT_LIVE_NAME_PREFIX` is set). The engine auto-registers
/// unknown names, so a minted name logs into a fresh save instead of the
/// shared `test` account. The engine enforces the classic 12-character
/// username limit. A randomly seeded counter keeps consecutive runs
/// distinct; its low base-36 digits fill the remaining budget without
/// truncating away the changing part of the counter. Player saves
/// accumulate under the engine's `player/` dir — wipe it to reset.
///
/// Panics if the override is set but invalid: it's a harness setting, and
/// silently minting `live` names would defeat it.
pub fn mint_live_names(n: usize) -> Vec<String> {
    static PREFIX: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let raw = std::env::var(LIVE_NAME_PREFIX_ENV).ok();
        parse_live_name_prefix(raw.as_deref())
            .unwrap_or_else(|e| panic!("{e}"))
            .to_owned()
    });
    mint_live_names_with(&PREFIX, n)
}

pub(crate) fn mint_live_names_with(prefix: &str, n: usize) -> Vec<String> {
    static NEXT: std::sync::OnceLock<std::sync::atomic::AtomicU64> = std::sync::OnceLock::new();
    let mut nonce = NEXT
        .get_or_init(|| std::sync::atomic::AtomicU64::new(OsRng.next_u64()))
        .fetch_add(1, Ordering::Relaxed);
    let slot_digits = n.saturating_sub(1).max(1).to_string().len();
    let max_token = 12usize
        .saturating_sub(prefix.len() + 1 + slot_digits)
        .max(1);
    let mut token = vec![b'0'; max_token];
    for digit in token.iter_mut().rev() {
        *digit = b"0123456789abcdefghijklmnopqrstuvwxyz"[(nonce % 36) as usize];
        nonce /= 36;
    }
    let token = String::from_utf8(token).expect("base-36 token is ASCII");
    (0..n).map(|i| format!("{prefix}{token}_{i}")).collect()
}

/// Profile login password for a freshly minted account.
pub fn profile_password(_username: &str) -> String {
    mint_game_password()
}

/// Ephemeral live-vault passphrase, fresh for every run.
pub fn live_vault_passphrase() -> String {
    mint_vault_passphrase()
}

/// `(username, password)` pairs for a live boot from minted names.
pub fn mint_live_entries(names: &[String]) -> Vec<(String, String)> {
    names
        .iter()
        .map(|username| (username.clone(), mint_game_password()))
        .collect()
}

/// Whether `host` is loopback (local engine RSA is safe).
pub fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.trim()
            .trim_end_matches('.')
            .to_ascii_lowercase()
            .as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

/// Refuse a non-loopback play host while local (well-known Java) RSA is active.
/// Refuse plaintext off-loopback unless the selected profile opted in.
pub fn validate_play_host(host: &str, allow_plaintext_offhost: bool) -> Result<(), &'static str> {
    if !allow_plaintext_offhost && !is_loopback_host(host) {
        Err("host-play: non-loopback tcp requires allow_plaintext_offhost")
    } else {
        Ok(())
    }
}

/// Connection settings shared by every spawned slot.
#[derive(Clone)]
pub struct PlayOptions {
    pub host: String,
    pub transport: client::Transport,
    pub port: u16,
    pub cache_dir: String,
    pub lowmem: bool,
    /// After scene 2, queue rs2b0t `mainlandAccount` tele+setvar (no relog).
    pub mainland: bool,
}

/// Production launch input. Connection fields come only from the immutable
/// profile; `PlayOptions` remains the legacy 274 caller interface.
#[derive(Clone)]
pub struct ProfilePlayOptions {
    pub profile: Arc<ServerProfile>,
    pub mainland: bool,
}

/// One validated asset decode for a process profile. Preparing clients clones
/// only the shared Arcs; mutable interface overlays remain per client.
pub struct SharedClientTemplate {
    profile: Arc<ServerProfile>,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    cache: Arc<Cache>,
    ifaces: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    world: Option<Arc<NavWorld>>,
    scatter: std::sync::OnceLock<Vec<WorldTile>>,
}

/// One-use ordering ticket for Play construction. Resource identities are
/// checked when the template is loaded; this type only proves handoff order.
/// Private fields prevent callers from constructing Play without the ticket.
pub struct ValidatedTemplate {
    template: Arc<SharedClientTemplate>,
}

impl ValidatedTemplate {
    /// The exact template whose profile produced this validation proof.
    pub fn template(&self) -> &Arc<SharedClientTemplate> {
        &self.template
    }
}

impl SharedClientTemplate {
    pub fn load(profile: Arc<ServerProfile>) -> Result<Arc<Self>, String> {
        Self::load_with_progress(profile, &progress::ProfileProgressObserver::default())
    }

    pub fn load_with_progress(
        profile: Arc<ServerProfile>,
        observer: &progress::ProfileProgressObserver,
    ) -> Result<Arc<Self>, String> {
        profile.validate_resources_with_progress(observer)?;
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::LoadingGameData,
            0,
            4,
        ));
        let (cache, ifaces, ifaces_mut) =
            load_template_checked(profile.client().cache_dir(), observer)?;
        let game_data = profile.game_data();
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::LoadingGameData,
            4,
            4,
        ));
        let world = profile.world();
        Ok(Arc::new(Self {
            profile,
            game_data,
            cache: Arc::new(cache),
            ifaces: Arc::new(ifaces),
            ifaces_mut: Arc::new(ifaces_mut),
            world,
            scatter: std::sync::OnceLock::new(),
        }))
    }

    pub fn profile(&self) -> &Arc<ServerProfile> {
        &self.profile
    }
    pub fn game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.game_data.clone()
    }
    pub fn world(&self) -> Option<Arc<NavWorld>> {
        self.world.clone()
    }

    /// Return the consuming handoff required by [`run_prepared_template`].
    /// Navigation is the already-loaded world; a post-load disk edit does not
    /// replace it. Cache identity was checked at load; this does not re-hash.
    pub fn validate_for_play(self: &Arc<Self>) -> Result<ValidatedTemplate, String> {
        self.validate_for_play_with_progress(&progress::ProfileProgressObserver::default())
    }

    pub fn validate_for_play_with_progress(
        self: &Arc<Self>,
        observer: &progress::ProfileProgressObserver,
    ) -> Result<ValidatedTemplate, String> {
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::FinalChecks,
            1,
            1,
        ));
        Ok(ValidatedTemplate {
            template: Arc::clone(self),
        })
    }

    /// Stable account scatter over this template's selected navigation world.
    /// The seed vector is shared once per template, as are the world tables.
    pub fn scatter_tile_for(&self, uid: i32) -> WorldTile {
        let tiles = self
            .scatter
            .get_or_init(|| scatter::shuffled_for_world(self.world.as_deref()));
        tiles[(uid.unsigned_abs() as usize) % tiles.len()]
    }

    /// This constructor is usable for protocol qualification before bot action
    /// support is released. It does not start a host loop or any bot policy.
    pub fn prepare_client(&self, uid: i32, lowmem: bool) -> Result<Client, String> {
        let mut client = host::prepare_client_with_profile(
            Arc::clone(self.profile.client()),
            uid,
            self.profile.map_members(),
            lowmem,
            Arc::clone(&self.cache),
            Arc::clone(&self.ifaces),
            Arc::clone(&self.ifaces_mut),
        )?;
        client.set_cheat_admission(match self.profile.profile_class() {
            crate::ProfileClass::Local => client::CheatAdmission::Granted,
            crate::ProfileClass::Remote => client::CheatAdmission::Remote(self.profile.transport()),
        });
        Ok(client)
    }
}

#[derive(Clone)]
pub(super) enum PlayConnection {
    Direct(PlayOptions),
    Bound {
        template: Arc<SharedClientTemplate>,
        mainland: bool,
    },
}

impl PlayConnection {
    pub(super) fn profile(&self) -> Option<&Arc<ServerProfile>> {
        match self {
            Self::Direct(_) => None,
            Self::Bound { template, .. } => Some(template.profile()),
        }
    }

    pub(super) fn require_bot_operation(&self) -> Result<(), String> {
        self.profile()
            .map_or(Ok(()), |profile| profile.require_bot_operation())
    }

    pub(super) fn profile_class(&self) -> crate::ProfileClass {
        match self {
            Self::Direct(options) => crate::profile::profile_class(
                options.transport,
                std::iter::once(options.host.as_str()),
            ),
            Self::Bound { template, .. } => template.profile().profile_class(),
        }
    }

    pub(super) fn game_host(&self) -> &str {
        match self {
            Self::Direct(options) => options.host.as_str(),
            Self::Bound { template, .. } => template.profile().client().game_host(),
        }
    }
}

/// Nav pack path: `$NAV_PACK`, else `~/.274bot/274bot.navpack` (same rule
/// as the panel picker; host-play must not depend on panel).
pub fn default_pack_path() -> std::path::PathBuf {
    match std::env::var("NAV_PACK") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => match client::operator_home() {
            Ok(home) => std::path::PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
            Err(_) => std::path::PathBuf::from(".274bot/274bot.navpack"),
        },
    }
}

impl Play {
    /// Shared construction for the public `run*` entry points: one login
    /// FIFO, one shared cache/iface template, and the per-slot script/cheat
    /// maps. `per_frame` starts as a no-op — slots stay draw-off (headless)
    /// until a caller's own per-frame hook turns a slot's renderer on.
    pub(super) fn new(options: &PlayOptions) -> Play {
        assert_eq!(
            options.transport,
            client::Transport::Tcp,
            "direct PlayOptions supports only tcp"
        );
        let (cache, ifaces, ifaces_mut_template) = load_template(&options.cache_dir);
        let cache = Arc::new(cache);
        Self::assemble(
            PlayConnection::Direct(options.clone()),
            None,
            cache,
            Arc::new(ifaces),
            Arc::new(ifaces_mut_template),
            NavWorld::load_pack(&default_pack_path()).ok().map(Arc::new),
        )
    }

    pub(super) fn from_template(template: Arc<SharedClientTemplate>, mainland: bool) -> Play {
        Self::assemble(
            PlayConnection::Bound {
                template: Arc::clone(&template),
                mainland,
            },
            template.game_data.clone(),
            Arc::clone(&template.cache),
            Arc::clone(&template.ifaces),
            Arc::clone(&template.ifaces_mut),
            template.world.clone(),
        )
    }

    fn assemble(
        connection: PlayConnection,
        game_data: Option<Arc<api::game_data::SelectedGameData>>,
        cache: Arc<Cache>,
        ifaces: Arc<Vec<Option<Box<IfType>>>>,
        ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
        world: Option<Arc<NavWorld>>,
    ) -> Play {
        let obj_names = Arc::new(api::obj_names::ObjNames::from_objs(&cache.objs));
        let named_banks = world
            .as_deref()
            .map(|world| {
                if let Some(data) = game_data.as_deref() {
                    // A profile world may already be shared by another Play.
                    let _ = world.bind_named_bank_facts(data);
                }
                world.named_bank_facts().cloned().unwrap_or_default()
            })
            .unwrap_or_default();
        Play {
            statuses: Arc::new(Mutex::new(Vec::new())),
            auto_world: None,
            handles: HashMap::new(),
            retiring: HashMap::new(),
            connection,
            game_data,
            named_banks,
            cache,
            obj_names,
            catalog_core: catalog_core::CoreWatch::default(),
            paired_core: paired_core::PairWatch::default(),
            ifaces,
            ifaces_mut_template,
            queue: Arc::new(QueueMutex::new(LoginQueue::default())),
            per_frame: Arc::new(|_: &mut Client, _: &str, _: crate::SlotFrameInput| {}),
            spawned: HashSet::new(),
            arms: HashMap::new(),
            profiles: HashMap::new(),
            focused: None,
            scripts: Arc::new(Mutex::new(HashMap::new())),
            script_start_hold: Arc::new(Mutex::new(std::sync::Weak::new())),
            channels: super::script_channels::ChannelBroker::default(),
            cheats: Arc::new(Mutex::new(HashMap::new())),
            wires: Arc::new(Mutex::new(HashMap::new())),
            navs: Arc::new(Mutex::new(HashMap::new())),
            pause_script_on_manual_walk_abort: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            walk_globals: Arc::new(Mutex::new(super::WalkGlobals::default())),
            walk_globals_store: None,
            world,
            wakes: HashMap::new(),
        }
    }
}

/// Spawn one slot thread per profile. Each slot waits for a login-queue
/// permit, sends the handshake, then drives `mainloop` at the host cadence
/// while mirroring its state into the shared status list. Slots run with no
/// input and no frame mailbox; [`run_with_io`] adds per-slot channels.
pub fn run(options: &PlayOptions, profiles: Vec<Profile>) -> Play {
    run_with_io(options, profiles, |_| (None, None), |_, _, _| {})
}

/// Like [`run`], but each slot gets the `SlotInput`/`FrameBuf` mailbox
/// returned by `per_slot` (called synchronously, keyed by username), and
/// `per_frame` runs inside the observe hook on every 20 ms frame so callers
/// can mirror panel state (e.g. `client.set_draw`) into the slot thread.
/// The frame input carries pre-mainloop human movement intent and queued manual
/// step counts before the callback's follow pump; sending remains slot-owned.
/// The FIFO login queue and mainland hop are shared by every slot.
pub fn run_with_io<F, G>(
    options: &PlayOptions,
    profiles: Vec<Profile>,
    per_slot: F,
    per_frame: G,
) -> Play
where
    F: Fn(&str) -> (Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>),
    G: Fn(&mut Client, &str, crate::SlotFrameInput) + Send + Sync + 'static,
{
    let mut play = Play::new(options);
    play.per_frame = Arc::new(per_frame);
    for profile in profiles {
        let (slot_input, slot_mailbox) = per_slot(&profile.username);
        play.spawn_slot(profile, slot_input, slot_mailbox, None);
    }
    play
}

/// Checked production path. Callers can load the template before opening a
/// vault, then hand the same shared resources to this function after unlock.
pub fn run_with_template<F, G>(
    template: Arc<SharedClientTemplate>,
    mainland: bool,
    profiles: Vec<Profile>,
    per_slot: F,
    per_frame: G,
) -> Result<Play, String>
where
    F: Fn(&str) -> (Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>),
    G: Fn(&mut Client, &str, crate::SlotFrameInput) + Send + Sync + 'static,
{
    if !profiles.is_empty() {
        template.profile().require_bot_operation()?;
    }
    let validated = template.validate_for_play()?;
    run_prepared_template(validated, mainland, profiles, per_slot, per_frame)
}

/// Construct Play from a one-use template handoff. The public checked
/// convenience entry remains [`run_with_template`]; callers cannot construct
/// this function's ticket except through [`SharedClientTemplate::validate_for_play`].
pub fn run_prepared_template<F, G>(
    validated: ValidatedTemplate,
    mainland: bool,
    profiles: Vec<Profile>,
    per_slot: F,
    per_frame: G,
) -> Result<Play, String>
where
    F: Fn(&str) -> (Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>),
    G: Fn(&mut Client, &str, crate::SlotFrameInput) + Send + Sync + 'static,
{
    let template = validated.template;
    if !profiles.is_empty() {
        template.profile().require_bot_operation()?;
    }
    let mut play = Play::from_template(template, mainland);
    play.per_frame = Arc::new(per_frame);
    for profile in profiles {
        let (input, mailbox) = per_slot(&profile.username);
        play.try_spawn_slot(profile, input, mailbox, None)?;
    }
    Ok(play)
}

pub fn run_with_profile(
    options: &ProfilePlayOptions,
    profiles: Vec<Profile>,
) -> Result<Play, String> {
    if !profiles.is_empty() {
        options.profile.require_bot_operation()?;
    }
    let template = SharedClientTemplate::load(Arc::clone(&options.profile))?;
    run_with_template(
        template,
        options.mainland,
        profiles,
        |_| (None, None),
        |_, _, _| {},
    )
}

/// Wall spawn for the e2e ladder: every profile spawns one full `Client`
/// slot (the old `1 fat + N lean` split is gone). `heads` only selects
/// whether the first profile gets the login-FIFO front.
pub fn run_channels(options: &PlayOptions, profiles: Vec<Profile>, heads: usize) -> Play {
    let mut play = Play::new(options);
    let tv_name = profiles.first().map(|p| p.username.clone());
    for profile in profiles {
        play.spawn_slot(profile, None, None, None);
    }
    if heads >= 1 {
        if let Some(name) = tv_name {
            play.prefer_login(&name);
        }
    }
    play
}

/// Slot client config: connection fields from `options`, memory profile
/// from the vault profile (`settings.lowmem` defaults true; panel/CLI may
/// set false for this run).
pub(super) fn bot_client_config(options: &PlayOptions, profile: &Profile) -> ClientConfig {
    ClientConfig {
        host: options.host.clone(),
        port: options.port,
        cache_dir: options.cache_dir.clone(),
        members: true,
        lowmem: profile.settings.lowmem,
    }
}

/// Unlock `path`, or create it (and parent dirs) when missing. Any other
/// unlock error (`WrongPassphrase`, `Corrupt`, `EmptyPassphrase`) is returned
/// as-is so the CLI can print it instead of falling through to `AlreadyExists`.
/// A new vault requires only a non-empty passphrase after trimming surrounding
/// whitespace; an existing vault is opened with whatever decrypts it.
pub fn open_vault(path: &Path, passphrase: &str) -> Result<Vault, VaultError> {
    match Vault::unlock(path, passphrase) {
        Ok(v) => Ok(v),
        Err(VaultError::NotFound(_)) => {
            vault::check_new_passphrase(passphrase)?;
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    vault::create_private_dir(parent)?;
                }
            }
            Vault::create(path, passphrase)
        }
        Err(e) => Err(e),
    }
}

/// Unpack the config/interface jags once and share the tables across slots
/// (the client's `load_cache` is private; this mirrors it with the same
/// public `Cache::unpack` / `IfType::unpack` entry points).
type IfaceTables = (Cache, Vec<Option<Box<IfType>>>, Vec<Option<Arc<IfTypeMut>>>);
fn load_template_checked(
    cache_dir: &Path,
    observer: &progress::ProfileProgressObserver,
) -> Result<IfaceTables, String> {
    let config =
        std::fs::read(cache_dir.join("config")).map_err(|e| format!("config archive: {e}"))?;
    let interface = std::fs::read(cache_dir.join("interface"))
        .map_err(|e| format!("interface archive: {e}"))?;
    observer.report(progress::ProfileProgress::steps(
        progress::ProfileProgressStage::LoadingGameData,
        1,
        4,
    ));
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let cache = Cache::unpack(&JagFile::new(config));
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::LoadingGameData,
            2,
            4,
        ));
        let (ifaces, mutable) = IfType::unpack(&JagFile::new(interface));
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::LoadingGameData,
            3,
            4,
        ));
        if cache.objs.is_empty()
            || cache.npcs.is_empty()
            || cache.locs.is_empty()
            || !ifaces.iter().any(Option::is_some)
        {
            return Err(format!(
                "missing required config/interface tables at {}",
                cache_dir.display()
            ));
        }
        Ok((
            cache,
            ifaces,
            mutable
                .into_iter()
                .map(|v| v.map(|b| Arc::new(*b)))
                .collect(),
        ))
    }))
    .map_err(|_| {
        format!(
            "invalid config/interface archives at {}",
            cache_dir.display()
        )
    })?
}

fn load_template(cache_dir: &str) -> IfaceTables {
    let cache = match std::fs::read(format!("{cache_dir}/config")) {
        Ok(bytes) => {
            std::panic::catch_unwind(AssertUnwindSafe(|| Cache::unpack(&JagFile::new(bytes))))
                .unwrap_or_default()
        }
        Err(_) => Cache::default(),
    };
    let (ifaces, ifaces_mut) = match std::fs::read(format!("{cache_dir}/interface")) {
        Ok(bytes) => std::panic::catch_unwind(AssertUnwindSafe(|| {
            let (ifaces, ifaces_mut) = IfType::unpack(&JagFile::new(bytes));
            (
                ifaces,
                ifaces_mut
                    .into_iter()
                    .map(|o| o.map(|b| Arc::new(*b)))
                    .collect(),
            )
        }))
        .unwrap_or_default(),
        Err(_) => (Vec::new(), Vec::new()),
    };
    (cache, ifaces, ifaces_mut)
}
