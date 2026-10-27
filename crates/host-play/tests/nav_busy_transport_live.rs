//! Ignored headless live proof: two bots cross the Yanille dungeon ledge in
//! the same game tick, and the one the server refuses waits and crosses again.
//!
//! The cell uses two disposable accounts on the shared local R289 engine and
//! drives the real `Traveller::follow` loop for each. It seeds only account
//! state (Agility 99, so the content's `stat_random(agility, 65, 355)` always
//! succeeds, and the start tile). The ledge, its reservation varp and the
//! route are the game's own. `agility_dungeon.rs2:53-57` refuses the second
//! crossing with "The ledge is being used"; the cell requires that refusal,
//! the wait of the content's hold in observed ticks, and both bots on the far
//! side. Run with Engine A and its nav pack:
//!
//! ```text
//! HOME="$TMPDIR/274bot-busy-home" LIVE=1 BOT_LIVE_NAME_PREFIX=nbt \
//!   WORLD_NAV_PACK=/path/to/274bot.navpack \
//!   cargo test -p host-play --test nav_busy_transport_live -- --ignored --nocapture --test-threads=1
//! ```

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ServerProfile, SharedClientTemplate};
use nav_executor::{TravelEvent, TravelOptions, TravelOutcome, Traveller};
use nav_planner::router::{find_with, FindOptions, Leg, Route};
use nav_planner::transport::TransportKind;
use nav_planner::world::NavWorld;
use serde_json::json;

/// `agility_dungeon.rs2:39-45`: the northern stand of loc 2303 (`balancing_ledge3`
/// at (2580,9519)); the crossing moves 8 tiles south to the landing.
const NORTH_STAND: WorldTile = WorldTile {
    x: 2580,
    z: 9520,
    level: 0,
};
const SOUTH_LANDING: WorldTile = WorldTile {
    x: 2580,
    z: 9512,
    level: 0,
};
const LEDGE_LOC_ID: i32 = 2303;
const LEDGE_BUSY: &str = "The ledge is being used";
const AGILITY: i32 = 16;

/// One transport observation of a bot's follow.
#[derive(Clone, Debug)]
enum Seen {
    Attempt {
        tick: u32,
        refused_by_client: bool,
    },
    Busy {
        tick: u32,
        message: Option<&'static str>,
        resume_tick: Option<u32>,
        tries: u32,
    },
}

struct Bot {
    name: String,
    client: client::client::Client,
    snapshot: GameSnapshot,
    pump: Pump,
    traveller: Traveller,
    seen: Rc<RefCell<Vec<Seen>>>,
    last_tick: Option<u32>,
    outcome: Option<TravelOutcome>,
}

