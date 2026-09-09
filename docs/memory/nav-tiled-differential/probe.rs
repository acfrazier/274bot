// Included as a sibling of unchanged frozen modules, not a replacement router.
use crate::bank_fetch::{plan_bank_fetch, BankStep};
use crate::router::{self, CostModel, FindOptions, Route, RouteError};
use crate::router::{find_missing_item_reqs, find_with};
use crate::world::NavWorld;
use crate::WorldState;
use crate::{collision::WorldCollision, pack::BankStand, transport::TransportGraph};
use api::snapshot::WorldTile;
use std::collections::VecDeque;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
include!("host-probe.rs");

fn outcome(out: &mut impl Write, result: RouteOutcome) {
    match result {
        RouteOutcome::NoPath => debug(out, "host-outcome", "NoPath"),
        RouteOutcome::Routed(r) => route(out, "host-route", Ok(r)),
        RouteOutcome::BankSession { pending, route: r } => {
            debug(
                out,
                "host-bank",
                (pending.steps, pending.dest, pending.opts),
            );
            route(out, "host-final", Ok(pending.final_route));
            route(out, "host-route", Ok(r));
        }
    }
}

fn radius_searches(out: &mut impl Write, world: Arc<NavWorld>) {
    let (o, w, h, _) = geometry(&world.collision);
    for p in 0..4 {
        for z in 0..h {
            for x in 0..w {
                let from = tile(o.x + x as i32, o.z + z as i32, p);
                for q in 0..4 {
                    for tz in 0..h {
                        for tx in 0..w {
                            let to = tile(o.x + tx as i32, o.z + tz as i32, q);
                            for radius in [1, 2] {
                                debug(out, "radius-selector", (from, to, radius));
                                debug(
                                    out,
                                    "approach-ordered",
                                    approach_tiles(&world, from, to, radius),
                                );
                                let request = ScriptRouteRequest {
                                    generation: 0,
                                    world: world.clone(),
                                    from,
                                    to,
                                    radius,
                                    opts: FindOptions {
                                        allow_bank_fetch: true,
                                        allow_teleports: true,
                                        ..Default::default()
                                    },
                                    state: Some(facts(1)),
                                    bank: vec![(995, 10), (1712, 1)],
                                };
                                outcome(out, request.calculate());
                            }
                        }
                    }
                }
            }
        }
    }
}

