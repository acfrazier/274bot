use mlua::{ffi, Error, Lua};

unsafe extern "C-unwind" fn step(state: *mut ffi::lua_State, _debug: *mut ffi::lua_Debug) {
    // SAFETY: `enable` installs this hook on the main state before any
    // coroutine exists. `exec_raw` and each `Thread::resume` hold mlua's
    // reentrant state lock. A resume holds that lock for the full entry, so
    // another thread cannot change the callback table while this hook reads
    // it. The VM-owned watchdog interrupt outlives every entry. The `-1`
    // argument selects mlua's non-GC path. Its `used_memory` check re-enters
    // the same lock on this thread. The callback returns `Continue`, never
    // `Yield`. No Rust value with a destructor crosses this C-unwind callback
    // or `lua_error`. If the interrupt is absent, this hook errors before the
    // next Luau instruction.
    unsafe {
        let callbacks = ffi::lua_callbacks(state);
        if let Some(interrupt) = (*callbacks).interrupt {
            interrupt(state, -1);
        } else {
            ffi::lua_pushnil(state);
            ffi::lua_error(state);
        }
    }
}

pub(super) fn enable(lua: &Lua) -> mlua::Result<()> {
    let mut installed = false;
    // SAFETY: `exec_raw` holds this VM's mlua lock and restores its stack. The
    // main state is live, and no chunk or coroutine has run yet. The callback
    // table and main state are VM-owned; later coroutines inherit single-step.
    unsafe {
        lua.exec_raw::<()>((), |state| {
            let callbacks = ffi::lua_callbacks(state);
            if (*callbacks).interrupt.is_some() {
                (*callbacks).debugstep = Some(step);
                ffi::lua_singlestep(state, 1);
                installed = true;
            }
        })?;
    }
    if installed {
        Ok(())
    } else {
        Err(Error::runtime(
            "single-step mode requires the installed watchdog interrupt",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use mlua::{Lua, VmState};

    use super::enable;

    #[test]
    fn refuses_to_install_without_an_interrupt() {
        let lua = Lua::new();
        assert!(enable(&lua).is_err());
    }

    #[test]
    fn each_short_chunk_reaches_the_installed_interrupt() -> mlua::Result<()> {
        let lua = Lua::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        lua.set_interrupt(move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
            Ok(VmState::Continue)
        });
        enable(&lua)?;
        lua.load("local count = 1; count += 2; assert(count == 3)")
            .exec()?;
        assert!(
            calls.load(Ordering::Relaxed) > 0,
            "a short chunk must reach the step hook before any coarse VM interval"
        );
        Ok(())
    }
}
