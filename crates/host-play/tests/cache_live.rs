//! Live: one selected profile, two actors, one shared cache preparation.
//!
//! The cache and unpack roots are isolated (`BOT_CACHE_LIVE_ROOT`, default a
//! temporary root), so this proof never prepares, fetches into or repairs the
//! operator's own cache. A root with no prepared snapshot is the cold proof
//! (the packs come from the selected update server through the real `/crc` +
//! `getJagFile` path, one snapshot is published and both actors share it); the
//! same command run again on the same root is the warm proof (the published
//! snapshot is reused: no fetch, no unpack, and still exactly one inject).
//!
//!   LIVE=1 BOT_CACHE_LIVE_REVISION=289 \
//!     cargo test -p host-play --test cache_live -- --ignored --nocapture
//!
//! Prerequisites (root owns the actual LIVE run): the selected revision's game
//! server and its update (asset) server are up on loopback, the profile
//! resolves (the engine `private.pem` or LOGIN_RSAN/E), and the two actors can
//! log in. `BOT_CACHE_LIVE_ENGINE_DIR` and `BOT_CACHE_MANIFEST` override the
//! engine root and the selected cache manifest for a non-default layout.
//! Bounded: two actors, one selected profile, no catalog or script work.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use client::unpack::{prepare_counters, snapshot_state, PrepareCounters, SnapshotState};
use host_play::progress::ProfileProgressObserver;
use host_play::{ProfileOptions, ProfilePlayOptions};

use common::{fail, live, profiles, wait_ingame};

/// Packs the selected cache directory must hold before preparation can read a
/// version (`CacheManifest::ARCHIVES` order).
const JAGS: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

fn revision() -> u16 {
    match std::env::var("BOT_CACHE_LIVE_REVISION").unwrap_or_else(|_| {
        fail("BOT_CACHE_LIVE_REVISION=274|289 is required (one revision per process)")
    }) {
        value if value == "274" => 274,
        value if value == "289" => 289,
        other => fail(&format!(
            "BOT_CACHE_LIVE_REVISION must be 274 or 289, got {other}"
        )),
    }
}

/// Dialect of one counter set against the run's baseline.
fn delta(after: &PrepareCounters, before: &PrepareCounters) -> PrepareCounters {
    PrepareCounters {
        attempts: after.attempts - before.attempts,
        jags_fetched: after.jags_fetched - before.jags_fetched,
        snapshots_published: after.snapshots_published - before.snapshots_published,
        warm_reuses: after.warm_reuses - before.warm_reuses,
        network_fills: after.network_fills - before.network_fills,
        injections: after.injections - before.injections,
    }
}

