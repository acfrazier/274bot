//! The shipped maintenance commands (T5.3 §5.3, §5.5; S8):
//!
//! ```text
//! panel-play state preferences recover --source "/absolute/path/private-valid.json"
//! panel-play state preferences recover --defaults
//! panel-play state vault recover --defaults --profile <server>
//! ```
//!
//! (and the same for `tui-play`). Each takes the root-use lease exclusive
//! without waiting, so it never runs beside a user of the root; it does its
//! work and exits. It opens no window, terminal session or game slot, and
//! never starts a normal preferences session.
//!
//! The 0.2.9 same-root preference export is retired (SD1 Q4): this release
//! converts saved state one way, and an edit a downgraded 0.2.9 makes is
//! never imported back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use api::host_log;
use api::hostlog::{Category, Level};
use api::local_state::StoreVersion;
use serde_json::Map;

use super::bank;
use super::files::{self, StateRoot};
use super::root_use::RootUse;
use super::sql::{self, Frontend, Legacy, SqlFailure};
use crate::servers::{LaunchProfile, LaunchTransport, LaunchWorld, LoginKey};

/// Field updates for `state servers edit <target>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerEditPatch {
    pub name: Option<String>,
    pub revision: Option<u16>,
    pub transport: Option<LaunchTransport>,
    pub vault: Option<String>,
    pub worlds: Option<Vec<LaunchWorld>>,
    pub login_key: Option<LoginKey>,
    pub members: Option<Option<bool>>,
    pub allow_plaintext_offhost: Option<bool>,
}

impl ServerEditPatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.revision.is_none()
            && self.transport.is_none()
            && self.vault.is_none()
            && self.worlds.is_none()
            && self.login_key.is_none()
            && self.members.is_none()
            && self.allow_plaintext_offhost.is_none()
    }

    pub fn apply(&self, existing: &LaunchProfile) -> LaunchProfile {
        let mut next = existing.clone();
        if let Some(name) = &self.name {
            next.name.clone_from(name);
        }
        if let Some(revision) = self.revision {
            next.revision = revision;
        }
        if let Some(transport) = self.transport {
            next.transport = transport;
        }
        if let Some(vault) = &self.vault {
            next.vault.clone_from(vault);
        }
        if let Some(worlds) = &self.worlds {
            next.worlds.clone_from(worlds);
        }
        if let Some(login_key) = &self.login_key {
            next.login_key = login_key.clone();
        }
        if let Some(members) = self.members {
            next.members = members;
        }
        if let Some(allow_plaintext_offhost) = self.allow_plaintext_offhost {
            next.allow_plaintext_offhost = allow_plaintext_offhost;
        }
        next
    }
}

/// One maintenance command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintenanceAction {
    /// `state vault recover --defaults --profile <server>`: after the
    /// operator authenticates the configured vault and confirms, create
    /// default settings rows with fresh uids for its credentials that have
    /// none (S8 missing-row recovery).
    RecoverVaultDefaults(String),
    /// `state preferences recover --source <path>`: preserve the database
    /// set, then start a new preference history from a private, valid JSON
    /// object at an absolute path.
    RecoverSource(PathBuf),
    /// `state preferences recover --defaults`: preserve the database set,
    /// then start a new preference history with default preferences.
    RecoverDefaults,
    /// `state bank-hints accept-projection <profile>/<account>/<revision>/<content>`:
    /// after the operator confirms, clear the read-only refusal of an
    /// imported bank hint whose file held fields this build does not know,
    /// so later saves hold the usable rows only.
    AcceptBankProjection(String),
    /// Accept only the named imported loadout projection after confirmation.
    AcceptLoadoutProjection(String),
    /// `state servers show [--json]`: read-only inspection of the durable
    /// server roster and available shipped presets.
    ServersShow { json: bool },
    /// `state servers add --preset <name>`: add a shipped preset to the
    /// durable server roster through the same checks as custom add.
    ServersAddPreset(String),
    /// `state servers add --name ...`: add a custom server profile.
    ServersAddCustom(LaunchProfile),
    /// `state servers edit <target> ...`: edit fields of a saved server profile.
    ServersEdit {
        target: String,
        patch: ServerEditPatch,
    },
    /// `state servers remove <target>`: remove a server profile from the
    /// durable roster while preserving any S8 vault binding and profile rows.
    ServersRemove(String),
}
/// Why a maintenance command did nothing (or stopped), in the operator's
/// words, with the remedy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct MaintenanceError(String);

impl MaintenanceError {
    /// The full message.
    pub fn message(&self) -> &str {
        &self.0
    }
}

