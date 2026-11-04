//! Process-scoped transport lifetime and composition-owned maintenance.
//!
//! Constructing a Play does not initialize the service. An embedding
//! composition starts it and retains a driver before profile cache fetches,
//! game admission, or any other host-network call.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use crate::health::{ReactorLoss, WakeFailure};
use crate::host_network::AdapterNetwork;
use crate::https_handshake::HttpsLane;
use crate::reactor::TransportService;
use crate::resolver::{Resolver, ResolverConfigError, ResolverUnavailable};
use crate::tls_handshake::GameTlsLane;

// Unlike LazyLock, this distinguishes an explicitly started composition from
// an unstarted Play/cache request without starting workers as a side effect.
static PROCESS_NETWORK: OnceLock<Result<Arc<NetworkService>, NetworkStartError>> = OnceLock::new();
#[cfg(feature = "test-support")]
static TEST_TLS_CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum NetworkStartError {
    #[error("network service was not started by this composition. Start the composition before cache preparation or transport admission")]
    NotStarted,
    #[error("game reactor startup failed: {0:?}")]
    Reactor(io::ErrorKind),
    #[error("DNS resolver startup failed: {0:?}")]
    Resolver(ResolverConfigError),
    #[error("host network hook conflicts with an external installer")]
    HookConflict,
    #[error("game TLS handshake worker startup failed: {0:?}")]
    TlsWorker(io::ErrorKind),
    #[error("HTTPS handshake worker startup failed: {0:?}")]
    HttpsWorker(io::ErrorKind),
    #[error("network service has shut down")]
    ShutDown,
    #[error("too many maintenance drivers registered")]
    DriverLimit,
    #[cfg(feature = "test-support")]
    #[error("test TLS configuration must be installed exactly once before service startup")]
    TestConfigAlreadySet,
}

/// One process instance. Its fixed workers live through TCP-only operation
/// and after the last game/asset client has gone away.
pub struct NetworkService {
    reactor: Arc<TransportService>,
    resolver: Arc<Resolver>,
    drivers: Arc<AtomicUsize>,
    game_tls: Arc<GameTlsLane>,
    https: Arc<HttpsLane>,
    shutdown: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetworkHealth {
    pub reactor: Option<ReactorLoss>,
    pub resolver: Option<ResolverUnavailable>,
    pub wake_failure: Option<WakeFailure>,
    /// Bounded WSS Close flush and drain deadlines reached since service start.
    pub graceful_close_timeouts: u64,
    /// Peer resets or fatal TLS alerts during the post-Close drain.
    pub graceful_close_drain_errors: u64,
    /// Unfinished WSS drains retired when the reactor stops.
    pub graceful_close_shutdown_drops: u64,
}

/// A registration owned by an actual panel/TUI/headless/embedder event loop.
/// The owner calls maintain at most 100 ms apart while network work is live.
pub struct MaintenanceDriver {
    service: Arc<NetworkService>,
}

impl NetworkService {
    fn start() -> Result<Arc<Self>, NetworkStartError> {
        let reactor = TransportService::process_shared()
            .map_err(|error| NetworkStartError::Reactor(error.kind()))?;
        let resolver = Resolver::process_shared().map_err(NetworkStartError::Resolver)?;
        let health = reactor.health();
        #[cfg(feature = "test-support")]
        let game_tls = if let Some(config) = TEST_TLS_CONFIG.get() {
            GameTlsLane::start_with_test_config(
                reactor.bus().reactor_waker(),
                Arc::clone(&health),
                Arc::clone(config),
            )
        } else {
            GameTlsLane::start(reactor.bus().reactor_waker(), Arc::clone(&health))
        }
        .map_err(|error| {
            health.latch(ReactorLoss::StartFailed);
            reactor.shutdown();
            NetworkStartError::TlsWorker(error.kind())
        })?;
        #[cfg(not(feature = "test-support"))]
        let game_tls = GameTlsLane::start(reactor.bus().reactor_waker(), Arc::clone(&health))
            .map_err(|error| {
                health.latch(ReactorLoss::StartFailed);
                reactor.shutdown();
                NetworkStartError::TlsWorker(error.kind())
            })?;
        let tls_config = game_tls.config();
        if health.loss().is_some() {
            let _ = game_tls.shutdown();
            reactor.shutdown();
            return Err(NetworkStartError::TlsWorker(io::ErrorKind::Other));
        }
        let https = HttpsLane::start(tls_config, Arc::clone(&health)).map_err(|error| {
            health.latch(ReactorLoss::StartFailed);
            let _ = game_tls.shutdown();
            reactor.shutdown();
            NetworkStartError::HttpsWorker(error.kind())
        })?;
        reactor.install_tls_lane(Arc::clone(&game_tls));
        let drivers = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        if client::io::host_network::install_host_network(AdapterNetwork::new(
            Arc::clone(&reactor),
            Arc::clone(&resolver),
            Arc::clone(&drivers),
            Arc::clone(&shutdown),
            Arc::clone(&https),
        ))
        .is_err()
        {
            https.shutdown();
            let _ = game_tls.shutdown();
            reactor.shutdown();
            return Err(NetworkStartError::HookConflict);
        }
        Ok(Arc::new(Self {
            reactor,
            resolver,
            game_tls,
            https,
            drivers,
            shutdown,
        }))
    }