#[test]
#[ignore = "requires the selected local engine, its update server and LIVE=1"]
fn cache_live() {
    if !live() {
        return;
    }
    let revision = revision();
    let root = std::env::var_os("BOT_CACHE_LIVE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("274bot-cache-live-{revision}")));
    let cache = root.join("cache");
    let unpack = root.join("unpack");
    std::fs::create_dir_all(&cache).unwrap_or_else(|e| fail(&format!("cache root: {e}")));
    let cache_dir = cache.to_string_lossy().into_owned();
    let unpack_dir = unpack.to_string_lossy().into_owned();

    // What the root already holds decides which proof this run is: no complete
    // snapshot yet is the cold run, a complete one is the warm run.
    let warm = matches!(
        snapshot_state(&cache_dir, &unpack_dir),
        Ok(SnapshotState::Ready(_))
    );
    let empty_cache = JAGS.iter().all(|name| !cache.join(name).is_file());
    let before = prepare_counters();

    let options = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        revision: Some(revision.to_string()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(if revision == 289 { 44594 } else { 43594 }),
        http_port: Some(if revision == 289 { 1080 } else { 80 }),
        cache_dir: Some(cache.clone()),
        unpack_dir: Some(unpack.clone()),
        engine_dir: std::env::var_os("BOT_CACHE_LIVE_ENGINE_DIR").map(PathBuf::from),
        cache_manifest: std::env::var_os("BOT_CACHE_MANIFEST").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .and_then(|selection| selection.bind_with_progress(&ProfileProgressObserver::default()))
        .unwrap_or_else(|e| fail(&format!("profile bind: {e}")));
    println!(
        "cache_live: revision={revision} profile={} cache_id={} root={} warm={warm} empty_cache={empty_cache} bind_status={:?}",
        profile.label(),
        profile.cache_id(),
        root.display(),
        profile.cache_availability()
    );

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let (first, second) = (
        format!("c{revision}{:07}a", stamp % 10_000_000),
        format!("c{revision}{:07}b", stamp % 10_000_000),
    );
    let actors = profiles(&[(first.as_str(), "test"), (second.as_str(), "test")]);
    let play = host_play::run_with_profile(
        &ProfilePlayOptions {
            profile: Arc::clone(&profile),
            mainland: false,
        },
        actors,
    )
    .unwrap_or_else(|e| fail(&format!("play: {e}")));
    wait_ingame(&play, 2, Duration::from_secs(180), "cache_live");

    let after = prepare_counters();
    let delta = delta(&after, &before);
    let ready = matches!(
        snapshot_state(&cache_dir, &unpack_dir),
        Ok(SnapshotState::Ready(_))
    );
    let statuses = play.statuses();
    let workers = client::io::ondemand::OnDemand::live_workers_for(
        profile.client().game_host(),
        profile.client().game_port(),
    );
    println!("cache_live: workers={workers} statuses={statuses:?}");
    if workers != 1 {
        fail(&format!(
            "two actors must share one OnDemand worker, saw {workers}"
        ));
    }
    println!(
        "cache_live: slots={} delta=attempts {} jags {} published {} warm {} network {} injections {}",
        statuses.len(),
        delta.attempts,
        delta.jags_fetched,
        delta.snapshots_published,
        delta.warm_reuses,
        delta.network_fills,
        delta.injections
    );

    if !ready {
        fail(
            "no prepared snapshot for the selected cache version after two actors reached scene 2",
        );
    }
    if delta.injections != 1 {
        fail(&format!(
            "two actors must share exactly one snapshot inject, saw {}",
            delta.injections
        ));
    }
    if delta.snapshots_published > 1 {
        fail(&format!(
            "one cache identity published {} snapshots (one preparation expected)",
            delta.snapshots_published
        ));
    }
    if warm {
        if delta.snapshots_published != 0 {
            fail(&format!(
                "a warm run must not republish the snapshot, published {}",
                delta.snapshots_published
            ));
        }
        if delta.jags_fetched != 0 {
            fail(&format!(
                "a warm run must not fetch pack files, fetched {}",
                delta.jags_fetched
            ));
        }
        if delta.warm_reuses == 0 {
            fail("a warm run must reuse the published snapshot");
        }
    } else if empty_cache {
        if delta.jags_fetched != JAGS.len() as u64 {
            fail(&format!(
                "an empty cache directory must fetch its {} pack files, fetched {}",
                JAGS.len(),
                delta.jags_fetched
            ));
        }
        if delta.network_fills != 1 {
            fail(&format!(
                "one cold fill for the cache identity expected, saw {}",
                delta.network_fills
            ));
        }
        if delta.snapshots_published != 1 {
            fail(&format!(
                "a genuinely cold start must publish exactly one snapshot, published {}",
                delta.snapshots_published
            ));
        }
    } else {
        if delta.jags_fetched != 0 {
            fail(&format!(
                "a complete pack set must not be refetched, fetched {}",
                delta.jags_fetched
            ));
        }
        if delta.snapshots_published != 1 {
            fail(&format!(
                "a cache with a local store must publish exactly one snapshot, published {}",
                delta.snapshots_published
            ));
        }
    }

    println!("PASS: cache_live: two actors shared one prepared cache snapshot at scene 2");
}
