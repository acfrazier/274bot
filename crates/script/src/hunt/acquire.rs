use super::*;

/// Frozen `acquireKey(h, site)`: leave the lair if inside; open the site's
/// bank, deposit, take the key out (or food for the fetch), withdraw and
/// wear the gear (`hunt-bank` with `acquire`); a bank stop that fails ends
/// the run. With no key in the bank, up to three `fetchFromVelrak`
/// attempts (`hunt-cell` with `maxAttempts: 1`). Settles the frozen
/// `KeyState` off the posted pages.
pub(crate) struct Acquire {
    site: Value,
    key: i32,
    name: String,
    phase: AcquirePhase,
    fetches: u32,
    child: Option<Child>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AcquirePhase {
    Start,
    Leaving,
    Banking,
    Fetching,
}

/// Frozen `for (let attempt = 0; attempt < 3 …)` around `fetchFromVelrak`.
const VELRAK_FETCHES: u32 = 3;

impl Family for Acquire {
    const NAME: &'static str = "hunt-acquire";
    const CALLBACKS: &'static [&'static str] = HOOKS;
    const KICK_ON_START: bool = true;
    type Args = HuntArgs;
    type Output = Value;

    fn begin(args: HuntArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        let Some(site) = args.site.filter(Value::is_object) else {
            return Begin::Refuse("missing site".into());
        };
        let key = site["keyItem"]["id"]
            .as_i64()
            .and_then(|id| i32::try_from(id).ok());
        let Some(key) = key else {
            return Begin::Done(json!("held"));
        };
        let name = site["keyItem"]["name"]
            .as_str()
            .unwrap_or("key")
            .to_string();
        Begin::Run(Self {
            site,
            key,
            name,
            phase: AcquirePhase::Start,
            fetches: 0,
            child: None,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        match self.drive(cx) {
            Ok(step) => step,
            Err(Ended) => {
                self.abort(AbortReason::Terminated);
                Step::Wait
            }
        }
    }

    fn abort(&mut self, why: AbortReason) {
        match self.child.as_mut() {
            Some(Child::Leave(hunt)) => hunt.abort(why),
            Some(Child::Bank(hunt)) => hunt.abort(why),
            Some(Child::Cell(hunt)) => hunt.abort(why),
            _ => {}
        }
        self.child = None;
    }
}

impl Acquire {
    fn state(&self) -> Value {
        crate::hunt_catalog::call("keyState", &[json!(self.key)]).unwrap_or_else(|_| json!("fetch"))
    }

    fn held(&self) -> bool {
        self.state() == "held"
    }

    /// The site with `extra` merged over it, for a child run.
    fn site_with(&self, extra: Value) -> Value {
        let mut site = self.site.clone();
        if let (Some(site), Some(extra)) = (site.as_object_mut(), extra.as_object()) {
            site.extend(extra.clone());
        }
        site
    }

    /// Frozen `site.inArea(Game.tile())`: the site's own predicate, else
    /// its boxes.
    fn inside(&self, cx: &mut Cx<'_>) -> Result<bool, Ended> {
        let Some(here) = observed::with(|scene| scene.since_login().here().map(Tile::from)) else {
            return Ok(false);
        };
        if cx.has(hook::IN_AREA) {
            let point = json!({ "x": here.x, "z": here.z, "level": here.level });
            return cx.ask(hook::IN_AREA, &[point]).map(|v| truthy(&v));
        }
        let mut proj = <crate::hunt_leave::Leave as Kind>::parse(&self.site);
        Ok(<crate::hunt_leave::Leave as Kind>::area(&mut proj).contains(here, 1))
    }

    fn bank(&mut self, cx: &mut Cx<'_>) -> Result<Step<Value>, Ended> {
        self.phase = AcquirePhase::Banking;
        notify(
            cx,
            hook::SET_STATUS,
            &[json!(format!("fetching the {}", self.name))],
        )?;
        let site = self.site_with(json!({ "acquire": true }));
        self.child = Some(Child::Bank(Box::new(Hunt::run(site))));
        self.drive(cx)
    }

    fn fetch(&mut self, cx: &mut Cx<'_>) -> Result<Step<Value>, Ended> {
        self.phase = AcquirePhase::Fetching;
        self.fetches += 1;
        let site = self.site_with(json!({ "maxAttempts": 1 }));
        self.child = Some(Child::Cell(Box::new(Hunt::run(site))));
        self.drive(cx)
    }

    fn drive(&mut self, cx: &mut Cx<'_>) -> Result<Step<Value>, Ended> {
        let Some(child) = self.child.as_mut() else {
            // Start.
            if self.held() {
                return Ok(Step::Done(self.state()));
            }
            if self.inside(cx)? {
                self.phase = AcquirePhase::Leaving;
                self.child = Some(Child::Leave(Box::new(Hunt::run(self.site.clone()))));
                return self.drive(cx);
            }
            return self.bank(cx);
        };
        let step = match child {
            Child::Leave(hunt) => hunt.drive(cx)?,
            Child::Bank(hunt) => hunt.drive(cx)?,
            Child::Cell(hunt) => hunt.drive(cx)?,
            _ => Step::Done(Value::Null),
        };
        let Step::Done(value) = step else {
            return Ok(step);
        };
        self.child = None;
        let ok = value == Value::Bool(true);
        match self.phase {
            AcquirePhase::Start => Ok(Step::Done(self.state())),
            // A leave or a bank stop that never happened is a reason to
            // stop rather than press on into the Jailer fight.
            AcquirePhase::Leaving if !ok => Ok(Step::Done(self.state())),
            AcquirePhase::Leaving => self.bank(cx),
            AcquirePhase::Banking if !ok => Ok(Step::Done(self.state())),
            AcquirePhase::Banking if self.held() => {
                notify(
                    cx,
                    hook::LOG,
                    &[json!(format!("took the {} out of the bank", self.name))],
                )?;
                Ok(Step::Done(self.state()))
            }
            AcquirePhase::Banking => {
                let status = json!(format!("fetching the {} from Velrak", self.name));
                notify(cx, hook::SET_STATUS, &[status])?;
                self.fetch(cx)
            }
            AcquirePhase::Fetching => {
                if ok {
                    notify(
                        cx,
                        hook::LOG,
                        &[json!(format!("Velrak handed over the {}", self.name))],
                    )?;
                }
                if self.fetches < VELRAK_FETCHES && !self.held() {
                    return self.fetch(cx);
                }
                Ok(Step::Done(self.state()))
            }
        }
    }
}