fn fixed_searches(out: &mut impl Write, mut world: Arc<NavWorld>, selectors: &str) {
    for line in selectors.lines() {
        let v: Vec<i32> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 10);
        let from = tile(v[0], v[1], v[2]);
        let to = tile(v[3], v[4], v[5]);
        let bits = v[6];
        let opts = FindOptions {
            allow_teleports: bits & 1 != 0,
            allow_wilderness: bits & 2 != 0,
            allow_bank_fetch: bits & 4 != 0,
            essence: if bits & 8 != 0 {
                crate::essence::essence_session_for_wizard(553)
            } else {
                None
            },
        };
        let model = if v[9] == 0 {
            CostModel::running()
        } else {
            CostModel {
                run_per_step: 1.0,
                walk_per_step: 1.0,
            }
        };
        debug(out, "fixed-selector", &v);
        route(
            out,
            "fixed-model",
            router::find_with_model(&world.collision, &world.graph, from, to, model),
        );
        route(
            out,
            "fixed-tele-model",
            router::find_allow_teleports_with_model(
                &world.collision,
                &world.graph,
                from,
                to,
                model,
                &facts(v[7] as u8),
            ),
        );
        let request = ScriptRouteRequest {
            generation: 0,
            world,
            from,
            to,
            radius: v[8],
            opts,
            state: Some(facts(v[7] as u8)),
            bank: vec![(995, 10), (1712, 1)],
        };
        outcome(out, request.calculate());
        world = request.world;
    }
}

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}
fn geometry(c: &WorldCollision) -> (WorldTile, usize, usize, usize) {
    unimplemented!("GEOMETRY")
}
fn pair(c: &WorldCollision, i: usize) -> Option<(u8, bool)> {
    unimplemented!("PAIR")
}
fn words(c: &WorldCollision) -> Vec<u64> {
    unimplemented!("WORDS")
}
fn frame(out: &mut impl Write, tag: &str, bytes: &[u8]) {
    writeln!(out, "{} {}", tag, bytes.len()).unwrap();
    out.write_all(bytes).unwrap();
    out.write_all(b"\n").unwrap();
}
fn debug(out: &mut impl Write, tag: &str, value: impl std::fmt::Debug) {
    frame(out, tag, format!("{:?}", value).as_bytes());
}
fn route(out: &mut impl Write, tag: &str, result: Result<Route, RouteError>) {
    // Full derived Debug includes every leg, tile and every edge field in order.
    // Add the f64 bits so float formatting cannot hide a different cost.
    debug(
        out,
        tag,
        (&result, result.as_ref().ok().map(|r| r.ticks.to_bits())),
    );
}
fn state(out: &mut impl Write, s: &crate::WorldState) {
    let mut inv: Vec<_> = s.inv.iter().collect();
    inv.sort();
    let mut worn: Vec<_> = s.worn.iter().collect();
    worn.sort();
    let mut stats: Vec<_> = s.stats.iter().collect();
    stats.sort();
    let mut varps: Vec<_> = s.varps.iter().collect();
    varps.sort();
    let mut quests: Vec<_> = s.quests.iter().collect();
    quests.sort();
    debug(out, "post-state", (inv, worn, stats, varps, quests));
}
fn graph(out: &mut impl Write, g: &TransportGraph, b: &[BankStand]) {
    debug(out, "edges", &g.edges);
    debug(out, "teleports", &g.teleports);
    let mut at: Vec<_> = g.at.iter().collect();
    // HashMap has no iteration-order contract; retain ALL keys and the entire
    // original index Vec order. Never sort edges/requirements/index vectors.
    at.sort_by_key(|(t, _)| (t.level, t.z, t.x));
    debug(out, "at", at);
    debug(out, "banks", b);
}
fn cells(out: &mut impl Write, c: &WorldCollision) {
    let (o, w, h, n) = geometry(c);
    debug(out, "geometry", (o, w, h, n));
    // Fixed-width lossless records, streaming (no second whole-world map).
    writeln!(out, "cells {}", n * 15).unwrap();
    for i in 0..n {
        let t = tile(
            o.x + (i % w) as i32,
            o.z + ((i % (w * h)) / w) as i32,
            (i / (w * h)) as i32,
        );
        let (f, b) = pair(c, i).unwrap();
        out.write_all(&[f, b as u8]).unwrap();
        out.write_all(&crate::collision::walk_word_from_parts(f, b).to_le_bytes())
            .unwrap();
        out.write_all(&c.walkable_word(t.x, t.z, t.level).to_le_bytes())
            .unwrap();
        out.write_all(&c.flag(t.x, t.z, t.level).to_le_bytes())
            .unwrap();
        out.write_all(&[(c.walkable(t) as u8) | ((c.standable(t) as u8) << 1)])
            .unwrap();
    }
    out.write_all(b"\n").unwrap();
    let v = words(c);
    writeln!(out, "blocked-words {}", v.len() * 8).unwrap();
    for w in v {
        out.write_all(&w.to_le_bytes()).unwrap();
    }
    out.write_all(b"\n").unwrap();
    for p in -1..=4 {
        for (x, z) in [
            (o.x - 1, o.z),
            (o.x, o.z - 1),
            (o.x + w as i32, o.z),
            (o.x, o.z + h as i32),
            (o.x, o.z),
        ] {
            let t = tile(x, z, p);
            debug(
                out,
                "bounds",
                (
                    t,
                    c.walkable_word(x, z, p),
                    c.flag(x, z, p),
                    c.walkable(t),
                    c.standable(t),
                ),
            );
        }
    }
}
fn facts(mask: u8) -> crate::WorldState {
    let mut s = crate::WorldState::empty();
    if mask & 1 != 0 {
        s.stats.insert(6, 25);
        s.stats.insert(2, 3);
        s.quests.insert("Rune Mysteries".into());
        s.quests.insert("é".into());
        s.varps.insert(150, 160);
    }
    if mask & 2 != 0 {
        s.inv.insert(995, 10);
        s.worn.insert(1712);
    }
    s
}
fn searches(out: &mut impl Write, c: &WorldCollision, g: &TransportGraph, b: &[BankStand]) {
    let (o, w, h, _) = geometry(c);
    assert!(w * h <= 9, "all-pairs only bounded tiny maps");
    let tiles: Vec<_> = (0..4)
        .flat_map(|p| {
            (0..h).flat_map(move |z| (0..w).map(move |x| tile(o.x + x as i32, o.z + z as i32, p)))
        })
        .collect();
    for &from in &tiles {
        for &to in &tiles {
            debug(out, "pair", (from, to));
            for budget in [0, 1, 5] {
                route(
                    out,
                    "budget",
                    router::differential_access::find(
                        c,
                        g,
                        from,
                        to,
                        CostModel::running(),
                        budget,
                        FindOptions::default(),
                        &facts(0),
                    ),
                );
            }
            for model in [
                CostModel::running(),
                CostModel {
                    run_per_step: 1.0,
                    walk_per_step: 1.0,
                },
            ] {
                route(out, "model", router::find_with_model(c, g, from, to, model));
                for sm in 0..4 {
                    route(
                        out,
                        "tele-model",
                        router::find_allow_teleports_with_model(c, g, from, to, model, &facts(sm)),
                    );
                }
            }
            for sm in 0..4 {
                let s = facts(sm);
                for bits in 0..16 {
                    let opts = FindOptions {
                        allow_teleports: bits & 1 != 0,
                        allow_wilderness: bits & 2 != 0,
                        allow_bank_fetch: bits & 4 != 0,
                        essence: if bits & 8 != 0 {
                            Some(crate::essence::EssenceSession {
                                wizard_npc: 553,
                                return_tile: tile(o.x + 2, o.z + 2, 3),
                            })
                        } else {
                            None
                        },
                    };
                    route(out, "options", router::find_with(c, g, from, to, opts, &s));
                    route(
                        out,
                        "walking-options",
                        router::differential_access::find(
                            c,
                            g,
                            from,
                            to,
                            CostModel {
                                run_per_step: 1.0,
                                walk_per_step: 1.0,
                            },
                            4_000_000,
                            opts,
                            &s,
                        ),
                    );
                    if bits & 4 != 0 {
                        let missing = router::find_missing_item_reqs(c, g, from, to, opts, &s);
                        debug(out, "missing", &missing);
                        if let Some(missing) = missing {
                            for bank in [&[][..], &[(995, 10), (1712, 1)][..]] {
                                let plan =
                                    crate::bank_fetch::plan_bank_fetch(&missing, &s, bank, b, from);
                                debug(out, "bank-steps", plan.as_ref().map(|p| &p.steps));
                                if let Some(plan) = plan {
                                    state(out, &plan.state);
                                    route(
                                        out,
                                        "after-bank",
                                        router::find_with(c, g, from, to, opts, &plan.state),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
fn corner(out: &mut impl Write, c: &WorldCollision) {
    let (o, w, _, _) = geometry(c);
    for p in 0..4 {
        for x in [31, 63] {
            let cur = if w == 65 {
                tile(o.x + x, o.z + 1, p)
            } else {
                tile(o.x + 1, o.z + x, p)
            };
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if dx != 0 || dz != 0 {
                        debug(
                            out,
                            "step",
                            (cur, dx, dz, router::step_ok(c, cur, (dx, dz))),
                        );
                    }
                }
            }
        }
    }
}
fn world(
    out: &mut impl Write,
    mut c: WorldCollision,
    g: TransportGraph,
    b: Vec<BankStand>,
    routes: bool,
    side: bool,
) -> Arc<NavWorld> {
    cells(out, &c);
    graph(out, &g, &b);
    let wire = crate::pack::encode(&c, &g, &b);
    frame(out, "canonical", &wire);
    let (round, rg, rb) = crate::pack::decode(&wire).unwrap();
    cells(out, &round);
    graph(out, &rg, &rb);
    frame(
        out,
        "round-canonical",
        &crate::pack::encode(&round, &rg, &rb),
    );
    drop(round);
    drop(wire);
    if side {
        let (_, _, _, n) = geometry(&c);
        let flags: Vec<_> = (0..n)
            .map(|i| (i as u32).wrapping_mul(0x9e3779b9))
            .collect();
        let (o, w, h, _) = geometry(&c);
        for i in 0..n {
            let t = tile(
                o.x + (i % w) as i32,
                o.z + ((i % (w * h)) / w) as i32,
                (i / (w * h)) as i32,
            );
            debug(
                out,
                "external-flag",
                c.flag_index(&flags, t.x, t.z, t.level),
            );
        }
        c.attach_flags(flags);
        cells(out, &c);
        c.drop_flags();
        cells(out, &c);
    }
    if routes {
        searches(out, &c, &g, &b);
    }
    let world = Arc::new(NavWorld::from_parts(c, g, b));
    if routes {
        radius_searches(out, world.clone());
    }
    world
}
pub fn run() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 4);
    assert!(args[3] == "generated" || args[3] == "real");
    let root = Path::new(&args[2]);
    let selectors = std::fs::read_to_string(&args[1]).unwrap();
    let mut out = BufWriter::new(std::io::stdout().lock());
    let mut count = 0;
    if args[3] == "real" {
        // Supervisor supplies one copied hash-bound pack and frozen route rows.
        let bytes = std::fs::read(root.join("input.bin")).unwrap();
        assert!(bytes.len() <= 128 * 1024 * 1024);
        let (c, g, b) = crate::pack::decode(&bytes).unwrap();
        drop(bytes);
        let world = world(&mut out, c, g, b, false, false);
        fixed_searches(&mut out, world, &selectors);
        debug(&mut out, "complete-input-count", 1);
        out.flush().unwrap();
        return;
    }
    for line in selectors.lines() {
        let f: Vec<_> = line.split('\t').collect();
        assert_eq!(f.len(), 3);
        let path = root.join(f[0]);
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() <= 1024 * 1024);
        debug(&mut out, "input", f[0]);
        if f[1] == "sidecar" {
            match crate::pack::decode_flags_sidecar(&bytes) {
                Ok((o, w, h, flags)) => {
                    debug(&mut out, "side", (o, w, h, &flags));
                    frame(
                        &mut out,
                        "side-wire",
                        &crate::pack::encode_flags_sidecar(o, w, h, &flags),
                    );
                }
                Err(e) => debug(&mut out, "side-error", e),
            }
        } else {
            match crate::pack::decode(&bytes) {
                Ok((c, g, b)) => {
                    debug(&mut out, "decode", "Ok");
                    if f[1] == "corner" {
                        corner(&mut out, &c);
                    } else {
                        let world = world(&mut out, c, g, b, f[2] == "all", true);
                        if f[2] == "all" {
                            fixed_searches(
                                &mut out,
                                world,
                                &std::fs::read_to_string(root.join("fixed.tsv")).unwrap(),
                            );
                        }
                    }
                }
                Err(e) => debug(&mut out, "decode-error", &e),
            }
            if f[1] != "corner" {
                // Exercise the original load function, including real second
                // file read in BadMagic-only fallback. No handwritten error oracle.
                debug(
                    &mut out,
                    "fallback",
                    matches!(
                        crate::pack::decode(&bytes),
                        Err(crate::pack::PackError::BadMagic)
                    ),
                );
                match crate::world::NavWorld::load_pack(&path) {
                    Ok(w) => {
                        debug(&mut out, "load", "Ok");
                        cells(&mut out, &w.collision);
                        graph(&mut out, &w.graph, w.banks());
                    }
                    Err(e) => debug(&mut out, "load-error", e),
                }
            }
        }
        count += 1;
    }
    debug(&mut out, "complete-input-count", count);
    out.flush().unwrap();
}
