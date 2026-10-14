//! Import of rs2b0t's own profile export (`rs2b0t-profiles.json`, written by
//! rs2b0t's Settings → export; rs2b0t `src/bot/multibox/ProfileTransfer.ts`).
//!
//! The file is rs2b0t's unlocked vault plus each account's box storage, in
//! plain text. [`parse_profile_file`] checks it as strictly as rs2b0t's own
//! `parseProfileFile` does (kind, version, shapes; `storage` may be absent,
//! as there) and never quotes a password in an error. [`plan_import`] maps
//! every account onto a 274bot profile without writing anything: the
//! preview. [`apply_import`] writes the planned rows through the session's
//! ordinary profile writes. Every item the import does not carry over is
//! listed with its reason, in the preview and in the final report.
//!
//! Passwords are held only in [`Secret`] (zeroed on drop), the file text and
//! the parsed JSON tree are wiped once read, and nothing here formats a
//! password into a line.
//!
//! [`ProfileImport`] is the flow the panel and the TUI both render: path,
//! preview, confirm, report, and the optional deletion of the export file.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use api::host_log;
use api::hostlog::{Category, Level};
use serde_json::{Map, Number, Value};
use vault::{Profile, ProfileSettings, RasterMode, ScriptAssignment, Secret, Vault};
use zeroize::Zeroize;

use crate::operations::{OperationId, Outcome};
use crate::scripts::Scripts;
use crate::session::{ArmMirror, OperatorSession};

/// rs2b0t's `PROFILE_FILE_KIND`.
pub const PROFILE_FILE_KIND: &str = "rs2b0t-multibox-profiles";
/// rs2b0t's `PROFILE_FILE_VERSION`; the only version this importer reads.
pub const PROFILE_FILE_VERSION: u64 = 1;
/// rs2b0t's `PROFILE_FILE_NAME`: the name its export suggests.
pub const PROFILE_FILE_NAME: &str = "rs2b0t-profiles.json";
/// Shown with every preview and report.
pub const PLAINTEXT_WARNING: &str = "This export file holds your rs2b0t account passwords in plain text. Delete it after the import, or keep it somewhere safe.";
/// The label failed writes carry on the banner and status line.
pub const LABEL: &str = "rs2b0t import";

/// rs2b0t's tab for accounts without one; it is never stored explicitly.
const MAIN_TAB: &str = "Main";
/// An export of a few hundred accounts is well under a megabyte.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Why a profile file was refused. No variant carries a password.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is larger than 16 MiB, so it is not an rs2b0t profile export", path.display())]
    TooLarge { path: PathBuf },
    #[error("the profile file is not JSON")]
    NotJson,
    #[error("the profile file has an unrecognized shape")]
    Shape,
    #[error("the profile file kind is {found}, expected '{PROFILE_FILE_KIND}': this is not an rs2b0t profile export")]
    Kind { found: String },
    #[error("the profile file version is {found}, expected {PROFILE_FILE_VERSION}: this export comes from an rs2b0t version this importer does not read")]
    Version { found: String },
    #[error("the profile file is missing profiles, tabs, or activeTab")]
    MissingLists,
    #[error("the profile file tabs must be strings")]
    TabsNotStrings,
    #[error("profile entry {index} is invalid: {reason}")]
    Entry { index: usize, reason: &'static str },
    #[error("profile '{username}' has an invalid world: expected 1, 2 or 3")]
    World { username: String },
    #[error("profile '{username}' has an invalid tab")]
    Tab { username: String },
    #[error("the profile file storage must be an object")]
    Storage,
    #[error("the profile file storage has an empty username")]
    StorageUser,
    #[error("the profile file storage for '{user}' must be an object")]
    StorageBox { user: String },
    #[error("the profile file storage '{user}.{key}' must be a string")]
    StorageValue { user: String, key: String },
    #[error("the vault is locked: unlock it before importing")]
    VaultLocked,
    #[error("cannot delete {}: {source}", path.display())]
    Delete {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// One account from the file. The password stays in a [`Secret`].
pub struct ExportAccount {
    username: String,
    password: Secret,
    tab: Option<String>,
    world: Option<u8>,
}

impl ExportAccount {
    pub fn username(&self) -> &str {
        &self.username
    }

    /// rs2b0t tab; `None` is its Main tab.
    pub fn tab(&self) -> Option<&str> {
        self.tab.as_deref()
    }

    pub fn world(&self) -> Option<u8> {
        self.world
    }
}

impl std::fmt::Debug for ExportAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportAccount")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("tab", &self.tab)
            .field("world", &self.world)
            .finish()
    }
}

/// One account's rs2b0t box storage: key suffix (after `rs2b0t:<user>:`)
/// to the stored string. The `creds` value (rs2b0t's per-tab login copy,
/// which holds the password) is never kept: its key maps to an empty value.
pub type BoxStorage = BTreeMap<String, String>;

/// rs2b0t's box key holding a per-tab copy of the login.
const CREDS_KEY: &str = "creds";

/// A parsed rs2b0t profile export.
#[derive(Debug)]
pub struct ExportFile {
    accounts: Vec<ExportAccount>,
    tabs: Vec<String>,
    active_tab: String,
    storage: BTreeMap<String, BoxStorage>,
}

impl ExportFile {
    pub fn accounts(&self) -> &[ExportAccount] {
        &self.accounts
    }

    pub fn tabs(&self) -> &[String] {
        &self.tabs
    }

    pub fn active_tab(&self) -> &str {
        &self.active_tab
    }

    pub fn storage(&self) -> &BTreeMap<String, BoxStorage> {
        &self.storage
    }
}

/// Zero every string in a parsed JSON tree when it goes out of scope, so the
/// passwords it held do not stay in freed memory (best effort, as
/// [`Secret`]: object keys are field and account names, never secrets).
struct WipeOnDrop(Value);

