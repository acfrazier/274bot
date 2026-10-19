//! Vault unlock with the passphrase as typed.

#[test]
fn a_vault_opens_with_the_typed_or_the_legacy_trimmed_passphrase() {
    let dir = std::env::temp_dir().join(format!(
        "fc-vault-typed-unlock-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    // Made in the terminal with the spaces kept: only the typed text opens it.
    let terminal = dir.join("terminal");
    drop(vault::Vault::create(&terminal, " pad ").unwrap());
    assert!(super::open_vault_typed(&terminal, " pad ").is_ok());

    // Made by the 0.1.9.1 panel, which stored the trimmed text.
    let legacy = dir.join("legacy");
    drop(vault::Vault::create(&legacy, "pad").unwrap());
    assert!(super::open_vault_typed(&legacy, "  pad ").is_ok());

    assert!(matches!(
        super::open_vault_typed(&legacy, " nope "),
        Err(vault::VaultError::WrongPassphrase)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}
