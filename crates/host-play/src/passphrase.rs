//! How a front end obtains the vault passphrase.
//!
//! Never from the environment and never from the command line: both are
//! readable by every other local user (`ps`, `/proc/<pid>/environ`, `ps eww`)
//! and inherited by every child process. The two channels are
//!
//! - a **hidden prompt** on the terminal, used whenever standard input is a
//!   terminal (a new vault asks twice), and
//! - **standard input**, one line, when the front end is started with
//!   `--vault-pass-stdin` and stdin is a pipe or file (launchers, CI,
//!   harnesses: `printf '%s\n' "$PASS" | tui-play --vault-pass-stdin`).
//!
//! The panel additionally has its in-window prompt. `BOT_VAULT_PASS` and
//! `--vault-pass PASS` are gone with no alias: [`legacy_env_notice`] tells an
//! operator whose shell still exports the variable that it is ignored, and
//! [`removed_flag_error`] rejects the flag without echoing the value.

use std::io::{BufRead, IsTerminal, Write};

use vault::{check_new_passphrase, Secret};
use zeroize::Zeroizing;

/// The variable that used to carry the passphrase. It is no longer read.
pub const LEGACY_ENV: &str = "BOT_VAULT_PASS";

/// The longest passphrase either channel accepts. The vault itself accepts any
/// length; this bounds what is read from a pipe or typed into the prompt.
pub const MAX_PASSPHRASE_BYTES: usize = 4096;

/// How many times a prompt for a new passphrase asks again after a rejected
/// or mismatched entry before giving up.
const MAX_ATTEMPTS: usize = 3;

/// Why the passphrase is being read: an existing vault is opened with whatever
/// its passphrase is; a new vault's passphrase must meet the floor and is
/// entered twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Unlock,
    Create,
}

impl Purpose {
    /// `Create` when no vault exists at the path yet, else `Unlock`.
    pub fn for_vault(exists: bool) -> Self {
        if exists {
            Self::Unlock
        } else {
            Self::Create
        }
    }
}

/// Reads the passphrase for `purpose` through the channel the process has: the
/// hidden prompt when stdin is a terminal, else one line of stdin when
/// `from_stdin` (`--vault-pass-stdin`) was given, else an error saying so.
/// `program` names the caller in prompts and notes printed while asking; the
/// returned error is for the caller to print under its own name.
pub fn obtain(program: &str, from_stdin: bool, purpose: Purpose) -> Result<Secret, String> {
    if std::io::stdin().is_terminal() {
        return ask(program, purpose, &mut read_hidden, &mut std::io::stderr());
    }
    if !from_stdin {
        return Err(
            "no terminal to ask for the vault passphrase on; pipe it in and pass \
             --vault-pass-stdin"
                .into(),
        );
    }
    let secret =
        read_line_from(std::io::stdin().lock()).map_err(|e| format!("vault passphrase: {e}"))?;
    if purpose == Purpose::Create {
        check_new_passphrase(&secret).map_err(|e| e.to_string())?;
    }
    Ok(secret)
}

/// The prompting policy over an injected `read` (one hidden line per call,
/// given its prompt) and note sink, so it can be tested without a terminal.
fn ask(
    program: &str,
    purpose: Purpose,
    read: &mut dyn FnMut(&str) -> Result<Secret, String>,
    notes: &mut dyn Write,
) -> Result<Secret, String> {
    if purpose == Purpose::Unlock {
        return read("Vault passphrase: ");
    }
    for _ in 0..MAX_ATTEMPTS {
        let first = read(&format!(
            "New vault passphrase (at least {} characters): ",
            vault::MIN_PASSPHRASE_CHARS
        ))?;
        if let Err(e) = check_new_passphrase(&first) {
            let _ = writeln!(notes, "{program}: {e}");
            continue;
        }
        let second = read("Repeat the passphrase: ")?;
        if first == second {
            return Ok(first);
        }
        let _ = writeln!(notes, "{program}: the passphrases do not match");
    }
    Err(format!(
        "no valid new vault passphrase after {MAX_ATTEMPTS} attempts"
    ))
}