impl Drop for WipeOnDrop {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

fn wipe(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe),
        Value::Object(map) => map.values_mut().for_each(wipe),
        _ => {}
    }
}

/// A short JSON rendering of a non-secret header value for an error.
fn shown(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "missing".into();
    };
    let mut text = value.to_string();
    if text.len() > 60 {
        let mut end = 60;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    text
}

/// Read and parse the export at `path`. The file text is wiped once parsed.
pub fn read_profile_file(path: &Path) -> Result<ExportFile, ImportError> {
    let read_error = |source| ImportError::Read {
        path: path.to_path_buf(),
        source,
    };
    let len = std::fs::metadata(path).map_err(read_error)?.len();
    if len > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge {
            path: path.to_path_buf(),
        });
    }
    let bytes = std::fs::read(path).map_err(read_error)?;
    let text = match String::from_utf8(bytes) {
        Ok(text) => Secret::from(text),
        Err(error) => {
            error.into_bytes().zeroize();
            return Err(ImportError::NotJson);
        }
    };
    parse_profile_file(&text)
}

/// Parse an rs2b0t profile export, with the checks and messages of rs2b0t's
/// `parseProfileFile` (no entry is ever quoted, so no password is).
pub fn parse_profile_file(text: &str) -> Result<ExportFile, ImportError> {
    let parsed = WipeOnDrop(serde_json::from_str(text).map_err(|_| ImportError::NotJson)?);
    let obj = parsed.0.as_object().ok_or(ImportError::Shape)?;
    let kind = obj.get("kind");
    if kind.and_then(Value::as_str) != Some(PROFILE_FILE_KIND) {
        return Err(ImportError::Kind { found: shown(kind) });
    }
    let version = obj.get("v");
    if version.and_then(Value::as_f64) != Some(PROFILE_FILE_VERSION as f64) {
        return Err(ImportError::Version {
            found: shown(version),
        });
    }
    let (Some(profiles), Some(tabs), Some(active_tab)) = (
        obj.get("profiles").and_then(Value::as_array),
        obj.get("tabs").and_then(Value::as_array),
        obj.get("activeTab").and_then(Value::as_str),
    ) else {
        return Err(ImportError::MissingLists);
    };
    let tabs = tabs
        .iter()
        .map(|tab| tab.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or(ImportError::TabsNotStrings)?;
    let accounts = profiles
        .iter()
        .enumerate()
        .map(|(index, raw)| parse_account(index + 1, raw))
        .collect::<Result<Vec<_>, _>>()?;
    let storage = parse_storage(obj.get("storage"))?;
    Ok(ExportFile {
        accounts,
        tabs,
        active_tab: active_tab.to_owned(),
        storage,
    })
}

fn parse_account(index: usize, raw: &Value) -> Result<ExportAccount, ImportError> {
    let entry = |reason| ImportError::Entry { index, reason };
    let p = raw.as_object().ok_or(entry("not an object"))?;
    let username = p
        .get("username")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or(entry("username must be a non-empty string"))?;
    let password = p
        .get("password")
        .and_then(Value::as_str)
        .ok_or(entry("password must be a string"))?;
    let world = match p.get("world") {
        None => None,
        Some(world) => match world.as_f64() {
            Some(n) if n == 1.0 || n == 2.0 || n == 3.0 => Some(n as u8),
            _ => {
                return Err(ImportError::World {
                    username: username.to_owned(),
                })
            }
        },
    };
    let tab = match p.get("tab") {
        None => None,
        Some(Value::String(tab)) if tab == MAIN_TAB => None,
        Some(Value::String(tab)) => Some(tab.clone()),
        Some(_) => {
            return Err(ImportError::Tab {
                username: username.to_owned(),
            })
        }
    };
    Ok(ExportAccount {
        username: username.to_owned(),
        password: Secret::from(password),
        tab,
        world,
    })
}

fn parse_storage(raw: Option<&Value>) -> Result<BTreeMap<String, BoxStorage>, ImportError> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let users = raw.as_object().ok_or(ImportError::Storage)?;
    let mut out = BTreeMap::new();
    for (user, entries) in users {
        if user.is_empty() {
            return Err(ImportError::StorageUser);
        }
        let entries = entries
            .as_object()
            .ok_or_else(|| ImportError::StorageBox { user: user.clone() })?;
        let mut storage = BoxStorage::new();
        for (key, value) in entries {
            let value = value.as_str().ok_or_else(|| ImportError::StorageValue {
                user: user.clone(),
                key: key.clone(),
            })?;
            let kept = if key == CREDS_KEY {
                String::new()
            } else {
                value.to_owned()
            };
            storage.insert(key.clone(), kept);
        }
        out.insert(user.clone(), storage);
    }
    Ok(out)
}

// ---- mapping --------------------------------------------------------------

/// What happens to an account that is already in the vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    /// Keep the vault's profile as it is.
    #[default]
    Skip,
    /// Replace its password, world and the imported settings.
    Overwrite,
}

/// A catalog card the import can assign and fill.
pub struct CatalogCard<'a> {
    pub schema: &'a [script::SettingDef],
    /// Why the card cannot run in 274bot (it is dimmed in Browse).
    pub unloadable: Option<&'a str>,
    pub assignment: ScriptAssignment,
}

/// The script catalog as the import sees it: rs2b0t script names are
/// catalog register names.
pub trait ImportCatalog {
    /// Whether the rs2b0t catalog is registered in this session.
    fn loaded(&self) -> bool;
    fn card(&self, name: &str) -> Option<CatalogCard<'_>>;
}

impl ImportCatalog for script::JsLibrary {
    fn loaded(&self) -> bool {
        self.cards()
            .iter()
            .any(|card| card.source == script::ScriptSource::Catalog)
    }