/// The maintenance help, for `{bin} state <domain> --help`.
pub fn maintenance_help(frontend: Frontend) -> String {
    let bin = frontend.binary();
    format!(
        "usage:\n  {bin} state servers show [--json]\n  \
         {bin} state servers add --preset <name>\n  \
         {bin} state servers add --name <name> --revision <274|289|377> --transport <tcp|wss> \
         --vault <component> --world <num:host:port[:node[:asset_host:asset_port]]> \
         [--world ...] [--login-key <served|engine-dir[:path]|inline:mod:exp>] \
         [--members <true|false|auto>] [--allow-plaintext-offhost <true|false>]\n  \
         {bin} state servers edit <target> [--name <name>] [--revision <274|289|377>] \
         [--transport <tcp|wss>] [--vault <component>] [--world <spec>...] \
         [--login-key <...>] [--members <true|false|auto>] \
         [--allow-plaintext-offhost <true|false>]\n  \
         {bin} state servers remove <target>\n  \
         {bin} state preferences recover --source \"/absolute/path/private-valid.json\"\n  \
         {bin} state preferences recover --defaults\n  \
         {bin} state bank-hints accept-projection <profile>/<account>/<revision>/<content>\n  \
         {bin} state loadouts accept-projection operator-presets\n  \
         {bin} state vault recover --defaults --profile <server>\n\n\
         Stop every 274bot process for this root before mutating commands (add/edit/remove, \
         recover, accept-projection); they take the exclusive maintenance lease and refuse while \
         another process uses the root (for live roster edits, use the panel or TUI Servers \
         editor). `state servers show` makes no roster edits (on an uninitialized root it runs \
         the one-time legacy import or default seed first). The root is the one normal use \
         resolves (<home>/.274bot).\n\n\
         servers: manages LocalState's durable server roster. Vault bindings are permanent: a \
         bound server cannot rename its namespace or change its vault component, and another \
         server cannot reuse a bound vault component even after removal. Removing a server keeps \
         its binding and profile rows; re-adding it requires the same vault component (or add a \
         new server name and vault component). Edits apply on the next selection.\n\n\
         recover first preserves the whole database set (state.sqlite3, -wal and -shm) in a \
         private state/preserved-<time> directory, then starts a new preference history: its \
         version numbers restart and are not comparable with the preserved ones. --source imports \
         a private, valid JSON object; --defaults discards the saved preferences (they stay in \
         the preserved set).\n\n\
         accept-projection: a bank hint file with fields this build does not know is imported \
         read-only, and its original is kept as .migrated evidence. The command shows the hint, \
         its version, the evidence file and how many fields were omitted, and asks for \
         confirmation (a terminal is required) before later saves hold the usable rows only.\n\n\
         vault recover --defaults: when a converted vault has credentials without saved settings \
         and no paired backup is usable, it asks for the vault passphrase, shows how many rows \
         are missing, and after confirmation creates them with defaults and fresh uids.\n\n\
         0.2.9 preference export (state preferences export --same-root) is retired: this release \
         converts saved state one way.\n"
    )
}

fn parse_world_spec(spec: &str) -> Result<LaunchWorld, String> {
    let parts: Vec<&str> = spec.split(':').collect();
    let parse_u16 = |value: &str, label: &str| {
        value
            .parse::<u16>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("--world {label} must be an integer from 1 to 65535"))
    };
    match parts.as_slice() {
        [num, host, port] => {
            let number = parse_u16(num, "number")?;
            let port = parse_u16(port, "port")?;
            Ok(LaunchWorld {
                number,
                host: (*host).to_string(),
                port,
                node_id: i32::from(number),
                asset_host: (*host).to_string(),
                asset_port: port,
            })
        }
        [num, host, port, node] => {
            let number = parse_u16(num, "number")?;
            let port = parse_u16(port, "port")?;
            let node_id = node
                .parse::<i32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| "--world node_id must be a positive integer".to_string())?;
            Ok(LaunchWorld {
                number,
                host: (*host).to_string(),
                port,
                node_id,
                asset_host: (*host).to_string(),
                asset_port: port,
            })
        }
        [num, host, port, node, asset_host, asset_port] => {
            let number = parse_u16(num, "number")?;
            let port = parse_u16(port, "port")?;
            let node_id = node
                .parse::<i32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| "--world node_id must be a positive integer".to_string())?;
            let asset_port = parse_u16(asset_port, "asset_port")?;
            Ok(LaunchWorld {
                number,
                host: (*host).to_string(),
                port,
                node_id,
                asset_host: (*asset_host).to_string(),
                asset_port,
            })
        }
        _ => Err(format!(
            "invalid --world {spec:?}; expected number:host:port[:node_id[:asset_host:asset_port]]"
        )),
    }
}

fn parse_login_key_spec(spec: &str) -> Result<LoginKey, String> {
    if spec == "served" {
        return Ok(LoginKey::Named("served".into()));
    }
    if spec == "engine-dir" {
        return Ok(LoginKey::EngineDir { engine_dir: None });
    }
    if let Some(path) = spec.strip_prefix("engine-dir:") {
        let engine_dir = if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        };
        return Ok(LoginKey::EngineDir { engine_dir });
    }
    if let Some(rest) = spec.strip_prefix("inline:") {
        let Some((modulus, exponent)) = rest.split_once(':') else {
            return Err("--login-key inline requires inline:<modulus>:<exponent>".into());
        };
        if modulus.is_empty() || exponent.is_empty() {
            return Err("--login-key inline requires nonempty modulus and exponent".into());
        }
        return Ok(LoginKey::Inline {
            modulus: modulus.to_string(),
            exponent: exponent.to_string(),
        });
    }
    Err(format!(
        "invalid --login-key {spec:?}; expected served, engine-dir[:path], or inline:modulus:exponent"
    ))
}

fn parse_bool_flag(flag: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{flag} requires true or false")),
    }
}