/// One line from `reader`: everything before the first `\n`, with a `\r\n` or
/// `\n` line ending removed and nothing else trimmed. Refuses an empty line,
/// end of input before any byte, a line longer than [`MAX_PASSPHRASE_BYTES`],
/// and text that is not UTF-8. Only the first line is consumed.
pub fn read_line_from(reader: impl BufRead) -> Result<Secret, String> {
    let mut raw = Zeroizing::new(Vec::with_capacity(128));
    // Room for the longest passphrase and its `\r\n`.
    reader
        .take(MAX_PASSPHRASE_BYTES as u64 + 2)
        .read_until(b'\n', &mut raw)
        .map_err(|e| format!("cannot read standard input: {e}"))?;
    if raw.is_empty() {
        return Err("standard input closed before a passphrase was sent".into());
    }
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
    if raw.is_empty() {
        return Err("the passphrase line is empty".into());
    }
    if raw.len() > MAX_PASSPHRASE_BYTES {
        return Err(format!(
            "the passphrase is longer than {MAX_PASSPHRASE_BYTES} bytes"
        ));
    }
    match String::from_utf8(std::mem::take(&mut *raw)) {
        Ok(text) => Ok(Secret::from(text)),
        Err(bad) => {
            drop(Zeroizing::new(bad.into_bytes()));
            Err("the passphrase is not valid UTF-8".into())
        }
    }
}

/// Reads one hidden line from the terminal: raw mode from before the prompt is
/// shown (so input sent the moment it appears is never echoed) until the line
/// ends. Enter (`\r`, or `\n` which raw mode reports as Ctrl-J) ends it,
/// Backspace edits, Ctrl-U clears, Ctrl-C, Ctrl-D and Esc cancel. The terminal
/// is restored on every path.
fn read_hidden(prompt: &str) -> Result<Secret, String> {
    use crossterm::event::{poll, read, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use std::time::Duration;

    struct RawGuard;
    impl Drop for RawGuard {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }

    crossterm::terminal::enable_raw_mode()
        .map_err(|e| format!("cannot read from the terminal: {e}"))?;
    let _restore = RawGuard;
    let mut err = std::io::stderr();
    let _ = write!(err, "{prompt}");
    let _ = err.flush();

    let mut typed = Secret::with_capacity(256);
    let outcome = loop {
        let event = match read() {
            Ok(event) => event,
            Err(e) => break Err(format!("cannot read from the terminal: {e}")),
        };
        let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) = event
        else {
            continue;
        };
        // AltGr arrives as Ctrl+Alt on Windows and types a character.
        let ctrl =
            modifiers.contains(KeyModifiers::CONTROL) && !modifiers.contains(KeyModifiers::ALT);
        match code {
            KeyCode::Enter => break Ok(true),
            KeyCode::Char('j' | 'm') if ctrl => break Ok(false),
            KeyCode::Esc => break Err("cancelled".to_string()),
            KeyCode::Char('c' | 'd') if ctrl => break Err("cancelled".to_string()),
            KeyCode::Char('u') if ctrl => typed.clear(),
            KeyCode::Backspace => {
                typed.pop();
            }
            KeyCode::Char(c) if !ctrl => typed.push(c),
            _ => {}
        }
    };
    let _ = write!(err, "\r\n");
    let _ = err.flush();
    match outcome {
        Ok(carriage_return) => {
            // A CRLF pair arrives as two events; swallow the second so it is
            // not left to whatever reads the terminal next.
            if carriage_return && poll(Duration::from_millis(10)).unwrap_or(false) {
                let _ = read();
            }
            Ok(typed)
        }
        Err(e) => Err(e),
    }
}