    fn card(&self, name: &str) -> Option<CatalogCard<'_>> {
        self.get(script::ScriptSource::Catalog, name)
            .map(|card| CatalogCard {
                schema: &card.settings_schema,
                unloadable: card.unloadable.as_deref(),
                assignment: card.assignment(),
            })
    }
}

/// One rs2b0t item the import does not carry over, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub item: String,
    pub reason: String,
}

fn skipped(item: impl Into<String>, reason: impl Into<String>) -> Skipped {
    Skipped {
        item: item.into(),
        reason: reason.into(),
    }
}

/// What the import does with one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    /// Not in the vault: created.
    New,
    /// In the vault and Overwrite chosen: replaced.
    Overwrite,
    /// In the vault and already equal to what Overwrite would write.
    Unchanged,
    /// In the vault and Skip chosen: kept as it is.
    Exists,
    /// The file lists this username again; its first entry is used.
    Duplicate,
    /// The username is only spaces.
    BlankName,
}

impl AccountStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "new profile",
            Self::Overwrite => "overwrite existing profile",
            Self::Unchanged => "already matches the export: nothing to write",
            Self::Exists => "already in the vault: kept as it is (choose Overwrite to replace it)",
            Self::Duplicate => "listed twice in the file: only its first entry is imported",
            Self::BlankName => "the username is only spaces: not imported",
        }
    }
}

/// The 274bot side of one account's mapped fields.
#[derive(Default)]
struct Mapped {
    world: Option<u16>,
    lamp_skill: Option<String>,
    clue_duel_partner: Option<String>,
    renderer_on: Option<bool>,
    assignment: Option<ScriptAssignment>,
    bags: BTreeMap<String, Map<String, Value>>,
    imported: Vec<String>,
    skipped: Vec<Skipped>,
}

impl Mapped {
    fn apply(&self, settings: &mut ProfileSettings) {
        settings.world = self.world;
        if let Some(skill) = &self.lamp_skill {
            settings.lamp_skill.clone_from(skill);
        }
        if let Some(partner) = &self.clue_duel_partner {
            settings.clue_duel_partner.clone_from(partner);
        }
        match self.renderer_on {
            Some(false) => settings.raster = RasterMode::Off,
            Some(true) if settings.raster == RasterMode::Off => settings.raster = RasterMode::Gpu,
            _ => {}
        }
        if let Some(assignment) = &self.assignment {
            settings.script_assignment = Some(assignment.clone());
        }
        for (key, bag) in &self.bags {
            settings.script_settings.insert(key.clone(), bag.clone());
        }
    }
}

/// One account in the preview.
pub struct AccountPlan {
    username: String,
    status: AccountStatus,
    /// The row to write (New and Overwrite only).
    row: Option<Profile>,
    /// The vault row it replaces (Overwrite only), for the log line.
    old: Option<Profile>,
    imported: Vec<String>,
    skipped: Vec<Skipped>,
}

impl AccountPlan {
    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn status(&self) -> AccountStatus {
        self.status
    }

    /// What the import carries over, one line per item (no password).
    pub fn imported(&self) -> &[String] {
        &self.imported
    }

    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// The settings the import writes (New and Overwrite only).
    pub fn settings(&self) -> Option<&ProfileSettings> {
        self.row.as_ref().map(|row| &row.settings)
    }
}

/// The preview: every account and file-level item, nothing written.
pub struct ImportPlan {
    policy: ConflictPolicy,
    accounts: Vec<AccountPlan>,
    file_skipped: Vec<Skipped>,
}

impl ImportPlan {
    pub fn policy(&self) -> ConflictPolicy {
        self.policy
    }

    pub fn accounts(&self) -> &[AccountPlan] {
        &self.accounts
    }

    /// Items of the file that belong to no single account.
    pub fn file_skipped(&self) -> &[Skipped] {
        &self.file_skipped
    }

    pub fn count(&self, status: AccountStatus) -> usize {
        self.accounts.iter().filter(|a| a.status == status).count()
    }

    /// Whether confirming would write anything.
    pub fn writes(&self) -> bool {
        self.accounts.iter().any(|a| a.row.is_some())
    }

    /// The preview text, shared by the panel and the TUI.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!(
                "{} accounts: {} new, {} to overwrite, {} unchanged, {} kept as they are, {} duplicate.",
                self.accounts.len(),
                self.count(AccountStatus::New),
                self.count(AccountStatus::Overwrite),
                self.count(AccountStatus::Unchanged),
                self.count(AccountStatus::Exists),
                self.count(AccountStatus::Duplicate),
            ),
            format!(
                "Existing profiles: {}.",
                match self.policy {
                    ConflictPolicy::Skip => "skip (kept as they are)",
                    ConflictPolicy::Overwrite => "overwrite",
                }
            ),
        ];
        for account in &self.accounts {
            out.push(String::new());
            out.push(format!("{}: {}", account.username, account.status.label()));
            if account.row.is_some() {
                out.push("  password: imported into the vault".into());
                for line in &account.imported {
                    out.push(format!("  {line}"));
                }
            }
            for skip in &account.skipped {
                out.push(format!("  not imported: {}: {}", skip.item, skip.reason));
            }
        }
        if !self.file_skipped.is_empty() {
            out.push(String::new());
            for skip in &self.file_skipped {
                out.push(format!("not imported: {}: {}", skip.item, skip.reason));
            }
        }
        out.push(String::new());
        out.push(PLAINTEXT_WARNING.into());
        out
    }
}

