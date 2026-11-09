//! Narrow content proofs for obstacle paths with failure rolls or scenery writes.
//! Geometry and requirements still come from the live parsed handler. The pinned
//! AST admits only the reviewed success path, never arbitrary random branches.
use super::*;
use std::sync::LazyLock;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    Stile,
    Rocks,
    Wall,
    Window,
    Ladder,
    Gate,
}

struct Templates {
    handlers: Vec<(Block, Class)>,
    labels: HashMap<String, Block>,
    failures: Vec<Stmt>,
}

fn templates() -> &'static Templates {
    static TEMPLATES: LazyLock<Templates> = LazyLock::new(|| {
        let mut templates = Templates {
            handlers: Vec::new(),
            labels: HashMap::new(),
            failures: Vec::new(),
        };
        for (kind, name, params, body) in header_blocks(include_str!("obstacle_handlers.rs2")) {
            let body = parse_body(&body).expect("reviewed obstacle syntax");
            for stmt in &body {
                if let Stmt::If(arms, _) = stmt {
                    if arms.iter().any(|(test, _)| {
                        let mut calls = Vec::new();
                        Stmt::ReturnValue(test.clone()).calls(&mut calls);
                        calls.iter().any(|call| call == "stat_random")
                    }) {
                        templates.failures.push(stmt.clone());
                    }
                }
            }
            let block = Block {
                params,
                body,
                reads_members: false,
            };
            if kind == "label" {
                templates.labels.insert(name, block);
            } else {
                let class = match name.as_str() {
                    "death_fullstyle" => Class::Stile,
                    "troll_climbingrocks"
                    | "troll_climbingrocks_top"
                    | "troll_climbingrocks_bottom" => Class::Rocks,
                    "ardougnewalldoor_left" | "ardougnewalldoor_right" => Class::Wall,
                    "loc_2299" => Class::Window,
                    "watchladderup" => Class::Ladder,
                    "_ball_irongate" => Class::Gate,
                    _ => unreachable!("reviewed obstacle handler"),
                };
                templates.handlers.push((block, class));
            }
        }
        templates
    });
    &TEMPLATES
}

pub(super) fn classify(handler: &Block, src: &Sources) -> Option<Class> {
    let templates = templates();
    let (_, class) = templates
        .handlers
        .iter()
        .find(|(template, _)| template.params == handler.params && template.body == handler.body)?;
    let mut calls = Vec::new();
    handler.body.iter().for_each(|stmt| stmt.calls(&mut calls));
    let mut seen = HashSet::new();
    while let Some(call) = calls.pop() {
        if let Some(name) = call.strip_prefix('@') {
            if !seen.insert(name.to_owned()) {
                continue;
            }
            let expected = templates.labels.get(name)?;
            let Lookup::Found(actual) = Sources::one(&src.labels, name) else {
                return None;
            };
            if actual.params != expected.params || actual.body != expected.body {
                return None;
            }
            actual.body.iter().for_each(|stmt| stmt.calls(&mut calls));
        }
    }
    Some(*class)
}

pub(super) fn failure(stmt: &Stmt) -> bool {
    templates().failures.contains(stmt)
}

impl Class {
    pub(super) fn kind(self) -> TransportKind {
        match self {
            Self::Stile | Self::Rocks | Self::Window => TransportKind::AgilityShortcut,
            Self::Wall | Self::Ladder | Self::Gate => TransportKind::Ladder,
        }
    }
}

impl Motion<'_> {
    pub(super) fn obstacle_assignment(&mut self, stmt: &Stmt, env: &mut Env) -> Option<End> {
        if self.obstacle != Some(Class::Gate) {
            return None;
        }
        let Stmt::Assign(names, Expr::Call(name, args)) = stmt else {
            return None;
        };
        let [x, z] = names.as_slice() else {
            return None;
        };
        if name != "~door_open" || !x.starts_with('$') || !z.starts_with('$') {
            return None;
        }
        Some(
            (|| {
                let [angle, shape] = args.as_slice() else {
                    return None;
                };
                if !self.base.supported.contains("door_open")
                    || self.value(shape, env) != Some(Val::Int(0))
                {
                    return None;
                }
                let Val::Int(angle) = self.value(angle, env)? else {
                    return None;
                };
                let (dx, dz) = dir_delta(door_dir(angle)?);
                env.insert(x.clone(), Val::Int(dx));
                env.insert(z.clone(), Val::Int(dz));
                Some(())
            })()
            .map_or(End::Refused, |()| End::Continue),
        )
    }

    pub(super) fn obstacle_call(&mut self, name: &str, args: &[Expr], env: &Env) -> Option<End> {
        self.obstacle?;
        let result = match name {
            "~agility_climb_up" if self.proofs.contains("agility_climb_up") => {
                let [xp, dest] = args else {
                    return Some(End::Refused);
                };
                if !matches!(self.value(xp, env), Some(Val::Int(xp)) if xp >= 0) {
                    return Some(End::Refused);
                }
                self.climb_obstacle(dest, env)
            }
            "~climb_ladder" if self.base.supported.contains("climb_ladder") => {
                let [dest, up] = args else {
                    return Some(End::Refused);
                };
                if !matches!(self.value(up, env), Some(Val::Bool(_))) {
                    return Some(End::Refused);
                }
                self.climb_obstacle(dest, env)
            }
            "~set_forcemove_bas" if self.proofs.contains("obstacle_appearance") => {
                if matches!(args, [animation] if self.value(animation, env).is_some()) {
                    Some(())
                } else {
                    None
                }
            }
            "~update_bas" if self.proofs.contains("obstacle_appearance") && args.is_empty() => {
                Some(())
            }
            "loc_add"
                if self.obstacle == Some(Class::Wall)
                    || self.obstacle == Some(Class::Gate) && self.moved =>
            {
                match args {
                    [coord, loc, angle, shape, duration] => {
                        match (
                            self.value(coord, env),
                            self.value(loc, env),
                            self.value(angle, env),
                            self.value(shape, env),
                            self.value(duration, env),
                        ) {
                            (
                                Some(Val::Coord(coord)),
                                Some(Val::Name(loc)),
                                Some(Val::Int(angle)),
                                Some(Val::Int(shape)),
                                Some(Val::Int(duration)),
                            ) if in_world_box(&coord)
                                && self.base.src.ids.contains_key(&loc)
                                && (0..=3).contains(&angle)
                                && (0..=22).contains(&shape)
                                && duration >= 0 =>
                            {
                                Some(())
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            "~agility_climb_up" | "~climb_ladder" | "~set_forcemove_bas" | "~update_bas" => None,
            _ => return None,
        };
        Some(result.map_or(End::Refused, |()| End::Continue))
    }

    fn climb_obstacle(&mut self, dest: &Expr, env: &Env) -> Option<()> {
        let Val::Coord(dest) = self.value(dest, env)? else {
            return None;
        };
        self.price(1)?;
        self.relocate(dest)?;
        self.moved = true;
        Some(())
    }
}
