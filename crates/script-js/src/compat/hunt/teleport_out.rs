use super::*;

/// Frozen `teleportOut(h, site)`: up to three casts of the site's escape
/// teleport while the player is inside, each waiting out the door window
/// fed; `null` once outside, else why the cast will not fire.
pub(crate) struct TeleportOut {
    id: String,
    label: String,
    level: i32,
    runes: Vec<(String, i32)>,
    tries: u32,
    phase: OutPhase,
    teleport: Option<crate::teleport::Teleport>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutPhase {
    Decide,
    Casting,
    Waiting { sustained: bool },
    Delay(u32),
}

/// `waitFed(() => !site.inArea(Game.tile()), DOOR_MS)`.
const DOOR_MS: u64 = 8_000;

#[derive(Deserialize)]
pub(crate) struct TeleportOutArgs {
    #[serde(default)]
    id: String,
}

impl Family for TeleportOut {
    const NAME: &'static str = "hunt-teleport-out";
    const CALLBACKS: &'static [&'static str] = HOOKS;
    const KICK_ON_START: bool = true;
    type Args = TeleportOutArgs;
    type Output = Value;

    fn begin(args: TeleportOutArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        let esc =
            crate::hunt_catalog::call("escapeRunesFor", &[json!(args.id)]).unwrap_or(Value::Null);
        let runes = esc["runes"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        let count = r["count"].as_i64().and_then(|n| i32::try_from(n).ok());
                        (
                            r["rune"].as_str().unwrap_or("").to_string(),
                            count.unwrap_or(0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Begin::Run(Self {
            label: esc["label"].as_str().unwrap_or(&args.id).to_string(),
            level: esc["level"]
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .unwrap_or(0),
            id: args.id,
            runes,
            tries: 0,
            phase: OutPhase::Decide,
            teleport: None,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        match self.phase {
            OutPhase::Casting => {
                let Some(teleport) = self.teleport.as_mut() else {
                    return self.after_cast(false, cx);
                };
                match teleport.step(cx) {
                    Step::Done(ok) => {
                        self.teleport = None;
                        self.after_cast(ok, cx)
                    }
                    Step::Fail(thrown) => Step::Fail(thrown),
                    _ => Step::Wait,
                }
            }
            OutPhase::Waiting { sustained } => {
                let Ok(inside) = self.inside(cx) else {
                    return Step::Wait;
                };
                if !inside {
                    if notify(
                        cx,
                        hook::LOG,
                        &[json!(format!("teleported out to {}", self.label))],
                    )
                    .is_err()
                    {
                        return Step::Wait;
                    }
                    return Step::Done(Value::Null);
                }
                if cx.clock().bound_reached() {
                    self.phase = OutPhase::Delay(3);
                    return Step::Wait;
                }
                if !sustained && cx.has(hook::SUSTAIN) {
                    self.phase = OutPhase::Waiting { sustained: true };
                    return Step::Call(Call {
                        hook: hook::SUSTAIN,
                        args: Vec::new(),
                    });
                }
                self.phase = OutPhase::Waiting { sustained: false };
                Step::Wait
            }
            OutPhase::Delay(n) if n > 1 => {
                self.phase = OutPhase::Delay(n - 1);
                Step::Wait
            }
            OutPhase::Delay(_) | OutPhase::Decide => self.decide(cx),
        }
    }
}

impl TeleportOut {
    fn inside(&self, cx: &mut Cx<'_>) -> Result<bool, Ended> {
        let Some(here) = observed::with(|scene| scene.since_login().here().map(Tile::from)) else {
            return Ok(false);
        };
        if !cx.has(hook::IN_AREA) {
            return Ok(false);
        }
        let point = json!({ "x": here.x, "z": here.z, "level": here.level });
        cx.ask(hook::IN_AREA, &[point]).map(|v| truthy(&v))
    }

    fn shortfall(&self) -> Option<String> {
        escape_shortfall(self.level, &self.runes)
    }

    fn decide(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        let Ok(inside) = self.inside(cx) else {
            return Step::Wait;
        };
        let why = self.shortfall();
        if why.is_some() || self.tries >= 3 || !inside {
            return Step::Done(if inside {
                json!(why.unwrap_or_else(|| "the cast never landed".into()))
            } else {
                Value::Null
            });
        }
        self.tries += 1;
        let status = json!(format!("teleporting to {}", self.label));
        if notify(cx, hook::SET_STATUS, &[status]).is_err() {
            return Step::Wait;
        }
        let args = serde_json::from_value(json!({ "name": self.id })).expect("teleport arguments");
        match <crate::teleport::Teleport as Family>::begin(args, cx) {
            Begin::Run(teleport) => {
                self.teleport = Some(teleport);
                self.phase = OutPhase::Casting;
                Step::Wait
            }
            Begin::Done(ok) => self.after_cast(ok, cx),
            Begin::Refuse(_) => self.after_cast(false, cx),
        }
    }

    fn after_cast(&mut self, ok: bool, cx: &mut Cx<'_>) -> Step<Value> {
        if ok {
            cx.clock().arm(DOOR_MS);
            self.phase = OutPhase::Waiting { sustained: false };
            return self.step(cx);
        }
        self.phase = OutPhase::Delay(3);
        Step::Wait
    }
}

/// Frozen `escapeShortfall`: why the escape cannot be cast from the posted
/// magic level and pack, or `None`. `runes` are the per-cast counts.
pub(crate) fn escape_shortfall(level: i32, runes: &[(String, i32)]) -> Option<String> {
    let (magic, held) = observed::with(|scene| {
        let session = scene.since_login();
        let magic = session
            .stats()
            .and_then(|skills| skills.magic)
            .map_or(0, |skill| skill.base);
        let held: Vec<i32> = runes
            .iter()
            .map(|(rune, _)| {
                session
                    .inv()
                    .map(|rows| {
                        rows.iter()
                            .filter(|row| {
                                row.name
                                    .as_deref()
                                    .is_some_and(|name| name.eq_ignore_ascii_case(rune))
                            })
                            .map(|row| row.count.max(0))
                            .sum()
                    })
                    .unwrap_or(0)
            })
            .collect();
        (magic, held)
    });
    if magic < level {
        return Some(format!("magic {magic} is below the {level} it needs"));
    }
    let short: Vec<&str> = runes
        .iter()
        .zip(&held)
        .filter(|((_, need), have)| *have < need)
        .map(|((rune, _), _)| rune.as_str())
        .collect();
    (!short.is_empty()).then(|| format!("no {}", short.join(" and ")))
}