/// Map the file onto the vault: the preview. Writes nothing.
pub fn plan_import(
    file: &ExportFile,
    vault: &Vault,
    catalog: &impl ImportCatalog,
    policy: ConflictPolicy,
) -> ImportPlan {
    let mut next_uid = crate::fresh_uid(vault);
    let mut seen = BTreeSet::new();
    let mut accounts = Vec::with_capacity(file.accounts.len());
    for account in &file.accounts {
        let username = account.username.trim().to_owned();
        let status_only = |username: String, status| AccountPlan {
            username,
            status,
            row: None,
            old: None,
            imported: Vec::new(),
            skipped: Vec::new(),
        };
        if username.is_empty() {
            accounts.push(status_only(
                account.username.clone(),
                AccountStatus::BlankName,
            ));
            continue;
        }
        if !seen.insert(username.clone()) {
            accounts.push(status_only(username, AccountStatus::Duplicate));
            continue;
        }
        let existing = vault.get(&username);
        if existing.is_some() && policy == ConflictPolicy::Skip {
            accounts.push(status_only(username, AccountStatus::Exists));
            continue;
        }
        let mapped = map_account(account, file.storage.get(&account.username), catalog);
        let (status, row, old) = match existing {
            None => {
                let mut settings = ProfileSettings::default();
                mapped.apply(&mut settings);
                let row = Profile {
                    username: username.clone(),
                    password: account.password.clone(),
                    uid: next_uid,
                    settings,
                };
                next_uid += 1;
                (AccountStatus::New, Some(row), None)
            }
            Some(current) => {
                let mut row = current.clone();
                row.password = account.password.clone();
                mapped.apply(&mut row.settings);
                if row == *current {
                    (AccountStatus::Unchanged, None, None)
                } else {
                    (AccountStatus::Overwrite, Some(row), Some(current.clone()))
                }
            }
        };
        accounts.push(AccountPlan {
            username,
            status,
            row,
            old,
            imported: mapped.imported,
            skipped: mapped.skipped,
        });
    }
    ImportPlan {
        policy,
        accounts,
        file_skipped: file_skipped(file),
    }
}

fn file_skipped(file: &ExportFile) -> Vec<Skipped> {
    let mut out = Vec::new();
    if !file.tabs.is_empty() {
        out.push(skipped(
            format!("tabs ({})", file.tabs.join(", ")),
            "274bot has no profile tabs or groups",
        ));
    }
    if file.active_tab != MAIN_TAB {
        out.push(skipped(
            format!("active tab '{}'", file.active_tab),
            "274bot has no profile tabs or groups",
        ));
    }
    let listed: BTreeSet<&str> = file.accounts.iter().map(|a| a.username.as_str()).collect();
    for (user, storage) in &file.storage {
        if !listed.contains(user.as_str()) {
            out.push(skipped(
                format!("stored settings for '{user}' ({} items)", storage.len()),
                "the file has no account with this name",
            ));
        }
    }
    out
}

/// rs2b0t `Global` settings 274bot has no per-account equivalent for, with
/// the reason each is skipped (rs2b0t `src/bot/runtime/Settings.ts`,
/// `GLOBAL_SETTINGS_CORE` and `NAV_SETTINGS`). `lampSkill` and
/// `clueDuelPartner` are mapped in [`map_global`]. The route-drawing ones
/// have app-wide panel equivalents (`panel` `NavSettings`), which one
/// account's import does not change.
const GLOBAL_NOT_SUPPORTED: [(&str, &str); 20] = [
    (
        "bankCommonJunk",
        GLOBAL_SCRIPT_DEFAULT,
    ),
    (
        "useZanarisBank",
        GLOBAL_SCRIPT_DEFAULT,
    ),
    (
        "useMageBank",
        GLOBAL_SCRIPT_DEFAULT,
    ),
    (
        "runAuto",
        "274bot turns run back on by itself (scripts may change that); there is no per-account setting",
    ),
    (
        "runEnergyMin",
        "274bot turns run back on at 20% energy (scripts may change that); there is no per-account setting",
    ),
    (
        "navTeleports",
        "274bot's teleport permission is one app-wide switch (Nav config: allow teleports), not per account",
    ),
    ("navPathStallTicks", "274bot's walker repaths by its own rules; there is no setting"),
    ("navPathDeviation", "274bot's walker repaths by its own rules; there is no setting"),
    (
        "navCameraFollow",
        "app-wide in 274bot (panel Nav settings: camera follows path), not per account",
    ),
    ("showNavPath", PANEL_PATH_PAINT),
    ("navPathShowText", PANEL_PATH_PAINT),
    ("navPathTextSize", PANEL_PATH_PAINT),
    ("navPathColorPath", PANEL_PATH_PAINT),
    ("navPathColorTransport", PANEL_PATH_PAINT),
    ("navPathColorClick", PANEL_PATH_PAINT),
    ("navPathColorText", PANEL_PATH_PAINT),
    (
        "navPathSceneExpand",
        "rs2b0t experimental route-drawing option; 274bot has no equivalent",
    ),
    ("navPathClientSegment", PANEL_PATH_PAINT),
    ("navPathColorClient", PANEL_PATH_PAINT),
    ("navPathColorClientRunAlt", PANEL_PATH_PAINT),
];

/// rs2b0t `Settings.ts` `winningRaw`: a script parameter with no saved value
/// of its own falls back to the `Global` value of the same key.
const GLOBAL_SCRIPT_DEFAULT: &str = "in rs2b0t this is the account-wide default for scripts that have this parameter; 274bot has no account-wide default, so set it in each script's parameters";

const PANEL_PATH_PAINT: &str =
    "route drawing is app-wide in 274bot (panel Nav settings), not per account";