fn parse_members_flag(value: &str) -> Result<Option<bool>, String> {
    match value {
        "true" => Ok(Some(true)),
        "false" => Ok(Some(false)),
        "auto" | "none" | "null" => Ok(None),
        _ => Err("--members requires true, false, or auto".into()),
    }
}

fn parse_server_fields(args: &[&str]) -> Result<ServerEditPatch, String> {
    let mut patch = ServerEditPatch::default();
    let mut worlds = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index];
        let value = *args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag {
            "--name" => patch.name = Some(value.to_string()),
            "--revision" => {
                let rev = value
                    .parse::<u16>()
                    .ok()
                    .filter(|r| matches!(r, 274 | 289 | 377))
                    .ok_or_else(|| {
                        format!("unsupported --revision {value:?}; use 274, 289, or 377")
                    })?;
                patch.revision = Some(rev);
            }
            "--transport" => {
                patch.transport = Some(match value {
                    "tcp" => LaunchTransport::Tcp,
                    "wss" => LaunchTransport::Wss,
                    _ => return Err(format!("invalid --transport {value:?}; use tcp or wss")),
                });
            }
            "--vault" => patch.vault = Some(value.to_string()),
            "--world" => worlds.push(parse_world_spec(value)?),
            "--login-key" => patch.login_key = Some(parse_login_key_spec(value)?),
            "--members" => patch.members = Some(parse_members_flag(value)?),
            "--allow-plaintext-offhost" => {
                patch.allow_plaintext_offhost = Some(parse_bool_flag(flag, value)?);
            }
            other => return Err(format!("unknown state servers flag {other:?}")),
        }
        index += 2;
    }
    if !worlds.is_empty() {
        patch.worlds = Some(worlds);
    }
    Ok(patch)
}

fn parse_servers_command(rest: &[&str], frontend: Frontend) -> Result<MaintenanceAction, String> {
    match rest {
        ["show"] => Ok(MaintenanceAction::ServersShow { json: false }),
        ["show", "--json"] => Ok(MaintenanceAction::ServersShow { json: true }),
        ["add", "--preset", name] => Ok(MaintenanceAction::ServersAddPreset((*name).to_string())),
        ["add", flags @ ..] => {
            if flags.contains(&"--preset") {
                return Err(format!(
                    "--preset cannot be combined with custom server fields\n{}",
                    maintenance_help(frontend)
                ));
            }
            let patch = parse_server_fields(flags)?;
            let name = patch.name.ok_or_else(|| {
                "state servers add requires --name <name> (or --preset <name>)".to_string()
            })?;
            let revision = patch
                .revision
                .ok_or_else(|| "state servers add requires --revision <274|289|377>".to_string())?;
            let transport = patch
                .transport
                .ok_or_else(|| "state servers add requires --transport <tcp|wss>".to_string())?;
            let vault = patch
                .vault
                .ok_or_else(|| "state servers add requires --vault <component>".to_string())?;
            let worlds = patch.worlds.ok_or_else(|| {
                "state servers add requires at least one --world <spec>".to_string()
            })?;
            let login_key = patch.login_key.unwrap_or_else(|| match transport {
                LaunchTransport::Wss => LoginKey::Named("served".into()),
                LaunchTransport::Tcp => LoginKey::EngineDir { engine_dir: None },
            });
            Ok(MaintenanceAction::ServersAddCustom(LaunchProfile {
                name,
                revision,
                transport,
                worlds,
                login_key,
                vault,
                members: patch.members.flatten(),
                allow_plaintext_offhost: patch.allow_plaintext_offhost.unwrap_or(false),
            }))
        }
        ["edit", target, flags @ ..] if !target.starts_with("--") => {
            let patch = parse_server_fields(flags)?;
            if patch.is_empty() {
                return Err(format!(
                    "state servers edit requires at least one field flag\n{}",
                    maintenance_help(frontend)
                ));
            }
            Ok(MaintenanceAction::ServersEdit {
                target: (*target).to_string(),
                patch,
            })
        }
        ["remove", target] if !target.starts_with("--") => {
            Ok(MaintenanceAction::ServersRemove((*target).to_string()))
        }
        _ => Err(format!(
            "invalid state servers command\n{}",
            maintenance_help(frontend)
        )),
    }
}

/// Parse a maintenance command line (the arguments after the binary
/// name). `None` when the arguments are not a maintenance command (they do
/// not start with `state`); `Some(Err)` with the usage error for a bad or
/// combined command, before any root is opened.
pub fn parse_maintenance(
    args: &[String],
    frontend: Frontend,
) -> Option<Result<MaintenanceAction, String>> {
    let first = args.first()?;
    if first != "state" {
        return None;
    }
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    Some(match words.as_slice() {
        ["state", "servers", rest @ ..] => parse_servers_command(rest, frontend),
        ["state", "preferences", "export", ..] => Err(format!(
            "state preferences export was retired: this release converts saved state one way, \
             and an edit 0.2.9 makes is not imported back. Use a private paired backup of the \
             vault and its saved state instead.\n{}",
            maintenance_help(frontend)
        )),
        ["state", "preferences", "recover", "--source", source] => {
            Ok(MaintenanceAction::RecoverSource(PathBuf::from(source)))
        }
        ["state", "preferences", "recover", "--defaults"] => Ok(MaintenanceAction::RecoverDefaults),
        ["state", "bank-hints", "accept-projection", key] => {
            Ok(MaintenanceAction::AcceptBankProjection((*key).to_string()))
        }
        ["state", "loadouts", "accept-projection", key] => Ok(
            MaintenanceAction::AcceptLoadoutProjection((*key).to_string()),
        ),
        ["state", "vault", "recover", "--defaults", "--profile", server] => Ok(
            MaintenanceAction::RecoverVaultDefaults((*server).to_string()),
        ),
        _ => Err(format!(
            "not one maintenance command (one action per run; recoveries cannot be \
             combined)\n{}",
            maintenance_help(frontend)
        )),
    })
}

