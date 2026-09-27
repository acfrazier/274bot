use super::*;

pub(super) const HOOK_ENABLED: usize = 0;
pub(super) const HOOK_LOG: usize = 1;
pub(super) const HOOK_STATUS: usize = 2;

#[derive(Deserialize)]
pub(crate) struct ClueArgs {
    pub(super) token: u64,
    #[serde(default)]
    pub(super) clue_duel_partner: String,
    #[serde(default)]
    pub(super) self_name: String,
}

/// Execute-loop family over the isolate clue session.
pub(crate) struct Clue {
    pub(super) token: u64,
    pub(super) last_hook: Option<usize>,
    pub(super) resume: Option<bool>,
    pub(super) clue_duel_partner: String,
    pub(super) self_name: String,
    pub(super) duel_refusing: bool,
    pub(super) duel_travel: Option<crate::duel::Travel>,
    pub(super) duel_crossed: bool,
}

impl Family for Clue {
    const NAME: &'static str = "clue";
    const EXCLUSIVE: bool = true;
    const KICK_ON_START: bool = true;
    const CALLBACKS: &'static [&'static str] = &["enabled", "log", "setStatus"];
    type Args = ClueArgs;
    type Output = Value;

    fn begin(args: ClueArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        let live = RUNTIME.with(|rt| {
            let rt = rt.borrow();
            (rt.phase != Phase::Idle && rt.token == args.token).then_some(args.token)
        });
        match live {
            Some(token) => Begin::Run(Self {
                token,
                last_hook: None,
                resume: None,
                clue_duel_partner: args.clue_duel_partner,
                self_name: args.self_name,
                duel_refusing: false,
                duel_travel: None,
                duel_crossed: false,
            }),
            None => Begin::Refuse("stale".into()),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        if let Some(reply) = cx.reply() {
            match self.last_hook.take() {
                Some(HOOK_ENABLED) => match reply {
                    Reply::Value(value) => self.resume = Some(value.as_bool() == Some(true)),
                    Reply::Threw(thrown) => return Step::Fail(thrown),
                },
                Some(_) => {
                    if let Reply::Threw(thrown) = reply {
                        return Step::Fail(thrown);
                    }
                }
                None => {}
            }
        }
        if self.duel_refusing {
            self.duel_refusing = false;
            return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
        }
        let selected = crate::supply_v2::selected_data();
        if let Some(travel) = self.duel_travel.as_mut() {
            match travel.step(cx) {
                Step::Wait => return Step::Wait,
                Step::Done(true) => {
                    self.duel_travel = None;
                    self.duel_crossed = true;
                }
                Step::Done(false) => {
                    self.duel_travel = None;
                    return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
                }
                Step::Fail(thrown) => return Step::Fail(thrown),
                Step::Call(_) => {
                    return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
                }
            }
        }
        loop {
            let mut payload = json!({ "op": "next", "token": self.token });
            if let Some(resume) = self.resume.take() {
                payload["resume"] = json!(resume);
            }
            if self.duel_crossed {
                payload["duel_crossed"] = json!(true);
            }
            let step = dispatch(selected.as_deref(), &payload);
            let kind = step.get("kind").and_then(Value::as_str).unwrap_or("");
            match kind {
                "callback.enabled" => {
                    if !cx.has(HOOK_ENABLED) {
                        self.resume = Some(true);
                        continue;
                    }
                    self.last_hook = Some(HOOK_ENABLED);
                    return Step::Call(Call {
                        hook: HOOK_ENABLED,
                        args: Vec::new(),
                    });
                }
                "callback.log" => {
                    if !cx.has(HOOK_LOG) {
                        continue;
                    }
                    self.last_hook = Some(HOOK_LOG);
                    return Step::Call(Call {
                        hook: HOOK_LOG,
                        args: vec![step.get("message").cloned().unwrap_or(json!(""))],
                    });
                }
                "callback.setStatus" => {
                    if !cx.has(HOOK_STATUS) {
                        continue;
                    }
                    self.last_hook = Some(HOOK_STATUS);
                    return Step::Call(Call {
                        hook: HOOK_STATUS,
                        args: vec![step.get("message").cloned().unwrap_or(json!(""))],
                    });
                }
                "grind-ready" => {}
                "wait" | "supplies-needed" | "no-shop" => return Step::Wait,
                "yield" => return Step::Done(step),
                "done" | "dead" | "abandon" | "guardian-lost" | "aborted" => {
                    return Step::Done(step)
                }
                "duel-travel" => {
                    if !crate::duel::valid_partner(&self.clue_duel_partner, &self.self_name) {
                        if cx.has(HOOK_LOG) {
                            self.duel_refusing = true;
                            self.last_hook = Some(HOOK_LOG);
                            return Step::Call(Call {
                                hook: HOOK_LOG,
                                args: vec![json!(
                                    "set Global clue duel partner and run that account in Duel Arena Clue helper mode"
                                )],
                            });
                        }
                        return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
                    }
                    let Some(x) = step
                        .get("x")
                        .and_then(Value::as_i64)
                        .and_then(|v| i32::try_from(v).ok())
                    else {
                        return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
                    };
                    let Some(z) = step
                        .get("z")
                        .and_then(Value::as_i64)
                        .and_then(|v| i32::try_from(v).ok())
                    else {
                        return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
                    };
                    let Some(level) = step
                        .get("level")
                        .and_then(Value::as_i64)
                        .and_then(|v| i32::try_from(v).ok())
                    else {
                        return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
                    };
                    let Some(radius) = step
                        .get("radius")
                        .and_then(Value::as_i64)
                        .and_then(|v| i32::try_from(v).ok())
                    else {
                        return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
                    };
                    self.duel_travel = Some(crate::duel::Travel::clue(
                        self.clue_duel_partner.clone(),
                        self.self_name.clone(),
                        x,
                        z,
                        level,
                        radius,
                    ));
                    let travel = self.duel_travel.as_mut().expect("just inserted");
                    match travel.step(cx) {
                        Step::Wait => return Step::Wait,
                        Step::Done(true) => {
                            self.duel_travel = None;
                            self.duel_crossed = true;
                            continue;
                        }
                        Step::Done(false) | Step::Call(_) => {
                            self.duel_travel = None;
                            return Step::Done(json!({ "kind": "aborted", "reason": "clue-duel" }));
                        }
                        Step::Fail(thrown) => return Step::Fail(thrown),
                    }
                }
                _ => {
                    if let Some(req) = verb_req(&step) {
                        cx.emit(req);
                        return Step::Wait;
                    }
                    return Step::Done(step);
                }
            }
        }
    }
}