    /// A test binary may install a private CA/custom verifier before its
    /// first process-scoped service starts. Not compiled into product builds.
    #[cfg(feature = "test-support")]
    pub fn install_test_tls_config(
        config: Arc<rustls::ClientConfig>,
    ) -> Result<(), NetworkStartError> {
        if PROCESS_NETWORK.get().is_some() {
            return Err(NetworkStartError::TestConfigAlreadySet);
        }
        TEST_TLS_CONFIG
            .set(config)
            .map_err(|_| NetworkStartError::TestConfigAlreadySet)
    }

    /// Start once and register the caller's maintenance owner before any
    /// network admission. An OS startup or hook conflict remains a typed
    /// process-scoped failure; a retry cannot create a replacement worker.
    pub fn start_composition() -> Result<MaintenanceDriver, NetworkStartError> {
        let service = PROCESS_NETWORK
            .get_or_init(Self::start)
            .as_ref()
            .map_err(|error| *error)?;
        if service.shutdown.load(Ordering::Acquire) {
            return Err(NetworkStartError::ShutDown);
        }
        service
            .drivers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_add(1)
            })
            .map_err(|_| NetworkStartError::DriverLimit)?;
        Ok(MaintenanceDriver {
            service: Arc::clone(service),
        })
    }

    /// Observe an already-started composition; do not initialize one as a
    /// side effect of cache preparation, Play construction or a slot login.
    pub fn current_started() -> Result<Arc<Self>, NetworkStartError> {
        let service = PROCESS_NETWORK
            .get()
            .ok_or(NetworkStartError::NotStarted)?
            .as_ref()
            .map_err(|error| *error)?;
        if service.shutdown.load(Ordering::Acquire) {
            return Err(NetworkStartError::ShutDown);
        }
        if service.drivers.load(Ordering::Acquire) == 0 {
            return Err(NetworkStartError::NotStarted);
        }
        Ok(Arc::clone(service))
    }

    pub fn game_tls_waiter(
        &self,
        park: Arc<crate::park::Park>,
        deadline: Instant,
        token: crate::cancel::CancelToken,
    ) -> io::Result<crate::tls_handshake::GameTlsTicket> {
        if self.shutdown.load(Ordering::Acquire) || self.drivers.load(Ordering::Acquire) == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "network composition unavailable",
            ));
        }
        self.reactor.game_tls_waiter(park, deadline, token)
    }

    pub fn reactor(&self) -> &Arc<TransportService> {
        &self.reactor
    }
    pub fn resolver(&self) -> &Arc<Resolver> {
        &self.resolver
    }
    /// Shut admission before any owner is stopped. Workers never restart;
    /// an opaque platform verifier may finish late and only release custody.
    pub fn shutdown(&self) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return;
        }
        self.https.shutdown();
        let _ = self.game_tls.shutdown();
        self.reactor.shutdown();
    }

    pub fn maintain(&self, now: Instant) -> NetworkHealth {
        NetworkHealth {
            reactor: self.reactor.maintain(now),
            resolver: self.resolver.maintain().unavailable,
            wake_failure: self.reactor.health().wake_failure(),
            graceful_close_timeouts: self.reactor.health().graceful_close_timeouts(),
            graceful_close_drain_errors: self.reactor.health().graceful_close_drain_errors(),
            graceful_close_shutdown_drops: self.reactor.health().graceful_close_shutdown_drops(),
        }
    }
}

impl MaintenanceDriver {
    pub fn service(&self) -> &Arc<NetworkService> {
        &self.service
    }
    pub fn maintain(&self, now: Instant) -> NetworkHealth {
        self.service.maintain(now)
    }
}

impl Drop for MaintenanceDriver {
    fn drop(&mut self) {
        self.service.drivers.fetch_sub(1, Ordering::AcqRel);
    }
}
