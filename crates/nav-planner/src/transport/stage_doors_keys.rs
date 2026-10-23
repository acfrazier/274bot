//! Item-on-loc openers use the same certified door interpreter as ordinary
//! operations. The selected item is a carried requirement, never a free op.
use super::*;

fn selected_items(body: &[Stmt], out: &mut HashSet<String>) {
    fn condition(expr: &Expr, out: &mut HashSet<String>) {
        match expr {
            Expr::Cmp(CmpOp::Eq, a, b) => {
                if let (Expr::Word(a), Expr::Word(b)) = (a.as_ref(), b.as_ref()) {
                    if a == "last_useitem" {
                        out.insert(b.clone());
                    }
                    if b == "last_useitem" {
                        out.insert(a.clone());
                    }
                }
            }
            Expr::And(a, b) | Expr::Or(a, b) => {
                condition(a, out);
                condition(b, out);
            }
            _ => {}
        }
    }
    for stmt in body {
        match stmt {
            Stmt::If(arms, other) => {
                for (test, body) in arms {
                    condition(test, out);
                    selected_items(body, out);
                }
                if let Some(body) = other {
                    selected_items(body, out);
                }
            }
            Stmt::Block(body) => selected_items(body, out),
            _ => {}
        }
    }
}

pub(super) fn key_use_edges(
    src: &Sources,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
    observable: &ObservableGates,
    audit: &mut VarpGateAudit,
) {
    let supported = src.supported_procs().0;
    let mut ids: Vec<_> = src.oplocu.keys().collect();
    ids.sort_unstable();
    for name in ids {
        let (Some(&id), Lookup::Found(handler)) =
            (src.ids.get(name), Sources::one(&src.oplocu, name))
        else {
            continue;
        };
        if !names_terminal(src, handler) {
            continue;
        }
        let mut items = HashSet::new();
        selected_items(&handler.body, &mut items);
        let mut items: Vec<_> = items.into_iter().collect();
        items.sort_unstable();
        for item in items {
            let Some(&obj) = src.objs.get(&item) else {
                continue;
            };
            for placement in positions.get(&id).into_iter().flatten() {
                let Some(Role::Wall(angle_dir)) = placement_role(placement) else {
                    continue;
                };
                let at = WorldTile {
                    x: placement.x,
                    z: placement.z,
                    level: placement.level,
                };
                for dir in [angle_dir, opposite(angle_dir)] {
                    let (dx, dz) = dir_delta(angle_dir);
                    let stand = if dir == angle_dir {
                        at
                    } else {
                        WorldTile {
                            x: at.x + dx,
                            z: at.z + dz,
                            level: at.level,
                        }
                    };
                    let mut eval = Eval {
                        src,
                        supported: &supported,
                        at,
                        stand: Some(stand),
                        angle: placement.angle,
                        needs: Needs {
                            items: vec![(obj, 1)],
                            ..Needs::default()
                        },
                        relative_landing_safe: false,
                        jumps: 0,
                    };
                    let mut env = Env::from([("last_useitem".to_owned(), Val::Name(item.clone()))]);
                    let Flow::Crossed(Arrival::Door(leaf), _) = eval.run(&handler.body, &mut env)
                    else {
                        continue;
                    };
                    let Some(to) = straight_door_landing(at, angle_dir, dir, collision) else {
                        continue;
                    };
                    let mut edge = gated_edge(TransportKind::Door, id, at, to, eval.needs);
                    edge.option = 0;
                    edge.dir = Some(dir);
                    edge.open_loc_id = match leaf {
                        LocRef::Id(id) => Some(id),
                        _ => None,
                    };
                    if observable.admit_edge(graph, edge, audit) && src.has_member_reads(handler) {
                        audit.record_member_path(graph.edges.last().expect("admitted edge"));
                    }
                }
            }
        }
    }
    let _ = skipped;
}
