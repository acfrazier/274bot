//! Narrow content rules for obstacle handlers with failure rolls or scenery
//! writes. The obstacle class is selected by handler name; geometry and
//! requirements come from the live parsed handler.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    Stile,
    Rocks,
    Wall,
    Window,
    Ladder,
    Gate,
}

/// The obstacle handlers the interpreter models, by handler name.
fn class_of(name: &str) -> Option<Class> {
    match name {
        "death_fullstyle" => Some(Class::Stile),
        "troll_climbingrocks" | "troll_climbingrocks_top" | "troll_climbingrocks_bottom" => {
            Some(Class::Rocks)
        }
        "ardougnewalldoor_left" | "ardougnewalldoor_right" => Some(Class::Wall),
        "loc_2299" => Some(Class::Window),
        "watchladderup" => Some(Class::Ladder),
        "_ball_irongate" => Some(Class::Gate),
        _ => None,
    }
}

/// The class of the obstacle handler `name`. Every label the handler can
/// jump to must resolve to exactly one definition.
pub(super) fn classify(name: &str, handler: &Block, src: &Sources) -> Option<Class> {
    let class = class_of(name)?;
    let mut calls = Vec::new();
    handler.body.iter().for_each(|stmt| stmt.calls(&mut calls));
    let mut seen = HashSet::new();
    while let Some(call) = calls.pop() {
        if let Some(label) = call.strip_prefix('@') {
            if !seen.insert(label.to_owned()) {
                continue;
            }
            let Lookup::Found(actual) = Sources::one(&src.labels, label) else {
                return None;
            };
            actual.body.iter().for_each(|stmt| stmt.calls(&mut calls));
        }
    }
    Some(class)
}

/// A failure roll of a Rocks handler: an `if` whose test calls `stat_random`.
pub(super) fn failure(stmt: &Stmt) -> bool {
    let Stmt::If(arms, _) = stmt else {
        return false;
    };
    arms.iter().any(|(test, _)| {
        let mut calls = Vec::new();
        Stmt::ReturnValue(test.clone()).calls(&mut calls);
        calls.iter().any(|call| call == "stat_random")
    })
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
