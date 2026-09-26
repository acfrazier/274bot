use super::*;

/// Frozen `waitFed(cond, ms)`: `cond()` each tick until it holds or `ms`
/// pass, pumping the script's `sustain` between polls; then `cond()` once
/// more. Synchronous `cond`, awaited `sustain`.
pub(crate) struct WaitFed {
    sustained: bool,
}

#[derive(Deserialize)]
pub(crate) struct WaitFedArgs {
    ms: f64,
}

impl Family for WaitFed {
    const NAME: &'static str = "hunt-wait-fed";
    const CALLBACKS: &'static [&'static str] = HOOKS;
    const KICK_ON_START: bool = true;
    type Args = WaitFedArgs;
    type Output = bool;

    fn begin(args: WaitFedArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let ms = if args.ms.is_finite() {
            args.ms.max(0.0) as u64
        } else {
            0
        };
        cx.clock().arm(ms);
        Begin::Run(Self { sustained: false })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        match cx.reply() {
            Some(Reply::Threw(thrown)) => return Step::Fail(thrown),
            Some(Reply::Value(_)) if self.sustained => {
                self.sustained = false;
                return Step::Wait;
            }
            _ => {}
        }
        let Ok(held) = cx.ask(hook::COND, &[]) else {
            return Step::Wait;
        };
        if truthy(&held) || cx.clock().bound_reached() {
            return Step::Done(truthy(&held));
        }
        if cx.has(hook::SUSTAIN) {
            self.sustained = true;
            return Step::Call(Call {
                hook: hook::SUSTAIN,
                args: Vec::new(),
            });
        }
        Step::Wait
    }
}
