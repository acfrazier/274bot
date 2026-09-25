use super::*;

pub(super) const HOOK_ENABLED: usize = 0;
pub(super) const HOOK_LOG: usize = 1;
pub(super) const HOOK_STATUS: usize = 2;

#[derive(Deserialize)]
pub(crate) struct ClueArgs {
    pub(super) token: u64,
}

/// Execute-loop family over the isolate clue session.
pub(crate) struct Clue {
    pub(super) token: u64,
    pub(super) last_hook: Option<usize>,
    pub(super) resume: Option<bool>,
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
        let selected = crate::supply_v2::selected_data();
        loop {
            let mut payload = json!({ "op": "next", "token": self.token });
            if let Some(resume) = self.resume.take() {
                payload["resume"] = json!(resume);
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