fn map_account(
    account: &ExportAccount,
    storage: Option<&BoxStorage>,
    catalog: &impl ImportCatalog,
) -> Mapped {
    let mut mapped = Mapped {
        world: account.world.map(u16::from),
        ..Mapped::default()
    };
    if let Some(world) = account.world {
        mapped.imported.push(format!(
            "world: w{world} (used when the server's world list offers it)"
        ));
    }
    if let Some(tab) = &account.tab {
        mapped.skipped.push(skipped(
            format!("tab '{tab}'"),
            "274bot has no profile tabs or groups",
        ));
    }
    let Some(storage) = storage else {
        return mapped;
    };
    for (key, value) in storage {
        if key == "selectedScript" {
            map_selected_script(&mut mapped, value, catalog);
        } else if key == "rendererEnabled" {
            match value.as_str() {
                "0" => {
                    mapped.renderer_on = Some(false);
                    mapped
                        .imported
                        .push("display: off (rs2b0t renderer disabled)".into());
                }
                "1" => {
                    mapped.renderer_on = Some(true);
                    mapped
                        .imported
                        .push("display: on (rs2b0t renderer enabled)".into());
                }
                other => mapped.skipped.push(skipped(
                    "rendererEnabled",
                    format!("unrecognized value '{other}'"),
                )),
            }
        } else if key == CREDS_KEY {
            mapped.skipped.push(skipped(
                "creds",
                "rs2b0t's per-tab copy of the login; the password comes from the account list (value not shown)",
            ));
        } else if let Some(rest) = key.strip_prefix("set:") {
            match rest.split_once(':') {
                Some(("Global", setting)) => map_global(&mut mapped, setting, value),
                Some(("MapPicker", setting)) => mapped.skipped.push(skipped(
                    format!("MapPicker.{setting}"),
                    "rs2b0t tile-map picker display option; 274bot's map has its own settings",
                )),
                Some(("Loadouts", setting)) => mapped.skipped.push(skipped(
                    format!("Loadouts.{setting}"),
                    "rs2b0t loadouts are not imported; 274bot keeps one loadout list for all accounts in its Loadouts window",
                )),
                Some(("PriceBooks", setting)) => mapped.skipped.push(skipped(
                    format!("PriceBooks.{setting}"),
                    "order books are not supported in 274bot",
                )),
                Some((script, setting)) => {
                    map_script_setting(&mut mapped, script, setting, value, catalog)
                }
                None => mapped.skipped.push(skipped(
                    key.clone(),
                    "unrecognized rs2b0t storage key",
                )),
            }
        } else {
            mapped
                .skipped
                .push(skipped(key.clone(), "unrecognized rs2b0t storage key"));
        }
    }
    mapped
}

const CATALOG_NOT_LOADED: &str = "the rs2b0t script catalog is not loaded in 274bot (import it from Scripts first, then preview again)";

fn map_selected_script(mapped: &mut Mapped, name: &str, catalog: &impl ImportCatalog) {
    let item = format!("selected script '{name}'");
    if name.is_empty() {
        mapped.skipped.push(skipped(item, "no script selected"));
        return;
    }
    if !catalog.loaded() {
        mapped.skipped.push(skipped(item, CATALOG_NOT_LOADED));
        return;
    }
    match catalog.card(name) {
        None => mapped
            .skipped
            .push(skipped(item, "274bot's catalog has no script of this name")),
        Some(card) => match card.unloadable {
            Some(reason) => mapped.skipped.push(skipped(
                item,
                format!("dimmed in 274bot, it cannot run here ({reason}); its parameters are still imported"),
            )),
            None => {
                mapped.imported.push(format!("script: {name}"));
                mapped.assignment = Some(card.assignment);
            }
        },
    }
}

fn map_global(mapped: &mut Mapped, setting: &str, value: &str) {
    let item = format!("Global.{setting}");
    match setting {
        "lampSkill" => {
            let skill = value.trim().to_ascii_lowercase();
            if host::is_lamp_skill(&skill) {
                mapped.imported.push(format!("lamp skill: {skill}"));
                mapped.lamp_skill = Some(skill);
            } else {
                mapped.skipped.push(skipped(
                    item,
                    format!("'{value}' is not a skill the genie lamp offers"),
                ));
            }
        }
        "clueDuelPartner" => {
            let partner = value.trim().to_owned();
            mapped.imported.push(if partner.is_empty() {
                "clue duel partner: (none)".to_string()
            } else {
                format!("clue duel partner: {partner}")
            });
            mapped.clue_duel_partner = Some(partner);
        }
        _ => {
            let reason = GLOBAL_NOT_SUPPORTED
                .iter()
                .find(|(key, _)| *key == setting)
                .map_or("not a Global setting this importer knows", |(_, reason)| {
                    reason
                });
            mapped.skipped.push(skipped(item, reason));
        }
    }
}

fn map_script_setting(
    mapped: &mut Mapped,
    script: &str,
    setting: &str,
    raw: &str,
    catalog: &impl ImportCatalog,
) {
    let item = format!("{script}.{setting}");
    if !catalog.loaded() {
        mapped.skipped.push(skipped(item, CATALOG_NOT_LOADED));
        return;
    }
    let Some(card) = catalog.card(script) else {
        mapped.skipped.push(skipped(
            item,
            format!("274bot's catalog has no script named '{script}'"),
        ));
        return;
    };
    let Some(def) = card.schema.iter().find(|def| def.id == setting) else {
        mapped.skipped.push(skipped(
            item,
            format!("'{script}' has no parameter '{setting}' in 274bot's catalog"),
        ));
        return;
    };
    match convert_setting(def, raw) {
        Ok(converted) => {
            for entry in converted.dropped {
                mapped.skipped.push(skipped(
                    format!("{item} entry '{entry}'"),
                    "not one of this parameter's options",
                ));
            }
            let value = script::migrate_legacy_setting_value(script, setting, &converted.value);
            let mut line = format!("{item} = {}", script::format_setting_value(&value));
            if let Some(note) = converted.note {
                let _ = write!(line, " ({note})");
            }
            mapped.imported.push(line);
            mapped
                .bags
                .entry(card.assignment.key())
                .or_default()
                .insert(setting.to_owned(), value);
        }
        Err(reason) => mapped.skipped.push(skipped(item, reason)),
    }
}

