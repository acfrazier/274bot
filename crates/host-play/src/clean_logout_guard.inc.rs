macro_rules! define_clean_logout_guard {
    ($play:ty, $deadline:expr) => {
        struct CleanLogoutGuard<'a> {
            play: &'a $play,
            account: &'a str,
            label: &'a str,
            attempted: bool,
        }

        impl CleanLogoutGuard<'_> {
            fn logout(&mut self) -> Result<(), String> {
                if self.attempted {
                    return Ok(());
                }
                self.attempted = true;
                self.play.script_stop(self.account);
                let arm = self
                    .play
                    .arm(self.account)
                    .ok_or_else(|| format!("{} has no slot arm for clean logout", self.label))?;
                if !self.play.slot_connected(self.account) || arm.session_key().is_none() {
                    return Err(format!(
                        "{} account {} was not connected to an active session for clean logout",
                        self.label, self.account
                    ));
                }
                arm.request_logout();
                self.play.wake(self.account);
                let deadline = ::std::time::Instant::now() + $deadline;
                while arm.session_key().is_some() || self.play.slot_connected(self.account) {
                    if ::std::time::Instant::now() >= deadline {
                        return Err(format!(
                            "{} account {} stayed connected after clean logout",
                            self.label, self.account
                        ));
                    }
                    ::std::thread::sleep(::std::time::Duration::from_millis(100));
                }
                println!("CLEAN LOGOUT label={} account={}", self.label, self.account);
                Ok(())
            }
        }

        impl Drop for CleanLogoutGuard<'_> {
            fn drop(&mut self) {
                if !self.attempted && self.play.slot_connected(self.account) {
                    let _ = self.logout();
                }
            }
        }
    };
}
