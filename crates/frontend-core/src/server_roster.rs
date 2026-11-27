//! Shared non-blocking server-roster editor state and form parsers for both
//! frontends (S9e, C6 parity).
//!
//! Every mutation flows through [`host_play::local_state::LocalStateService::submit_server_roster_edit`]
//! on the single SQLite writer thread and settles asynchronously inside
//! [`crate::OperatorSession::poll`]. Neither panel nor TUI ever blocks a frame
//! or input loop on SQLite I/O.

use std::path::PathBuf;

use api::identity::RequestId;
use api::local_state::StoreVersion;

pub use host_play::local_state::{
    ServerRoster, ServerRosterEdit, ServerRosterError, ServerRosterMutation,
    ServerRosterMutationError,
};
pub use host_play::{LaunchProfile, LaunchTransport, LaunchWorld, LoginKey, Servers};

/// An in-flight server roster edit awaiting writer settlement.
pub(crate) struct PendingServerRoster {
    pub(crate) request: RequestId,
    pub(crate) action: &'static str,
    pub(crate) target: String,
    pub(crate) previous: StoreVersion,
    pub(crate) mutation: ServerRosterMutation,
}

/// Shared non-blocking server-roster editor state owned by [`crate::OperatorSession`].
#[derive(Default)]
pub(crate) struct ServerRosterDesk {
    pub(crate) snapshot: Option<ServerRoster>,
    pub(crate) pending: Option<PendingServerRoster>,
    pub(crate) staged_add: Option<LaunchProfile>,
    pub(crate) staged_preset: Option<String>,
    pub(crate) staged_edit: Option<(String, LaunchProfile)>,
    pub(crate) staged_remove: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) generation: u64,
}

impl ServerRosterDesk {
    pub(crate) fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Adopt a newer cached snapshot from `local_state` without blocking.
    pub(crate) fn adopt_snapshot(&mut self, roster: ServerRoster) {
        let changed = self
            .snapshot
            .as_ref()
            .is_none_or(|current| current.version() != roster.version());
        if changed {
            self.snapshot = Some(roster);
            self.bump();
        }
    }
}

/// All shipped built-in server presets (kept separate from any saved roster).
pub fn shipped_presets() -> Vec<LaunchProfile> {
    Servers::shipped_presets()
}

/// Shipped presets that are not yet present in `roster`.
pub fn available_presets(roster: &ServerRoster) -> Vec<LaunchProfile> {
    roster.servers().available_presets()
}