fn selected() -> (Arc<ServerProfile>, Arc<SharedClientTemplate>) {
    let nav_pack = PathBuf::from(
        std::env::var("WORLD_NAV_PACK")
            .expect("WORLD_NAV_PACK must point at the local R289 navigation pack"),
    );
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        port: Some(44594),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(1080),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("WORLD_ENGINE_DIR").map(PathBuf::from),
        cache_dir: std::env::var_os("WORLD_CACHE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("R289 local profile resolve")
        .bind()
        .expect("R289 local profile bind");
    assert_eq!(profile.profile_class(), host_play::ProfileClass::Local);
    assert_eq!(profile.client().game_port(), 44594, "Engine A only");
    assert_eq!(profile.client().asset_port(), 1080, "Engine A only");
    let template = SharedClientTemplate::load(Arc::clone(&profile))
        .expect("selected R289 template and nav pack load");
    assert!(template.world().is_some(), "selected nav pack must load");
    (profile, template)
}

impl Bot {
    fn pump_once(&mut self) {
        self.client.mainloop();
        host::publish_snapshot(
            &mut self.snapshot,
            &self.client,
            self.pump.drain_client(&self.client),
        );
    }

    fn wait_for(&mut self, timeout: Duration, label: &str, ready: impl Fn(&GameSnapshot) -> bool) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump_once();
            if ready(&self.snapshot) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "{} {label}: timed out at {:?}",
            self.name,
            self.snapshot.tile()
        );
    }

    fn login(template: &SharedClientTemplate, name: String, uid: i32) -> Self {
        let mut client = template
            .prepare_client(uid, true)
            .expect("prepare disposable R289 client");
        client.set_draw(false);
        client.maininit();
        assert!(
            !client.error_loading,
            "R289 client asset initialization failed"
        );
        assert!(
            interact::login(&mut client, &name, &name, false),
            "{name}: login failed"
        );
        let mut bot = Self {
            name,
            client,
            snapshot: GameSnapshot::new(),
            pump: Pump::new(),
            traveller: Traveller::new(),
            seen: Rc::new(RefCell::new(Vec::new())),
            last_tick: None,
            outcome: None,
        };
        bot.wait_for(Duration::from_secs(120), "initial scene", |s| {
            s.ingame() && s.attached() && s.scene_state() == 2 && s.tile().is_some()
        });
        bot
    }

    /// Account state only: Agility 99 and the northern stand.
    fn seed(&mut self) {
        assert!(interact::cheat(&mut self.client, "setstat agility 99").is_sent());
        self.wait_for(Duration::from_secs(30), "agility grant", |s| {
            s.stats()
                .iter()
                .any(|stat| stat.index == AGILITY && stat.base >= 99)
        });
        assert!(interact::cheat(
            &mut self.client,
            &interact::tele_args(NORTH_STAND.level, NORTH_STAND.x, NORTH_STAND.z)
        )
        .is_sent());
        self.wait_for(Duration::from_secs(30), "teleport to the ledge", |s| {
            s.ingame()
                && s.scene_state() == 2
                && s.tile() == Some((NORTH_STAND.x, NORTH_STAND.z, NORTH_STAND.level))
        });
    }
}