/// The notice for an operator whose environment still carries
/// [`LEGACY_ENV`]: it is ignored (not honoured as an alias), and leaving it
/// exported keeps the passphrase visible to other local users and every child
/// process. `lookup` is the environment, injected for tests.
pub fn legacy_env_notice(
    program: &str,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<String> {
    lookup(LEGACY_ENV).map(|_| {
        format!(
            "{program}: {LEGACY_ENV} is set but is no longer read. Unset it: an exported \
             passphrase is visible to other local users and to every child process. Enter the \
             passphrase at the prompt, or pipe it in with --vault-pass-stdin"
        )
    })
}

/// Prints [`legacy_env_notice`] for the real environment to stderr.
pub fn warn_legacy_env(program: &str) {
    if let Some(notice) = legacy_env_notice(program, |name| std::env::var_os(name)) {
        eprintln!("{notice}");
    }
}

/// True for `--vault-pass` and `--vault-pass=VALUE`, the removed flag.
pub fn is_removed_flag(arg: &str) -> bool {
    arg == "--vault-pass" || arg.starts_with("--vault-pass=")
}

/// The error for the removed flag. It never repeats the flag's value.
pub fn removed_flag_error(program: &str) -> String {
    format!(
        "{program}: --vault-pass was removed: a passphrase on the command line is visible to \
         every local user in the process list. Enter it at the prompt, or pipe it in and pass \
         --vault-pass-stdin"
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// What the child processes below expect to be typed or piped.
    const TYPED: &str = "correct horse battery";

    fn line(input: &[u8]) -> Result<Secret, String> {
        read_line_from(Cursor::new(input.to_vec()))
    }

    #[test]
    fn a_line_loses_its_ending_and_nothing_else() {
        assert_eq!(line(b"open sesame 123\n").unwrap(), "open sesame 123");
        assert_eq!(line(b"open sesame 123\r\n").unwrap(), "open sesame 123");
        assert_eq!(line(b"open sesame 123").unwrap(), "open sesame 123");
        assert_eq!(line(b"  padded  \n").unwrap(), "  padded  ");
        assert_eq!(line("口令口令\n".as_bytes()).unwrap(), "口令口令");
    }

    #[test]
    fn only_the_first_line_is_consumed() {
        let mut input = Cursor::new(b"first line\nsecond line\n".to_vec());
        assert_eq!(read_line_from(&mut input).unwrap(), "first line");
        assert_eq!(read_line_from(&mut input).unwrap(), "second line");
    }

    #[test]
    fn empty_and_closed_input_are_refused_with_distinct_reasons() {
        assert!(line(b"").unwrap_err().contains("closed"));
        assert!(line(b"\n").unwrap_err().contains("empty"));
        assert!(line(b"\r\n").unwrap_err().contains("empty"));
    }

    #[test]
    fn the_length_bound_is_exact_and_needs_no_newline_to_trigger() {
        let at = "a".repeat(MAX_PASSPHRASE_BYTES);
        assert_eq!(line(format!("{at}\n").as_bytes()).unwrap().len(), at.len());
        assert_eq!(
            line(format!("{at}\r\n").as_bytes()).unwrap().len(),
            at.len()
        );
        assert_eq!(line(at.as_bytes()).unwrap().len(), at.len());
        let over = "a".repeat(MAX_PASSPHRASE_BYTES + 1);
        assert!(line(format!("{over}\n").as_bytes())
            .unwrap_err()
            .contains("longer than"));
        // An endless line without a newline is cut off, not read whole.
        let endless = std::io::repeat(b'a');
        assert!(read_line_from(std::io::BufReader::new(endless))
            .unwrap_err()
            .contains("longer than"));
    }

    #[test]
    fn text_that_is_not_utf8_is_refused() {
        assert!(line(b"caf\xe9 au lait 123\n")
            .unwrap_err()
            .contains("UTF-8"));
    }

    /// A scripted terminal: hands out `entries` in order and records prompts.
    fn scripted(
        entries: &[&str],
        purpose: Purpose,
    ) -> (Result<Secret, String>, Vec<String>, String) {
        let mut entries = entries.iter();
        let mut prompts = Vec::new();
        let mut messages = Vec::new();
        let result = ask(
            "tui-play",
            purpose,
            &mut |prompt: &str| {
                prompts.push(prompt.to_string());
                entries
                    .next()
                    .map(|entry| Secret::from(*entry))
                    .ok_or_else(|| "cancelled".to_string())
            },
            &mut messages,
        );
        (result, prompts, String::from_utf8(messages).unwrap())
    }

    #[test]
    fn unlocking_asks_once_and_applies_no_floor() {
        let (result, prompts, _) = scripted(&["bot"], Purpose::Unlock);
        assert_eq!(result.unwrap(), "bot");
        assert_eq!(prompts, ["Vault passphrase: "]);
    }

    #[test]
    fn a_new_passphrase_is_asked_twice_and_must_match() {
        let (result, prompts, _) = scripted(&[TYPED, TYPED], Purpose::Create);
        assert_eq!(result.unwrap(), TYPED);
        assert_eq!(prompts.len(), 2);
        assert!(prompts[0].contains("at least 12 characters"), "{prompts:?}");

        let (result, _, messages) =
            scripted(&[TYPED, "typo typo typo", TYPED, TYPED], Purpose::Create);
        assert_eq!(result.unwrap(), TYPED, "a mismatch asks again");
        assert!(messages.contains("do not match"), "{messages}");
    }

    #[test]
    fn a_too_short_new_passphrase_is_explained_and_asked_again_before_the_repeat() {
        let (result, prompts, messages) = scripted(&["short", TYPED, TYPED], Purpose::Create);
        assert_eq!(result.unwrap(), TYPED);
        assert!(
            messages.contains("at least 12 characters (got 5)"),
            "{messages}"
        );
        assert_eq!(prompts.len(), 3, "the short entry is not repeated back");
    }

    #[test]
    fn a_new_passphrase_gives_up_after_three_bad_attempts() {
        let (result, _, messages) = scripted(&["a", "b", "c", TYPED, TYPED], Purpose::Create);
        assert!(result.unwrap_err().contains("after 3 attempts"));
        assert_eq!(messages.matches("at least 12").count(), 3, "{messages}");
    }

    #[test]
    fn a_cancelled_prompt_is_an_error() {
        let (result, _, _) = scripted(&[], Purpose::Unlock);
        assert_eq!(result.unwrap_err(), "cancelled");
    }

    #[test]
    fn the_legacy_variable_is_reported_and_never_honoured() {
        let notice = legacy_env_notice("host-play", |name| {
            (name == LEGACY_ENV).then(|| "hunter2".into())
        })
        .expect("a set variable is reported");
        assert!(notice.contains("no longer read"), "{notice}");
        assert!(!notice.contains("hunter2"), "the value is never echoed");
        assert_eq!(legacy_env_notice("host-play", |_| None), None);
    }

    #[test]
    fn the_removed_flag_is_recognised_in_both_spellings_and_never_echoed() {
        assert!(is_removed_flag("--vault-pass"));
        assert!(is_removed_flag("--vault-pass=hunter2"));
        assert!(!is_removed_flag("--vault-pass-stdin"));
        assert!(!is_removed_flag("--vault"));
        assert!(!removed_flag_error("host-play").contains("hunter2"));
    }

    /// The child side of the process tests: only acts when its parent asks
    /// (the pattern `instance_lock` uses), and reports whether it received
    /// [`TYPED`] without ever printing the passphrase itself.
    #[test]
    fn passphrase_child_entry() {
        let Ok(mode) = std::env::var("HOST_PLAY_PASSPHRASE_TEST_CHILD") else {
            return;
        };
        let (purpose, from_stdin) = match mode.as_str() {
            "stdin-create" => (Purpose::Create, true),
            "stdin" => (Purpose::Unlock, true),
            "terminal-create" => (Purpose::Create, false),
            _ => (Purpose::Unlock, false),
        };
        match obtain("child", from_stdin, purpose) {
            Ok(secret) => println!("CHILD-RESULT match={}", secret == TYPED),
            Err(error) => println!("CHILD-RESULT error={error}"),
        }
    }

    #[cfg(unix)]
    mod processes {
        use std::io::{Read, Write};
        use std::os::fd::{FromRawFd, OwnedFd};
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        use super::TYPED;

        fn child(mode: &str) -> Command {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "passphrase::tests::passphrase_child_entry",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("HOST_PLAY_PASSPHRASE_TEST_CHILD", mode);
            command
        }

        /// A child attached to a fresh pseudo-terminal as its controlling
        /// terminal, plus the parent's end of it.
        fn on_a_terminal(mode: &str) -> (std::process::Child, std::fs::File) {
            let (mut master, mut slave) = (0, 0);
            // SAFETY: both descriptor out-pointers are valid; the other
            // arguments are optional and null.
            let opened = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(opened, 0, "openpty");
            // SAFETY: openpty returned these descriptors and nothing else owns them.
            let (master, slave) =
                unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
            let mut command = child(mode);
            command.stdin(Stdio::from(slave.try_clone().unwrap()));
            command.stdout(Stdio::from(slave.try_clone().unwrap()));
            command.stderr(Stdio::from(slave.try_clone().unwrap()));
            // SAFETY: only async-signal-safe calls between fork and exec.
            unsafe {
                command.pre_exec(|| {
                    libc::setsid();
                    libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                    Ok(())
                });
            }
            let spawned = command.spawn().expect("spawn the terminal child");
            drop(command);
            drop(slave);
            let master = std::fs::File::from(master);
            set_nonblocking(&master);
            (spawned, master)
        }

        fn set_nonblocking(file: &std::fs::File) {
            use std::os::fd::AsRawFd;
            // SAFETY: fcntl on a descriptor this test owns.
            unsafe {
                let flags = libc::fcntl(file.as_raw_fd(), libc::F_GETFL);
                libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }

        /// Appends what the terminal shows to `seen` until `done(seen)` holds,
        /// the child closes the terminal, or a minute passes (a failure).
        fn read_until(
            master: &mut std::fs::File,
            seen: &mut String,
            what: &str,
            done: impl Fn(&str) -> bool,
        ) {
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut buf = [0u8; 4096];
            while !done(seen) {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {what}: {seen:?}"
                );
                match master.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    // Linux reports EIO once the child has closed its side.
                    Err(_) => break,
                }
            }
        }

        fn until_shown(master: &mut std::fs::File, seen: &mut String, text: &str) {
            read_until(master, seen, text, |seen| seen.contains(text));
        }

        /// Waits for the child's whole result line and its exit, then takes what
        /// is still buffered on the terminal, so nothing it displayed is missed.
        fn until_reported(
            spawned: &mut std::process::Child,
            master: &mut std::fs::File,
            seen: &mut String,
        ) {
            read_until(master, seen, "the child's result line", |seen| {
                seen.find("CHILD-RESULT")
                    .is_some_and(|at| seen[at..].contains('\n'))
            });
            let _ = spawned.wait();
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = master.read(&mut buf) {
                seen.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }

        /// Types `keys` at the terminal once the prompt shows and returns
        /// everything the terminal displayed.
        fn type_at_prompt(mode: &str, prompt: &str, keys: &[u8]) -> String {
            let (mut spawned, mut master) = on_a_terminal(mode);
            let mut seen = String::new();
            until_shown(&mut master, &mut seen, prompt);
            master.write_all(keys).unwrap();
            until_reported(&mut spawned, &mut master, &mut seen);
            seen
        }

        #[test]
        fn the_terminal_prompt_reads_the_passphrase_and_echoes_none_of_it() {
            for ending in ["\r", "\n", "\r\n"] {
                let typed = format!("{TYPED}{ending}");
                let seen = type_at_prompt("terminal", "Vault passphrase: ", typed.as_bytes());
                assert!(
                    seen.contains("CHILD-RESULT match=true"),
                    "{ending:?}: {seen:?}"
                );
                assert!(
                    !seen.contains("horse") && !seen.contains(TYPED),
                    "the terminal must not echo the passphrase: {seen:?}"
                );
            }
        }

        #[test]
        fn backspace_edits_and_ctrl_u_clears_without_echo() {
            // "wrong" is typed and erased, ctrl-U drops "zzz", then the real one.
            let keys = format!("wrong\x7f\x7f\x7f\x7f\x7fzzz\x15{TYPED}\r");
            let seen = type_at_prompt("terminal", "Vault passphrase: ", keys.as_bytes());
            assert!(seen.contains("CHILD-RESULT match=true"), "{seen:?}");
            assert!(!seen.contains("wrong") && !seen.contains("zzz"), "{seen:?}");
        }

        #[test]
        fn ctrl_c_cancels_and_leaves_the_terminal_usable() {
            let seen = type_at_prompt("terminal", "Vault passphrase: ", b"abc\x03");
            assert!(seen.contains("CHILD-RESULT error=cancelled"), "{seen:?}");
        }

        #[test]
        fn a_new_vault_on_a_terminal_asks_for_the_passphrase_twice() {
            let (mut spawned, mut master) = on_a_terminal("terminal-create");
            let mut seen = String::new();
            until_shown(&mut master, &mut seen, "(at least 12 characters): ");
            master.write_all(format!("{TYPED}\r").as_bytes()).unwrap();
            until_shown(&mut master, &mut seen, "Repeat the passphrase: ");
            master.write_all(format!("{TYPED}\r").as_bytes()).unwrap();
            until_reported(&mut spawned, &mut master, &mut seen);
            assert!(seen.contains("CHILD-RESULT match=true"), "{seen:?}");
            assert!(!seen.contains("horse"), "{seen:?}");
        }

        /// A child whose stdin is a pipe, never a terminal.
        fn piped(mode: &str, input: &[u8]) -> String {
            let mut spawned = child(mode)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            spawned.stdin.take().unwrap().write_all(input).unwrap();
            let output = spawned.wait_with_output().unwrap();
            String::from_utf8_lossy(&output.stdout).into_owned()
        }

        #[test]
        fn a_piped_passphrase_is_read_from_stdin_only_when_asked_for() {
            let asked = piped("stdin", format!("{TYPED}\n").as_bytes());
            assert!(asked.contains("CHILD-RESULT match=true"), "{asked}");

            // Without --vault-pass-stdin a pipe is never read as a passphrase.
            let unasked = piped("no-flag", format!("{TYPED}\n").as_bytes());
            assert!(
                unasked.contains("CHILD-RESULT error=no terminal"),
                "{unasked}"
            );
            assert!(unasked.contains("--vault-pass-stdin"), "{unasked}");
        }

        #[test]
        fn a_piped_new_passphrase_must_meet_the_floor() {
            let short = piped("stdin-create", b"short\n");
            assert!(short.contains("at least 12 characters (got 5)"), "{short}");
            let fine = piped("stdin-create", format!("{TYPED}\n").as_bytes());
            assert!(fine.contains("CHILD-RESULT match=true"), "{fine}");
        }
    }
}