/// An rs2b0t setting string converted to 274bot's stored value.
struct Converted {
    value: Value,
    /// How the value differs from the stored text, when it does.
    note: Option<String>,
    /// List entries rs2b0t itself drops (not one of the options).
    dropped: Vec<String>,
}

/// rs2b0t's `parseValue` (`Settings.ts`) for one stored string: the value
/// the account actually ran with. A string rs2b0t would replace with the
/// default is refused instead, so the skip is reported.
fn convert_setting(def: &script::SettingDef, raw: &str) -> Result<Converted, String> {
    let plain = |value| {
        Ok(Converted {
            value,
            note: None,
            dropped: Vec::new(),
        })
    };
    let option = |wanted: &str| {
        def.options
            .iter()
            .find(|option| option.eq_ignore_ascii_case(wanted))
            .cloned()
    };
    match def.ty.as_str() {
        "boolean" => {
            let normalized = raw.trim().to_ascii_lowercase();
            plain(Value::Bool(matches!(
                normalized.as_str(),
                "true" | "1" | "yes"
            )))
        }
        "number" => {
            let text = raw.trim();
            let parsed: f64 = text
                .parse()
                .ok()
                .filter(|n: &f64| n.is_finite())
                .ok_or_else(|| format!("'{raw}' is not a number"))?;
            let bound = |text: &Option<String>| text.as_deref().and_then(|t| t.parse::<f64>().ok());
            let mut clamped = parsed;
            if let Some(min) = bound(&def.min) {
                clamped = clamped.max(min);
            }
            if let Some(max) = bound(&def.max) {
                clamped = clamped.min(max);
            }
            let number =
                Number::from_f64(clamped).ok_or_else(|| format!("'{raw}' is not a number"))?;
            Ok(Converted {
                value: script::coerce_setting_value("number", &Value::Number(number)),
                note: (clamped != parsed)
                    .then(|| format!("{text} limited to the parameter's range, as rs2b0t does")),
                dropped: Vec::new(),
            })
        }
        "string" if !def.options.is_empty() => {
            let chosen = option(raw.trim())
                .ok_or_else(|| format!("'{raw}' is not one of this parameter's options"))?;
            plain(Value::String(chosen))
        }
        "string" => plain(Value::String(raw.trim().to_owned())),
        "string[]" | "list" => {
            let entries = raw.split(',').map(str::trim).filter(|s| !s.is_empty());
            if def.options.is_empty() {
                return plain(Value::Array(
                    entries.map(|s| Value::String(s.to_owned())).collect(),
                ));
            }
            let mut values = Vec::new();
            let mut dropped = Vec::new();
            for entry in entries {
                match option(entry) {
                    Some(chosen) => values.push(Value::String(chosen)),
                    None => dropped.push(entry.to_owned()),
                }
            }
            Ok(Converted {
                value: Value::Array(values),
                note: None,
                dropped,
            })
        }
        "tile" => {
            let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
            let coords = parts
                .iter()
                .map(|part| part.parse::<i64>())
                .collect::<Result<Vec<_>, _>>();
            match coords {
                Ok(coords) if coords.len() >= 2 => plain(serde_json::json!({
                    "x": coords[0],
                    "z": coords[1],
                    "level": coords.get(2).copied().unwrap_or(0),
                })),
                _ => Err(format!("'{raw}' is not a tile (x,z or x,z,level)")),
            }
        }
        other => Err(format!("parameter type '{other}' is not supported")),
    }
}

// ---- apply and report ------------------------------------------------------

/// One account's write.
struct RunEntry {
    username: String,
    status: AccountStatus,
    op: Option<OperationId>,
    /// Refused before anything was written.
    refused: Option<String>,
}

/// A confirmed import: its queued writes, then its final report.
pub struct ImportRun {
    entries: Vec<RunEntry>,
    skipped: Vec<(String, Skipped)>,
}

/// One account's settled result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountResult {
    Created,
    Overwritten,
    /// Nothing written: unchanged, kept or duplicate.
    NotWritten(AccountStatus),
    Failed(String),
    /// The write is still being saved.
    Saving,
}

/// Write the plan's rows through the session's profile writes (a new
/// profile never replaces one that appeared since the preview) and log one
/// line per write. Returns at once; [`ImportRun::settled`] tells when every
/// write is durable.
pub fn apply_import<Io>(
    core: &mut OperatorSession<Io>,
    plan: ImportPlan,
) -> Result<ImportRun, ImportError> {
    if core.vault().is_none() {
        return Err(ImportError::VaultLocked);
    }
    let mut entries = Vec::with_capacity(plan.accounts.len());
    let mut skips = Vec::new();
    for account in plan.accounts {
        for skip in account.skipped {
            skips.push((account.username.clone(), skip));
        }
        let mut entry = RunEntry {
            username: account.username,
            status: account.status,
            op: None,
            refused: None,
        };
        if let Some(row) = account.row {
            let summary = change_summary(account.old.as_ref(), &row);
            let (action, result) = match account.status {
                AccountStatus::New => (
                    "create",
                    core.create_profile_with_log_action(
                        row,
                        ArmMirror::Remember(None),
                        LABEL,
                        LABEL,
                    ),
                ),
                _ => (
                    "overwrite",
                    core.save_profile_with_log_action(row, ArmMirror::Remember(None), LABEL, LABEL),
                ),
            };
            match result {
                Ok(op) => {
                    host_log!(
                        Category::Lifecycle,
                        Level::Info,
                        slot = &entry.username,
                        "{LABEL}: {action} profile {} op#{}: {summary}",
                        entry.username,
                        op.0
                    );
                    entry.op = Some(op);
                }
                Err(error) => entry.refused = Some(error),
            }
        }
        entries.push(entry);
    }
    let skipped = skips
        .into_iter()
        .chain(
            plan.file_skipped
                .into_iter()
                .map(|s| ("file".to_owned(), s)),
        )
        .collect();
    Ok(ImportRun { entries, skipped })
}

