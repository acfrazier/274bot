//! The account identity a bank hint is keyed by (design-bank-snapshot
//! §1.4): the slot username as the client's login identity spells it
//! ([`account_component`]), so one account is one hint however the
//! operator typed it.
//!
//! The hint itself — one account's last-seen bank rows on disk, so
//! planning works before any bank is opened in a new process — is the
//! `bank-hints` domain of the process local-state service (T5.2,
//! `host-play`'s `local_state`), reached through the slot's
//! [`crate::local_state::SlotState`]. Nothing in `script` resolves a hint
//! path or reads or writes a hint file. The same rule, with the profile
//! component, is `api::local_state::AccountKey`; this entry keeps the
//! account half for the host's memory key, which has no profile.

/// The most characters of a login name. The login block carries the name
/// as typed (`Client::write_login_block`), the server refuses a longer one
/// before it decodes anything (`World.ts`, `username.length > 12` → reply
/// 3), and the client's login hash (`JString.toUserhash`) reads at most
/// this many.
pub const MAX_ACCOUNT_CHARS: usize = 12;

/// The name is not a game login name ([`account_component`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "account {0:?} is not a login name (at most {MAX_ACCOUNT_CHARS} letters, digits, spaces \
     and underscores as typed, with a letter or digit among them)"
)]
pub struct UnsafeAccount(pub String);

/// The account name as the one path component the client's login identity
/// gives it, or why it cannot be one. A name is refused on its raw
/// spelling first, before anything is trimmed or hashed: one with any
/// byte outside ASCII letters, digits, space and underscore (the hash
/// would fold it to a separator and log into some other account), and one
/// longer than [`MAX_ACCOUNT_CHARS`] as typed — the login block carries
/// the raw name and the server refuses a longer one, so `ab3456789xyz1`,
/// `_ab3456789xyz` and `_alice_smith_` (thirteen) never log in and get no
/// hint; a refused name means no load and no save for that slot. An
/// accepted name's identity is what the client logs it in as, computed by
/// the client's own decoder: `JString.toRawUsername(JString.toUserhash(name))`
/// — a whitespace trim, the characters hashed base-37 (case folded, a
/// space and an underscore both the zero digit, so leading zeros vanish
/// and the name ends at its last letter or digit), decoded back. So
/// `Alice Smith`, `alice_smith` and ` alice smith` are one account and get
/// one file, `alice_smith.json`, and `_ab3456789xy` logs in as
/// `ab3456789xy`. A name the hash spells as no account at all (no letter
/// or digit, e.g. `____________`) is refused after the hash.
pub fn account_component(account: &str) -> Result<String, UnsafeAccount> {
    let unsafe_account = || UnsafeAccount(account.to_owned());
    if !account
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        return Err(unsafe_account());
    }
    // The admitted alphabet is ASCII, so bytes are characters: the raw
    // length, spaces and underscores included, is what the server sees.
    if account.len() > MAX_ACCOUNT_CHARS {
        return Err(unsafe_account());
    }
    // The decoder answers `invalid_name` for a hash that spells no account;
    // of what `to_userhash` can return, that is exactly zero.
    let identity = api::bank_memory::login_identity(account).ok_or_else(unsafe_account)?;
    debug_assert!(
        vault::valid_component(&identity),
        "the decoder spells {account:?} as {identity:?}"
    );
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the client logs the name in as: `toRawUsername(toUserhash(name))`.
    fn client_login_name(name: &str) -> String {
        let hash = client::util::JString::to_userhash(name);
        client::util::JString::to_raw_username(i64::try_from(hash).unwrap())
    }

    #[test]
    fn account_component_spells_the_name_as_the_client_login_identity() {
        // Every accepted spelling is the client's own login identity: case
        // folds, a space is an underscore, surrounding separators go, and
        // leading separators spend the twelve-character budget.
        let twelve = "ab3456789xyZ";
        assert_eq!(twelve.len(), MAX_ACCOUNT_CHARS);
        for (typed, file) in [
            ("alice", "alice"),
            ("Alice Smith", "alice_smith"),
            ("alice_smith", "alice_smith"),
            ("ALICE_SMITH", "alice_smith"),
            (" alice smith", "alice_smith"),
            ("alice smith_", "alice_smith"),
            ("_alice_smith", "alice_smith"),
            ("bm4h30nbov_0", "bm4h30nbov_0"),
            ("Bob_2", "bob_2"),
            ("a  b", "a__b"),
            (twelve, "ab3456789xyz"),
            // Surrounding whitespace within the twelve is trimmed.
            (" b3456789xyZ", "b3456789xyz"),
            // REVIEW-BANK-SNAPSHOT-S2-R2 M1-R2: a leading underscore is one
            // of the twelve and the hash drops it, as it does at the
            // client's login.
            ("_ab3456789xy", "ab3456789xy"),
            ("__ab3456789x", "ab3456789x"),
        ] {
            assert!(typed.len() <= MAX_ACCOUNT_CHARS, "{typed:?}");
            assert_eq!(account_component(typed).unwrap(), file, "{typed:?}");
            assert_eq!(
                account_component(typed).unwrap(),
                client_login_name(typed),
                "{typed:?}: the file name is the client's login spelling"
            );
            // The frozen account key spells the same identity.
            assert_eq!(
                api::local_state::AccountKey::try_new("local-289", typed)
                    .unwrap()
                    .account(),
                file,
                "{typed:?}: one rule for the host's memory key and the durable key"
            );
        }
    }

    #[test]
    fn account_component_rejects_hostile_overlength_and_no_account_names() {
        // Refused before the hash sees them: the client would fold each of
        // these characters to a separator and log into some other account.
        for hostile in [
            "a/b",
            "a\\b",
            "..",
            ".",
            "../alice",
            "x-y.z",
            "a\0b",
            "a\x1bb",
            "tab\tname",
            "del\x7f",
            "élan",
        ] {
            assert_eq!(
                account_component(hostile),
                Err(UnsafeAccount(hostile.to_string())),
                "{hostile:?} must be refused"
            );
            assert!(api::local_state::AccountKey::try_new("local-289", hostile).is_err());
        }
        // REVIEW-BANK-SNAPSHOT-S2-R3 M1-R3: refused on the raw length,
        // before any trim or hash. The login block carries the name as
        // typed and the server refuses more than twelve, so none of these
        // logs in, however the decoder would have shortened it.
        for long in [
            "ab3456789xyz1",
            "_ab3456789xyz",
            "__ab3456789xyz",
            "_alice_smith_",
            " ab3456789xyZ ",
            "             ",
            "____________a",
        ] {
            assert!(long.len() > MAX_ACCOUNT_CHARS, "{long:?}");
            assert_eq!(
                account_component(long),
                Err(UnsafeAccount(long.to_string())),
                "{long:?} must be refused"
            );
            assert!(api::local_state::AccountKey::try_new("local-289", long).is_err());
        }
        // Refused after the hash: the client's decoder spells these as no
        // account (`invalid_name`).
        for empty in ["", " ", "___", "_ _", "____________"] {
            assert_eq!(client_login_name(empty), "invalid_name", "{empty:?}");
            assert_eq!(
                account_component(empty),
                Err(UnsafeAccount(empty.to_string())),
                "{empty:?} must be refused"
            );
            assert!(api::local_state::AccountKey::try_new("local-289", empty).is_err());
        }
    }
}