/// Format worlds as compact `number:host:port:node_id:asset_host:asset_port` lines.
pub fn format_worlds_text(worlds: &[LaunchWorld]) -> String {
    worlds
        .iter()
        .map(|w| {
            format!(
                "{}:{}:{}:{}:{}:{}",
                w.number, w.host, w.port, w.node_id, w.asset_host, w.asset_port
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse comma- or newline-separated world specs:
/// `number:host:port[:node_id[:asset_host:asset_port]]`.
pub fn parse_worlds_text(text: &str) -> Result<Vec<LaunchWorld>, String> {
    let mut worlds = Vec::new();
    for raw in text.split([',', '\n']) {
        let spec = raw.trim();
        if spec.is_empty() {
            continue;
        }
        worlds.push(parse_one_world(spec)?);
    }
    if worlds.is_empty() {
        return Err("at least one world is required (number:host:port)".into());
    }
    Ok(worlds)
}

fn parse_one_world(spec: &str) -> Result<LaunchWorld, String> {
    let parts: Vec<&str> = spec.split(':').collect();
    let parse_u16 = |value: &str, label: &str| {
        value
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("world {label} must be 1..=65535"))
    };
    match parts.as_slice() {
        [num, host, port] => {
            let number = parse_u16(num, "number")?;
            let port = parse_u16(port, "port")?;
            let host = host.trim().to_string();
            Ok(LaunchWorld {
                number,
                asset_host: host.clone(),
                asset_port: port,
                host,
                port,
                node_id: i32::from(number),
            })
        }
        [num, host, port, node] => {
            let number = parse_u16(num, "number")?;
            let port = parse_u16(port, "port")?;
            let node_id = node
                .trim()
                .parse::<i32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| "world node_id must be a positive integer".to_string())?;
            let host = host.trim().to_string();
            Ok(LaunchWorld {
                number,
                asset_host: host.clone(),
                asset_port: port,
                host,
                port,
                node_id,
            })
        }
        [num, host, port, node, asset_host, asset_port] => {
            let number = parse_u16(num, "number")?;
            let port = parse_u16(port, "port")?;
            let node_id = node
                .trim()
                .parse::<i32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| "world node_id must be a positive integer".to_string())?;
            let asset_port = parse_u16(asset_port, "asset_port")?;
            Ok(LaunchWorld {
                number,
                host: host.trim().to_string(),
                port,
                node_id,
                asset_host: asset_host.trim().to_string(),
                asset_port,
            })
        }
        _ => Err(format!(
            "invalid world {spec:?}; use number:host:port[:node_id[:asset_host:asset_port]]"
        )),
    }
}

/// Format a [`LoginKey`] for compact editing in panel and TUI forms.
pub fn format_login_key_text(key: &LoginKey) -> String {
    match key {
        LoginKey::Named(name) => name.clone(),
        LoginKey::EngineDir { engine_dir: None } => "engine-dir".into(),
        LoginKey::EngineDir {
            engine_dir: Some(path),
        } => format!("engine-dir:{}", path.display()),
        LoginKey::Inline { modulus, exponent } => format!("inline:{modulus}:{exponent}"),
    }
}

/// Parse a [`LoginKey`] from `served`, `engine-dir[:path]`, or `inline:modulus:exponent`.
pub fn parse_login_key_text(text: &str) -> Result<LoginKey, String> {
    let spec = text.trim();
    if spec == "served" {
        return Ok(LoginKey::Named("served".into()));
    }
    if spec == "engine-dir" {
        return Ok(LoginKey::EngineDir { engine_dir: None });
    }
    if let Some(path) = spec.strip_prefix("engine-dir:") {
        let trimmed = path.trim();
        let engine_dir = if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        };
        return Ok(LoginKey::EngineDir { engine_dir });
    }
    if let Some(rest) = spec.strip_prefix("inline:") {
        let Some((modulus, exponent)) = rest.split_once(':') else {
            return Err("inline login key requires inline:<modulus>:<exponent>".into());
        };
        if modulus.trim().is_empty() || exponent.trim().is_empty() {
            return Err("inline login key requires nonempty modulus and exponent".into());
        }
        return Ok(LoginKey::Inline {
            modulus: modulus.trim().to_string(),
            exponent: exponent.trim().to_string(),
        });
    }
    Err(format!(
        "invalid login key {spec:?}; use served, engine-dir[:path], or inline:modulus:exponent"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandFailure, Completion, FailureKind, OperatorCommand};
    use crate::{OperatorSession, Targets};
    use api::identity::Settlement;
    use host_play::local_state::{LocalStateService, StateRoot};
    use host_play::InstancePermit;
    use script::IsolatedEnv;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    fn sample_profile(name: &str, vault: &str) -> LaunchProfile {
        LaunchProfile {
            name: name.into(),
            revision: 289,
            transport: LaunchTransport::Tcp,
            worlds: vec![LaunchWorld {
                number: 1,
                host: "127.0.0.1".into(),
                port: 45594,
                node_id: 1,
                asset_host: "127.0.0.1".into(),
                asset_port: 1080,
            }],
            login_key: LoginKey::EngineDir { engine_dir: None },
            vault: vault.into(),
            members: None,
            allow_plaintext_offhost: false,
        }
    }

    fn poll_until_settled(session: &mut OperatorSession<()>, request: api::identity::RequestId) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            session.poll();
            if session
                .commands()
                .report(request)
                .is_some_and(|report| report.members().iter().all(|m| m.terminal().is_some()))
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("server roster command did not settle within 5s");
    }

    #[test]
    fn frame_thread_submit_and_poll_never_block_while_sqlite_owner_is_stalled() {
        let env = IsolatedEnv::enter("fc-roster-nonblock");
        let root = StateRoot::new(env.home.join(".274bot"));
        vault::create_private_dir(root.path()).unwrap();
        let service = LocalStateService::open_process(root);
        let _initial = service.wait_server_roster(Duration::from_secs(5)).unwrap();

        let mut session = OperatorSession::<()>::new(InstancePermit::SkipLock);
        session.bind_local_state(Arc::clone(&service));
        session.poll();

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(parking_lot::Mutex::new(release_rx));
        let fired = Arc::new(AtomicBool::new(false));
        service.set_write_stall_for_test(Arc::new(move || {
            if !fired.swap(true, Ordering::SeqCst) {
                let _ = entered_tx.send(());
                let _ = release_rx.lock().recv();
            }
        }));

        session.stage_add_server(sample_profile("nonblock-srv", "vault-nonblock"));
        let started = Instant::now();
        let submitted = session
            .submit_detached(OperatorCommand::AddServer, Targets::Application)
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "submit blocked the frame thread: {:?}",
            started.elapsed()
        );

        // Wait until the SQLite writer thread is genuinely stalled inside mutate_servers.
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("writer entered stall");

        // Frame polls must return immediately while the SQLite writer is blocked.
        for _ in 0..5 {
            let poll_start = Instant::now();
            session.poll();
            assert!(
                poll_start.elapsed() < Duration::from_millis(200),
                "poll blocked the frame thread while SQLite owner stalled: {:?}",
                poll_start.elapsed()
            );
            assert!(session.server_roster_pending());
        }

        // Release the SQLite writer and verify clean async settlement.
        let _ = release_tx.send(());
        poll_until_settled(&mut session, submitted.request());
        assert!(!session.server_roster_pending());
        assert_eq!(
            session
                .commands()
                .report(submitted.request())
                .unwrap()
                .members()[0]
                .terminal(),
            Some(&Settlement::Completed(Completion::Done))
        );
        assert!(session
            .server_roster()
            .unwrap()
            .servers()
            .resolve("nonblock-srv")
            .is_ok());
    }

    #[test]
    fn stale_frontend_edit_settles_with_cas_conflict_and_refreshes_snapshot() {
        let env = IsolatedEnv::enter("fc-roster-cas");
        let root = StateRoot::new(env.home.join(".274bot"));
        vault::create_private_dir(root.path()).unwrap();
        let service = LocalStateService::open_process(root.clone());
        let initial = service.wait_server_roster(Duration::from_secs(5)).unwrap();

        let mut session = OperatorSession::<()>::new(InstancePermit::SkipLock);
        session.bind_local_state(Arc::clone(&service));
        session.poll();

        // Stall the session's writer while a second service commits a newer version.
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(parking_lot::Mutex::new(release_rx));
        let fired = Arc::new(AtomicBool::new(false));
        service.set_write_stall_for_test(Arc::new(move || {
            if !fired.swap(true, Ordering::SeqCst) {
                let _ = entered_tx.send(());
                let _ = release_rx.lock().recv();
            }
        }));
        session.stage_add_server(sample_profile("session-srv", "vault-session-srv"));
        let submitted = session
            .submit_detached(OperatorCommand::AddServer, Targets::Application)
            .unwrap();
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("session writer stalled before transaction");

        // Another process/service commits against the same initial version first.
        let peer = LocalStateService::open_process(root);
        let peer_roster = peer.wait_server_roster(Duration::from_secs(5)).unwrap();
        assert_eq!(peer_roster.version(), initial.version());
        let winner = peer
            .add_server(
                peer_roster.version(),
                sample_profile("peer-srv", "vault-peer-srv"),
                Duration::from_secs(5),
            )
            .unwrap();
        drop(peer);

        // Release the stalled session writer: it must fail with CAS Conflict and adopt winner's snapshot.
        let _ = release_tx.send(());
        poll_until_settled(&mut session, submitted.request());
        let member = &session
            .commands()
            .report(submitted.request())
            .unwrap()
            .members()[0];
        assert_eq!(
            member.terminal(),
            Some(&Settlement::Failed(CommandFailure::Failed(
                FailureKind::WriteFailed
            )))
        );
        assert!(
            member
                .detail()
                .is_some_and(|d| d.contains("expected version") && d.contains("found version")),
            "{:?}",
            member.detail()
        );
        assert_eq!(session.server_roster().unwrap().version(), winner.version());
        assert!(session
            .server_roster()
            .unwrap()
            .servers()
            .resolve("peer-srv")
            .is_ok());
    }
}