/// The shipped entry point: run a maintenance command line at the root
/// normal use resolves, print its report or refusal, and give the exit
/// code. `None` when `args` is not a maintenance command line. Called
/// before any frontend, vault or Play startup.
pub fn maintenance_command(frontend: Frontend, args: &[String]) -> Option<i32> {
    use std::io::Write as _;
    if args.len() >= 3
        && args[0] == "state"
        && matches!(
            args[1].as_str(),
            "preferences" | "bank-hints" | "loadouts" | "vault" | "servers"
        )
        && matches!(args[2].as_str(), "--help" | "-h" | "help")
    {
        let _ = write!(std::io::stdout(), "{}", maintenance_help(frontend));
        return Some(0);
    }
    let action = match parse_maintenance(args, frontend)? {
        Ok(action) => action,
        Err(usage) => {
            let _ = writeln!(std::io::stderr(), "{}: {usage}", frontend.binary());
            return Some(2);
        }
    };
    let root = StateRoot::new(script::bot_home().join(".274bot"));
    let result = match &action {
        MaintenanceAction::RecoverVaultDefaults(server) => crate::passphrase::obtain(
            frontend.binary(),
            false,
            crate::passphrase::Purpose::for_vault(true),
        )
        .map_err(MaintenanceError)
        .and_then(|passphrase| {
            run_vault_recover(
                root.path(),
                frontend,
                server,
                &passphrase,
                &confirm_on_terminal,
            )
        }),
        action => run_maintenance(&root, frontend, action, &confirm_on_terminal),
    };
    Some(match result {
        Ok(report) => {
            let _ = writeln!(std::io::stdout(), "{report}");
            0
        }
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "{error}");
            1
        }
    })
}

