use mlua::{ffi, Error, Lua};

unsafe extern "C-unwind" fn step(state: *mut ffi::lua_State, _debug: *mut ffi::lua_Debug) {
    // SAFETY: Luau calls this hook with its currently executing, live state. Its
    // callback table belongs to that state. `install` writes this static hook
    // under mlua's lock before any coroutine exists; the VM-owned interrupt
    // outlives every entry. The callback's `-1` argument selects its guarded
    // non-GC path. No Rust value with a destructor crosses the C-unwind call or
    // `lua_error`. If the interrupt slot is absent, this hook raises before the
    // next script instruction can take effect.
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