/// `old -> new` for every field the import writes; the password only as
/// set/changed/unchanged.
fn change_summary(old: Option<&Profile>, new: &Profile) -> String {
    let default = ProfileSettings::default();
    let before = old.map_or(&default, |p| &p.settings);
    let after = &new.settings;
    let world = |w: Option<u16>| w.map_or_else(|| "auto".to_string(), |n| format!("w{n}"));
    let mut parts = vec![match old {
        None => "password set".to_string(),
        Some(old) if old.password == new.password => "password unchanged".into(),
        Some(_) => "password changed".into(),
    }];
    parts.push(format!(
        "world {} -> {}",
        world(before.world),
        world(after.world)
    ));
    if before.lamp_skill != after.lamp_skill {
        parts.push(format!(
            "lamp skill {} -> {}",
            before.lamp_skill, after.lamp_skill
        ));
    }
    if before.clue_duel_partner != after.clue_duel_partner {
        parts.push(format!(
            "clue duel partner '{}' -> '{}'",
            before.clue_duel_partner, after.clue_duel_partner
        ));
    }
    if before.raster != after.raster {
        parts.push(format!("display {:?} -> {:?}", before.raster, after.raster));
    }
    let key = |s: &ProfileSettings| {
        s.script_assignment
            .as_ref()
            .map_or_else(|| "None".to_string(), ScriptAssignment::key)
    };
    parts.push(format!("script {} -> {}", key(before), key(after)));
    for (card, bag) in &after.script_settings {
        if before.script_settings.get(card) != Some(bag) {
            parts.push(format!("{card} parameters: {} saved", bag.len()));
        }
    }
    parts.join("; ")
}

impl ImportRun {
    /// Every account's result now.
    pub fn results<Io>(&self, core: &OperatorSession<Io>) -> Vec<(&str, AccountResult)> {
        self.entries
            .iter()
            .map(|entry| {
                let result = if let Some(error) = &entry.refused {
                    AccountResult::Failed(error.clone())
                } else if let Some(op) = entry.op {
                    match core.operation(op).and_then(|r| r.outcome(&entry.username)) {
                        Some(Outcome::Pending) => AccountResult::Saving,
                        Some(Outcome::Failed(error)) => AccountResult::Failed(error.clone()),
                        // A completed write, or one a later write of the same
                        // profile carried (its row is durable either way).
                        _ if entry.status == AccountStatus::New => AccountResult::Created,
                        _ => AccountResult::Overwritten,
                    }
                } else {
                    AccountResult::NotWritten(entry.status)
                };
                (entry.username.as_str(), result)
            })
            .collect()
    }

    /// Whether every write has settled.
    pub fn settled<Io>(&self, core: &OperatorSession<Io>) -> bool {
        !self
            .results(core)
            .iter()
            .any(|(_, result)| *result == AccountResult::Saving)
    }

    /// Whether every write settled and none failed.
    pub fn succeeded<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.results(core)
            .iter()
            .all(|(_, result)| !matches!(result, AccountResult::Saving | AccountResult::Failed(_)))
    }

    /// The final report, shared by the panel and the TUI.
    pub fn lines<Io>(&self, core: &OperatorSession<Io>) -> Vec<String> {
        let results = self.results(core);
        let count =
            |want: fn(&AccountResult) -> bool| results.iter().filter(|(_, r)| want(r)).count();
        let mut out = vec![format!(
            "Import report: {} created, {} overwritten, {} not written, {} failed, {} still saving.",
            count(|r| *r == AccountResult::Created),
            count(|r| *r == AccountResult::Overwritten),
            count(|r| matches!(r, AccountResult::NotWritten(_))),
            count(|r| matches!(r, AccountResult::Failed(_))),
            count(|r| *r == AccountResult::Saving),
        )];
        for (name, result) in &results {
            out.push(match result {
                AccountResult::Created => format!("{name}: created"),
                AccountResult::Overwritten => format!("{name}: overwritten"),
                AccountResult::NotWritten(status) => format!("{name}: {}", status.label()),
                AccountResult::Failed(error) => format!("{name}: failed: {error}"),
                AccountResult::Saving => format!("{name}: saving…"),
            });
        }
        if !self.skipped.is_empty() {
            out.push(String::new());
            out.push(format!("Not imported ({} items):", self.skipped.len()));
            for (owner, skip) in &self.skipped {
                out.push(format!("{owner}: {}: {}", skip.item, skip.reason));
            }
        }
        out.push(String::new());
        out.push("Loaded bots use an imported password and world at their next login, and imported script parameters at their next Start.".into());
        out
    }
}

/// Delete the export file once the import succeeded (explicit confirmation
/// in the front end). Logs one line.
pub fn delete_export_file(path: &Path) -> Result<(), ImportError> {
    std::fs::remove_file(path).map_err(|source| ImportError::Delete {
        path: path.to_path_buf(),
        source,
    })?;
    host_log!(
        Category::Lifecycle,
        Level::Info,
        "{LABEL}: deleted the export file {}",
        path.display()
    );
    Ok(())
}

// ---- the shared front-end flow ---------------------------------------------

/// Where the import flow is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for a file path.
    Path,
    /// A preview is shown; nothing is written until Confirm.
    Preview,
    /// Confirmed: the writes are being saved, then the report shows.
    Report,
}