/// `state vault recover --defaults --profile <server>` at `root`: under the
/// exclusive root lease, resolve the server's configured vault component
/// from the saved roster, authenticate the vault with `passphrase`, and
/// after `confirm` create only its missing settings rows with defaults and
/// fresh uids. A pending split, a mismatched binding or an unconverted
/// vault is refused unchanged.
pub fn run_vault_recover(
    root: &Path,
    frontend: Frontend,
    server: &str,
    passphrase: &str,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, MaintenanceError> {
    let state_root = StateRoot::new(root.to_path_buf());
    let display = root.display().to_string();
    let action = MaintenanceAction::RecoverVaultDefaults(server.to_string());
    let Some(lease) = RootUse::exclusive(&state_root).map_err(|error| {
        MaintenanceError(format!(
            "Cannot recover the vault rows at {display:?}: the root-use lock could not be taken \
             ({error})."
        ))
    })?
    else {
        return Err(MaintenanceError(lease_conflict(
            &display, frontend, &action,
        )));
    };
    let service = super::LocalStateService::open_process_with(
        state_root.clone(),
        frontend,
        Some(lease.clone()),
    );
    let result = (|| {
        let roster = service
            .wait_server_roster(std::time::Duration::from_secs(30))
            .map_err(|error| MaintenanceError(error.to_string()))?;
        let profile = roster.servers().resolve(server).map_err(MaintenanceError)?;
        let target = crate::profile_vault::VaultTarget::new(
            &profile.name,
            &state_root.path().join(&profile.vault),
        )
        .map_err(|error| MaintenanceError(format!("vault {}: {error}", profile.vault)))?;
        crate::profile_vault::recover_defaults(&service, &target, passphrase, confirm)
            .map_err(|error| MaintenanceError(error.to_string()))
    })();
    drop(service);
    drop(lease);
    result
}

/// Run `action` at `root` for the `frontend` binary. `Ok` is the report
/// the command prints. An action that needs the operator's confirmation
/// (accepting a bank hint's projection) is refused: use
/// [`run_maintenance`] with a way to ask.
pub fn run_preferences_maintenance(
    root: &StateRoot,
    frontend: Frontend,
    action: &MaintenanceAction,
) -> Result<String, MaintenanceError> {
    run_maintenance(root, frontend, action, &|_| false)
}

/// [`run_preferences_maintenance`] with `confirm`, the operator's answer to
/// the question an action asks before it changes anything.
pub fn run_maintenance(
    root: &StateRoot,
    frontend: Frontend,
    action: &MaintenanceAction,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, MaintenanceError> {
    match action {
        MaintenanceAction::ServersShow { json } => run_servers_show(root, frontend, *json),
        MaintenanceAction::ServersAddPreset(_)
        | MaintenanceAction::ServersAddCustom(_)
        | MaintenanceAction::ServersEdit { .. }
        | MaintenanceAction::ServersRemove(_) => run_servers_mutation(root, frontend, action),
        _ => run(
            root,
            frontend,
            action,
            sql::Db::new(root.clone(), frontend, None),
            confirm,
        ),
    }
}

fn format_login_key(key: &LoginKey) -> String {
    match key {
        LoginKey::Named(name) => name.clone(),
        LoginKey::EngineDir { engine_dir: None } => "engine-dir".into(),
        LoginKey::EngineDir {
            engine_dir: Some(path),
        } => format!("engine-dir:{}", path.display()),
        LoginKey::Inline { exponent, .. } => format!("inline(exp={exponent})"),
    }
}

fn run_servers_show(
    root: &StateRoot,
    frontend: Frontend,
    as_json: bool,
) -> Result<String, MaintenanceError> {
    let Some(lease) = RootUse::shared(root).map_err(|error| {
        MaintenanceError(format!(
            "Cannot read the server roster at {:?}: the root-use lock could not be taken ({error}).",
            root.path().display()
        ))
    })?
    else {
        return Err(MaintenanceError(format!(
            "Cannot read the server roster at {:?}: another 274bot process holds an exclusive \
             lease on this root. Stop that process or inspect the roster in the running app.",
            root.path().display()
        )));
    };
    let service =
        super::LocalStateService::open_process_with(root.clone(), frontend, Some(lease.clone()));
    let roster = service
        .wait_server_roster(std::time::Duration::from_secs(30))
        .map_err(|error| MaintenanceError(error.to_string()))?;
    let available = roster.servers().available_presets();
    let output = if as_json {
        serde_json::to_string_pretty(&serde_json::json!({
            "version": roster.version().get(),
            "servers": roster.servers().servers,
            "available_presets": available.iter().map(|preset| &preset.name).collect::<Vec<_>>(),
        }))
        .map_err(|error| MaintenanceError(error.to_string()))?
    } else {
        let mut out = format!("server roster version {}\n", roster.version().get());
        for profile in &roster.servers().servers {
            let transport = match profile.transport {
                LaunchTransport::Tcp => "tcp",
                LaunchTransport::Wss => "wss",
            };
            let members = match profile.members {
                Some(true) => "true",
                Some(false) => "false",
                None => "auto",
            };
            let worlds = profile
                .worlds
                .iter()
                .map(|w| {
                    format!(
                        "w{} {}:{} (node {}, assets {}:{})",
                        w.number, w.host, w.port, w.node_id, w.asset_host, w.asset_port
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "  {}  rev={} transport={} vault={} members={} login_key={} plaintext={} worlds=[{}]\n",
                profile.name,
                profile.revision,
                transport,
                profile.vault,
                members,
                format_login_key(&profile.login_key),
                profile.allow_plaintext_offhost,
                worlds
            ));
        }
        if !available.is_empty() {
            let names = available
                .iter()
                .map(|preset| preset.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("available shipped presets: {names}\n"));
        }
        out.trim_end_matches('\n').to_string()
    };
    drop(service);
    drop(lease);
    Ok(output)
}

fn run_servers_mutation(
    root: &StateRoot,
    frontend: Frontend,
    action: &MaintenanceAction,
) -> Result<String, MaintenanceError> {
    let display = root.path().display().to_string();
    let Some(lease) = RootUse::exclusive(root).map_err(|error| {
        MaintenanceError(format!(
            "Cannot edit the server roster at {display:?}: the root-use lock could not be taken \
             ({error})."
        ))
    })?
    else {
        return Err(MaintenanceError(lease_conflict(&display, frontend, action)));
    };
    let service =
        super::LocalStateService::open_process_with(root.clone(), frontend, Some(lease.clone()));
    let timeout = std::time::Duration::from_secs(30);
    let result = (|| {
        let roster = service
            .wait_server_roster(timeout)
            .map_err(|error| MaintenanceError(error.to_string()))?;
        let old_version = roster.version();
        let edit = match action {
            MaintenanceAction::ServersAddPreset(name) => {
                super::ServerRosterEdit::AddPreset(name.clone())
            }
            MaintenanceAction::ServersAddCustom(profile) => {
                super::ServerRosterEdit::Add(profile.clone())
            }
            MaintenanceAction::ServersEdit { target, patch } => {
                let existing = roster
                    .servers()
                    .servers
                    .iter()
                    .find(|profile| profile.name.eq_ignore_ascii_case(target))
                    .ok_or_else(|| {
                        MaintenanceError(format!(
                            "Unknown server profile {target:?} in the durable server roster"
                        ))
                    })?;
                super::ServerRosterEdit::Edit {
                    target: target.clone(),
                    profile: patch.apply(existing),
                }
            }
            MaintenanceAction::ServersRemove(target) => super::ServerRosterEdit::Remove {
                target: target.clone(),
            },
            _ => unreachable!("non-server action dispatched to run_servers_mutation"),
        };
        let label = edit.action_label();
        let target = edit.target_label().to_string();
        let updated = service
            .edit_server_roster(old_version, edit, timeout)
            .map_err(|error| MaintenanceError(error.to_string()))?;
        Ok(format!(
            "server roster {label} {target:?} committed (version {} -> {}, {} profiles). \
             Edits take effect on next selection.",
            old_version.get(),
            updated.version().get(),
            updated.servers().servers.len()
        ))
    })();
    drop(service);
    drop(lease);
    result
}

/// Ask on the terminal: `true` only for the answer `yes` typed on a
/// terminal. A command run without a terminal never confirms.
fn confirm_on_terminal(question: &str) -> bool {
    use std::io::{BufRead as _, IsTerminal as _, Write as _};
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return false;
    }
    let _ = write!(std::io::stderr(), "{question}\nType yes to continue: ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    stdin.lock().read_line(&mut answer).is_ok() && answer.trim() == "yes"
}

fn run(
    root: &StateRoot,
    frontend: Frontend,
    action: &MaintenanceAction,
    mut db: sql::Db,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, MaintenanceError> {
    let display = root.path().display().to_string();
    let Some(lease) = RootUse::exclusive(root).map_err(|error| {
        MaintenanceError(format!(
            "Cannot export or recover preferences at {display:?}: the root-use lock could not \
             be taken ({error})."
        ))
    })?
    else {
        return Err(MaintenanceError(lease_conflict(&display, frontend, action)));
    };
    let guard = files::try_lock_for_write(&root.preferences_file()).map_err(|error| {
        MaintenanceError(format!(
            "Cannot export or recover preferences at {display:?}: the preferences guard could \
             not be taken ({error})."
        ))
    })?;
    let Some(guard) = guard else {
        return Err(MaintenanceError(lease_conflict(&display, frontend, action)));
    };
    let result = match action {
        MaintenanceAction::RecoverVaultDefaults(server) => Err(MaintenanceError(format!(
            "state vault recover --profile {server} asks for the vault passphrase on a terminal."
        ))),
        MaintenanceAction::ServersShow { .. }
        | MaintenanceAction::ServersAddPreset(_)
        | MaintenanceAction::ServersAddCustom(_)
        | MaintenanceAction::ServersEdit { .. }
        | MaintenanceAction::ServersRemove(_) => {
            unreachable!("server actions dispatch directly in run_maintenance")
        }
        MaintenanceAction::RecoverSource(source) => recover(&mut db, root, Some(source), &guard),
        MaintenanceAction::RecoverDefaults => recover(&mut db, root, None, &guard),
        MaintenanceAction::AcceptBankProjection(name) => {
            accept_bank_projection(&mut db, root, name, &guard, confirm)
        }
        MaintenanceAction::AcceptLoadoutProjection(name) => {
            accept_loadout_projection(&mut db, root, name, &guard, confirm)
        }
    };
    db.close();
    drop(guard);
    drop(lease);
    result
}

/// Accept the read-only projection of one imported bank hint (see
/// [`MaintenanceAction::AcceptBankProjection`]): show what is read, ask, and
/// only then clear the refusal. Nothing changes without the evidence kept
/// beside the source and the operator's yes.
fn accept_bank_projection(
    db: &mut sql::Db,
    root: &StateRoot,
    name: &str,
    guard: &files::WriteLock,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, MaintenanceError> {
    let Some(key) = bank::BankKey::parse(name) else {
        return Err(MaintenanceError(format!(
            "{name:?} is not a bank hint key. Name it as <profile>/<account>/<revision>/<content>, \
             for example local-289/alice/289/lostcity-289. Nothing was changed."
        )));
    };
    let opened = db.open_for_maintenance(guard).map_err(refusal)?;
    if !opened {
        return Err(MaintenanceError(
            "There is no state database in this root; nothing was changed.".to_string(),
        ));
    }
    let Some((row, ledger)) = db.bank_report(&key).map_err(refusal)? else {
        return Err(MaintenanceError(format!(
            "There is no stored bank hint {name}; nothing was changed. Names are listed in the \
             log line 'bank-hints hint ... has N omitted field(s)'."
        )));
    };
    if row.omitted == 0 {
        return Err(MaintenanceError(format!(
            "The bank hint {name} has no read-only projection to accept; nothing was changed."
        )));
    }
    let evidence = key.evidence_file(root, ledger.digest).ok_or_else(|| {
        MaintenanceError(format!(
            "The original of the bank hint {name} is not kept as a .migrated file beside \
             {}, so its omitted fields cannot be accepted away. Restore the original there, \
             or leave the hint read-only. Nothing was changed.",
            root.bank_hint_file(&key.profile, &key.account).display()
        ))
    })?;
    let question = format!(
        "Bank hint {name} is read-only at {}: {} usable row(s), {} omitted field(s) kept in \
         the original {}. Accepting means every later save of this hint holds the usable \
         rows only, and the omitted fields stay only in that file.",
        row.version,
        row.snapshot.as_bank_hints().map_or(0, <[_]>::len),
        row.omitted,
        evidence.display()
    );
    if !confirm(&question) {
        return Err(MaintenanceError(format!(
            "{question} Not confirmed (a terminal and the answer yes are required); nothing was \
             changed."
        )));
    }
    db.accept_bank_projection(guard, &key, row.version)
        .map_err(refusal)?;
    host_log!(
        Category::Lifecycle,
        Level::Info,
        "audit domain=bank-hints profile={} account={} action=bank-hint-accept-projection \
         origin=maintenance from={} omitted={} evidence={}",
        key.profile,
        key.account,
        row.version,
        row.omitted,
        evidence.display()
    );
    Ok(format!(
        "Accepted the projection of bank hint {name}: later saves hold its {} usable row(s) only. \
         The original stays in {}.",
        row.snapshot.as_bank_hints().map_or(0, <[_]>::len),
        evidence.display()
    ))
}

fn accept_loadout_projection(
    db: &mut sql::Db,
    root: &StateRoot,
    name: &str,
    guard: &files::WriteLock,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, MaintenanceError> {
    if !db.open_for_maintenance(guard).map_err(refusal)? {
        return Err(MaintenanceError(
            "There is no state database. Nothing was changed.".into(),
        ));
    }
    let Some((cached, ledger)) = db.loadout_report(name).map_err(refusal)? else {
        return Err(MaintenanceError(format!("No imported loadout projection exists for {name:?}. Use operator-presets. Nothing was changed.")));
    };
    let (unreadable, lossy) = match cached.rewrite {
        files::Rewrite::Lossless => {
            return Err(MaintenanceError(format!(
                "Loadouts {name} have no read-only projection. Nothing was changed."
            )))
        }
        files::Rewrite::Refused { unreadable, lossy } => (unreadable, lossy),
    };
    let evidence = super::loadouts::projection_evidence(root, ledger.digest).ok_or_else(|| MaintenanceError(
        "The private loadout evidence is missing. Restore the .migrated evidence or recover from a repaired source. Nothing was changed.".into()
    ))?;
    let question = format!(
        "Loadouts {name} at {} are read-only: {} usable row(s), {unreadable} unreadable row(s), {lossy} lossy row(s). Complete evidence: {}. Accepting means future saves contain only the usable data.",
        cached.version, cached.snapshot.as_loadouts().map_or(0, <[_]>::len), evidence.display()
    );
    if !confirm(&question) {
        return Err(MaintenanceError(format!(
            "{question} Not confirmed. Nothing was changed."
        )));
    }
    let version = db
        .accept_loadout_projection(guard, name, cached.version)
        .map_err(refusal)?;
    host_log!(Category::Lifecycle, Level::Info,
        "audit domain=loadouts-references key={name} action=loadout-accept-projection origin=maintenance old={} new={version} unreadable={unreadable} lossy={lossy} evidence={}",
        cached.version, evidence.display());
    Ok(format!("Accepted loadouts {name} at {version}. Future saves contain only usable data. Evidence stays in {}.", evidence.display()))
}

fn recover(
    db: &mut sql::Db,
    root: &StateRoot,
    source: Option<&PathBuf>,
    guard: &files::WriteLock,
) -> Result<String, MaintenanceError> {
    let (body, digest, kind, from) = match source {
        Some(source) => {
            let display = source.display().to_string();
            if !source.is_absolute() {
                return Err(MaintenanceError(format!(
                    "Preferences refused at {display:?}: the source must be an absolute path. \
                     Nothing was changed."
                )));
            }
            match sql::read_legacy(source) {
                Legacy::Valid {
                    bytes, canonical, ..
                } => (
                    canonical,
                    Some(files::digest(&bytes)),
                    "recovered-source",
                    display,
                ),
                Legacy::Absent => {
                    return Err(MaintenanceError(format!(
                        "Preferences refused at {display:?}: there is no file there. Nothing \
                         was changed."
                    )))
                }
                Legacy::Malformed(_) => {
                    return Err(MaintenanceError(format!(
                        "Preferences refused at {display:?}: the file is not a JSON object of \
                         preferences. Nothing was changed. Name a private, valid JSON object."
                    )))
                }
                Legacy::Refused(failure) => {
                    return Err(refusal(db.source_failure(
                        &display,
                        failure,
                        sql::SourceUse::Recovery,
                    )));
                }
            }
        }
        None => (
            files::preferences_bytes(&Map::new())
                .map_err(|error| MaintenanceError(error.to_string()))?,
            None,
            "recovered-defaults",
            "defaults".to_string(),
        ),
    };
    // An explicit preferences reset must not reset the other domains.
    let carry = db.read_state_carry();
    let preserved = preserve(root).map_err(|error| {
        MaintenanceError(format!(
            "Recovery stopped at {:?}: the old database files could not be preserved ({error}). \
             Nothing was removed or replaced.",
            root.state_dir().display().to_string()
        ))
    })?;
    // The file at `panel-ui.json`, whatever it holds, is what this recovery
    // accounts for: the next start must not take it for a later outside
    // edit (it may be the very over-limit file that caused the refusal).
    let accounted = sql::legacy_file_digest(&root.preferences_file()).or(digest);
    let version = db
        .start_new_history(guard, &body, accounted, kind, &from)
        .map_err(refusal)?;
    let carry_note = match carry {
        Ok(carry) => {
            db.restore_state_carry(guard, carry.as_ref())
                .map_err(refusal)?;
            match carry {
                Some(carry) => {
                    format!(
                    "{} bank hint(s) were carried over unchanged, along with {} operator card(s) \
                     and {} runtime settings store(s)",
                    carry.hints(), carry.operator_cards(), carry.runtime_stores()
                )
                }
                None => "there were no other stores to carry over".to_string(),
            }
        }
        Err(failure) => {
            db.mark_state_sources_unknown(guard).map_err(refusal)?;
            let why = match failure {
                SqlFailure::Wait(cause) => cause.reason().to_string(),
                SqlFailure::Refused { detail, .. } => detail.to_string(),
            };
            format!(
                "the other stores could not be read from the old database ({why}); they stay in \
                 the preserved set. No script settings file and no legacy bank hint file is imported again"
            )
        }
    };
    let kept = preserved
        .as_ref()
        .map(|dir| format!("the old database set is preserved in {}", dir.display()))
        .unwrap_or_else(|| "there was no old database set to preserve".into());
    host_log!(
        Category::Lifecycle,
        Level::Info,
        "{}",
        sql::audit_line(
            &format!("preferences-{kind}"),
            StoreVersion::INITIAL,
            version,
            &format!("preferences history restarted from {from}; {kept}"),
        )
    );
    Ok(format!(
        "Preferences recovered from {from} at {version}: preferences history restarted; \
         {kept}; {carry_note}."
    ))
}

/// Create `dir` new (an existing one is `AlreadyExists`) owner-only from the
/// start where the platform has modes, whatever the process umask (a umask
/// only removes bits). The preserved copies of the database never sit in a
/// directory others can enter, not even before the first one is written.
fn create_new_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(dir)
    }
}

/// Keep a private, verified copy of every existing file of the database
/// set in a new `preserved-<time>` directory, sync it, then remove the
/// originals. No connection is open in this process, and the exclusive
/// lease keeps every other user out. `None` when there was no set.
fn preserve(root: &StateRoot) -> io::Result<Option<PathBuf>> {
    let set: Vec<PathBuf> = sql::database_set(root)
        .into_iter()
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .collect();
    if set.is_empty() {
        return Ok(None);
    }
    let state = root.state_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut dir = None;
    for attempt in 0..100u32 {
        let candidate = state.join(match attempt {
            0 => format!("preserved-{stamp}"),
            n => format!("preserved-{stamp}-{n}"),
        });
        match create_new_private_dir(&candidate) {
            Ok(()) => {
                dir = Some(candidate);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let dir = dir.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free preserved-<time> name",
        )
    })?;
    for path in &set {
        let bytes = fs::read(path)?;
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?;
        let copy = dir.join(name);
        vault::create_private_file(&copy, &bytes)?;
        let held = fs::read(&copy)?;
        if files::digest(&held) != files::digest(&bytes) {
            return Err(io::Error::other(format!(
                "{}: the copy does not hold the file's bytes",
                copy.display()
            )));
        }
        fs::File::open(&copy)?.sync_all()?;
    }
    files::sync_directory(&dir)?;
    files::sync_directory(&state)?;
    for path in &set {
        fs::remove_file(path)?;
    }
    files::sync_directory(&state)?;
    Ok(Some(dir))
}

fn refusal(failure: SqlFailure) -> MaintenanceError {
    match failure {
        SqlFailure::Refused { detail, .. } => MaintenanceError(detail.to_string()),
        SqlFailure::Wait(cause) => MaintenanceError(format!(
            "{} Stop every 274bot process for this root, then retry the same maintenance \
             command.",
            cause.text()
        )),
    }
}

/// The maintenance lease conflict (plan §5.5).
fn lease_conflict(root: &str, frontend: Frontend, action: &MaintenanceAction) -> String {
    let bin = frontend.binary();
    if matches!(
        action,
        MaintenanceAction::ServersAddPreset(_)
            | MaintenanceAction::ServersAddCustom(_)
            | MaintenanceAction::ServersEdit { .. }
            | MaintenanceAction::ServersRemove(_)
    ) {
        return format!(
            "Cannot edit the server roster at {root:?}: another 274bot process holds the root \
             lease. Stop the running app and retry the CLI command, or edit the roster directly \
             in the running panel or TUI Servers editor."
        );
    }
    let mut text = format!(
        "Cannot export or recover preferences at {root:?}: another new-version process uses \
         this root. Stop that process. Retry the same maintenance command."
    );
    match action {
        MaintenanceAction::RecoverVaultDefaults(server) => {
            text.push_str(&format!(
                " Run '{bin} state vault recover --defaults --profile {server}' again."
            ));
        }
        MaintenanceAction::RecoverSource(_) | MaintenanceAction::RecoverDefaults => {
            text.push_str(&format!(
                " Run '{bin} state preferences recover --source \"<private-valid-json>\"' or \
                 '{bin} state preferences recover --defaults'."
            ));
        }
        MaintenanceAction::AcceptBankProjection(name) => {
            text.push_str(&format!(
                " Run '{bin} state bank-hints accept-projection {name}' again."
            ));
        }
        MaintenanceAction::AcceptLoadoutProjection(name) => {
            text.push_str(&format!(
                " Run '{bin} state loadouts accept-projection {name}' again."
            ));
        }
        MaintenanceAction::ServersShow { .. }
        | MaintenanceAction::ServersAddPreset(_)
        | MaintenanceAction::ServersAddCustom(_)
        | MaintenanceAction::ServersEdit { .. }
        | MaintenanceAction::ServersRemove(_) => {}
    }
    text
}