fn route_for(world: &NavWorld, bot: &Bot) -> Route {
    let state = nav_planner::WorldState::from_snapshot(&bot.snapshot).with_map_members(true);
    let route = find_with(
        &world.collision,
        &world.graph,
        NORTH_STAND,
        SOUTH_LANDING,
        FindOptions::default(),
        &state,
    )
    .unwrap_or_else(|error| panic!("{}: route to the ledge landing: {error:?}", bot.name));
    assert!(
        route.legs.iter().any(|leg| matches!(
            leg,
            Leg::Transport { edge }
                if edge.kind == TransportKind::AgilityShortcut && edge.loc_id == LEDGE_LOC_ID
        )),
        "{}: the route must cross loc {LEDGE_LOC_ID}, got {route:?}",
        bot.name
    );
    route
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX, Engine A on 127.0.0.1:44594/1080 and a matching nav pack"]
fn nav_busy_transport_two_bots_cross_the_same_ledge() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("BOT_LIVE_NAME_PREFIX");
    assert!(
        (1..=4).contains(&prefix.len()),
        "the prefix leaves room for the role and six digits"
    );
    let (_profile, template) = selected();
    let world = template.world().expect("selected nav world");
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let uid = (serial % 1_000_000) as i32;
    let mut bots: Vec<Bot> = ["a", "b"]
        .iter()
        .enumerate()
        .map(|(i, role)| {
            Bot::login(
                &template,
                format!("{prefix}{role}{uid:06}"),
                uid * 10 + i as i32,
            )
        })
        .collect();
    for bot in &mut bots {
        bot.seed();
    }
    let routes: Vec<Route> = bots.iter().map(|bot| route_for(&world, bot)).collect();
    let mut options: Vec<TravelOptions<'_>> = bots
        .iter()
        .map(|bot| {
            let seen = Rc::clone(&bot.seen);
            TravelOptions {
                close_enough: 0,
                teleports: Some(world.graph.teleports.as_slice()),
                edges: Some(world.graph.edges.as_slice()),
                on_event: Some(Box::new(move |event: TravelEvent| match event {
                    TravelEvent::TransportAttempt { tick, refusal, .. } => {
                        seen.borrow_mut().push(Seen::Attempt {
                            tick,
                            refused_by_client: refusal.is_some(),
                        });
                    }
                    TravelEvent::TransportBusy {
                        tick,
                        message,
                        resume_tick,
                        tries,
                        ..
                    } => seen.borrow_mut().push(Seen::Busy {
                        tick,
                        message,
                        resume_tick,
                        tries,
                    }),
                    _ => {}
                })),
                ..TravelOptions::default()
            }
        })
        .collect();

    // Start both follows in the same loop pass, so both interactions reach
    // the server in the same game tick.
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(300);
    let mut started = false;
    while bots.iter().any(|bot| bot.outcome.is_none()) {
        let mut fresh = [false; 2];
        for (i, bot) in bots.iter_mut().enumerate() {
            bot.pump_once();
            let tick = bot.snapshot.tick();
            if bot.last_tick != Some(tick) {
                bot.last_tick = Some(tick);
                fresh[i] = true;
            }
        }
        if !started {
            started = fresh.iter().all(|fresh| *fresh);
        }
        if started {
            for (i, bot) in bots.iter_mut().enumerate() {
                if fresh[i] && bot.outcome.is_none() {
                    bot.outcome = bot.traveller.follow(
                        &mut bot.client,
                        &bot.snapshot,
                        routes[i].clone(),
                        &mut options[i],
                    );
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "follow timed out: {:?}",
            bots.iter()
                .map(|bot| (&bot.name, bot.snapshot.tile(), &bot.outcome))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(20));
    }
    drop(options);

    let mut busy_refusals = 0;
    let mut waits_in_ticks = Vec::new();
    let mut receipt_bots = Vec::new();
    for bot in &bots {
        let seen = bot.seen.borrow();
        assert!(
            matches!(&bot.outcome, Some(TravelOutcome::Arrived { at }) if *at == SOUTH_LANDING),
            "{}: expected exact arrival at {SOUTH_LANDING:?}, got {:?}; seen={seen:?}",
            bot.name,
            bot.outcome
        );
        assert_eq!(
            bot.snapshot.tile(),
            Some((SOUTH_LANDING.x, SOUTH_LANDING.z, SOUTH_LANDING.level)),
            "{}: arrival snapshot must be fresh",
            bot.name
        );
        for (index, item) in seen.iter().enumerate() {
            if let Seen::Busy {
                tick,
                message: Some(message),
                resume_tick: Some(resume_tick),
                tries,
            } = item
            {
                assert_eq!(*message, LEDGE_BUSY, "{}: refusal text", bot.name);
                assert!(*tries >= 1, "{}: a refusal follows a send", bot.name);
                busy_refusals += 1;
                // The hold is counted in observed ticks: the next send is
                // exactly the content's hold after the refusal.
                let sent_again = seen[index + 1..].iter().find_map(|later| match later {
                    Seen::Attempt {
                        tick,
                        refused_by_client: false,
                    } => Some(*tick),
                    _ => None,
                });
                assert_eq!(
                    sent_again,
                    Some(*resume_tick),
                    "{}: resent at the resume tick, refusal at {tick}; seen={seen:?}",
                    bot.name
                );
                waits_in_ticks.push(resume_tick - tick);
            }
        }
        receipt_bots.push(json!({
            "name": bot.name,
            "outcome": format!("{:?}", bot.outcome),
            "tile": bot.snapshot.tile(),
            "seen": format!("{seen:?}"),
        }));
    }
    assert!(
        busy_refusals >= 1,
        "no bot was refused: the sends did not meet inside the hold; {receipt_bots:?}"
    );
    assert!(
        waits_in_ticks.iter().all(|wait| *wait == 9),
        "the ledge's content hold is 9 ticks, got {waits_in_ticks:?}"
    );
    let receipt = json!({
        "request": "NAV-BUSY-TRANSPORT",
        "ledge_loc": LEDGE_LOC_ID,
        "busy_refusals": busy_refusals,
        "waits_in_ticks": waits_in_ticks,
        "bots": receipt_bots,
        "wall_seconds": started_at.elapsed().as_secs_f64(),
    });
    if let Some(dir) = std::env::var_os("LIVE_EVIDENCE_DIR") {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir).expect("create LIVE_EVIDENCE_DIR");
        fs::write(
            dir.join("nav-busy-transport-two-bots.json"),
            serde_json::to_vec_pretty(&receipt).expect("serialize receipt"),
        )
        .expect("write receipt");
    }
    println!("PASS: nav_busy_transport_live {receipt}");
    for bot in &mut bots {
        if bot.client.ingame {
            bot.client.logout();
        }
    }
}