/// The import flow the panel window and the TUI overlay render: a file path,
/// then a preview with the existing-profile choice, then Confirm, then the
/// report with the offer to delete the export file.
pub struct ProfileImport {
    path: String,
    error: Option<String>,
    file: Option<(PathBuf, ExportFile)>,
    policy: ConflictPolicy,
    plan: Option<ImportPlan>,
    run: Option<(PathBuf, ImportRun)>,
    delete_asked: bool,
    deleted: Option<PathBuf>,
}

impl Default for ProfileImport {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileImport {
    pub fn new() -> Self {
        Self {
            path: String::new(),
            error: None,
            file: None,
            policy: ConflictPolicy::Skip,
            plan: None,
            run: None,
            delete_asked: false,
            deleted: None,
        }
    }

    pub fn stage(&self) -> Stage {
        if self.run.is_some() {
            Stage::Report
        } else if self.plan.is_some() {
            Stage::Preview
        } else {
            Stage::Path
        }
    }

    /// The file path being typed.
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn path_mut(&mut self) -> &mut String {
        &mut self.path
    }

    /// The last refusal (unreadable file, wrong kind, locked vault, …).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn policy(&self) -> ConflictPolicy {
        self.policy
    }

    pub fn plan(&self) -> Option<&ImportPlan> {
        self.plan.as_ref()
    }

    /// Read and map the file at the typed path. Writes nothing.
    pub fn preview<Io>(&mut self, core: &OperatorSession<Io>, scripts: &Scripts) {
        self.error = None;
        self.plan = None;
        self.file = None;
        let path = PathBuf::from(self.path.trim());
        if path.as_os_str().is_empty() {
            self.error = Some(format!(
                "Type the path of the {PROFILE_FILE_NAME} file rs2b0t exported."
            ));
            return;
        }
        let Some(vault) = core.vault() else {
            self.error = Some(ImportError::VaultLocked.to_string());
            return;
        };
        match read_profile_file(&path) {
            Ok(file) => {
                self.plan = Some(plan_import(&file, vault, &scripts.js, self.policy));
                self.file = Some((path, file));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Choose what happens to existing profiles; the preview follows.
    pub fn set_policy<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        scripts: &Scripts,
        policy: ConflictPolicy,
    ) {
        self.policy = policy;
        if let (Some((_, file)), Some(vault)) = (self.file.as_ref(), core.vault()) {
            self.plan = Some(plan_import(file, vault, &scripts.js, policy));
        }
    }

    /// The preview text.
    pub fn preview_lines(&self) -> Vec<String> {
        self.plan
            .as_ref()
            .map(ImportPlan::lines)
            .unwrap_or_default()
    }

    /// Write the previewed import. The plan is re-made against the vault as
    /// it is now, so a profile added since the preview is never replaced
    /// under Skip. The parsed file (and its passwords) is dropped.
    pub fn confirm<Io>(&mut self, core: &mut OperatorSession<Io>, scripts: &Scripts) {
        let Some((path, file)) = self.file.take() else {
            return;
        };
        self.plan = None;
        let Some(vault) = core.vault() else {
            self.error = Some(ImportError::VaultLocked.to_string());
            return;
        };
        let plan = plan_import(&file, vault, &scripts.js, self.policy);
        drop(file);
        match apply_import(core, plan) {
            Ok(run) => self.run = Some((path, run)),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Back to the path field, dropping the parsed file and its passwords.
    pub fn back(&mut self) {
        self.file = None;
        self.plan = None;
        self.error = None;
    }

    /// The report text (while writes settle and after).
    pub fn report_lines<Io>(&self, core: &OperatorSession<Io>) -> Vec<String> {
        let Some((path, run)) = self.run.as_ref() else {
            return Vec::new();
        };
        let mut out = run.lines(core);
        match &self.deleted {
            Some(deleted) => out.push(format!("Deleted {}.", deleted.display())),
            None => {
                out.push(PLAINTEXT_WARNING.into());
                if run.succeeded(core) && path.exists() {
                    out.push(format!("You can delete {} now.", path.display()));
                }
            }
        }
        out
    }

    /// Whether a confirmed import's writes have all settled.
    pub fn settled<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.run.as_ref().is_some_and(|(_, run)| run.settled(core))
    }

    /// Whether the flow holds nothing: no run, no parsed file (so no
    /// password) and no refusal. A front end resets a flow it stopped
    /// showing.
    pub fn is_fresh(&self) -> bool {
        self.run.is_none() && self.file.is_none() && self.error.is_none()
    }

    /// Whether the report may offer to delete the export file: every write
    /// settled without a failure and the file is still there.
    pub fn can_delete<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.deleted.is_none()
            && self
                .run
                .as_ref()
                .is_some_and(|(path, run)| run.succeeded(core) && path.exists())
    }

    /// Ask to delete the export file; [`Self::confirm_delete`] deletes it.
    pub fn ask_delete(&mut self) {
        self.delete_asked = true;
    }

    pub fn delete_asked(&self) -> bool {
        self.delete_asked
    }

    pub fn cancel_delete(&mut self) {
        self.delete_asked = false;
    }

    /// The export file the report offers to delete.
    pub fn export_path(&self) -> Option<&Path> {
        self.run.as_ref().map(|(path, _)| path.as_path())
    }

    /// Delete the export file after the operator confirmed.
    pub fn confirm_delete<Io>(&mut self, core: &OperatorSession<Io>) {
        self.delete_asked = false;
        if !self.can_delete(core) {
            return;
        }
        let Some((path, _)) = self.run.as_ref() else {
            return;
        };
        match delete_export_file(path) {
            Ok(()) => self.deleted = Some(path.clone()),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "rs2b0t_import_tests.rs"]
mod tests;
