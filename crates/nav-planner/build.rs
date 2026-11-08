#[cfg(not(test))]
use std::path::PathBuf;

#[cfg(not(test))]
const ENCODER_PACKAGES: [(&str, &str); 4] = [
    ("png", "NAV_PNG_VERSION"),
    ("crc32fast", "NAV_CRC32FAST_VERSION"),
    ("fdeflate", "NAV_FDEFLATE_VERSION"),
    ("flate2", "NAV_FLATE2_VERSION"),
];

#[cfg(not(test))]
fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lock_path = manifest.join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock = std::fs::read_to_string(&lock_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", lock_path.display()));

    for (package, variable) in ENCODER_PACKAGES {
        let section = package_section(&lock, package, None);
        let version = quoted_field(section, "version").expect("locked encoder library version");
        println!("cargo:rustc-env={variable}={version}");
    }
    let miniz = flate_miniz_version(&lock);
    println!("cargo:rustc-env=NAV_MINIZ_OXIDE_VERSION={miniz}");
}

fn quoted_field<'a>(section: &'a str, field: &str) -> Option<&'a str> {
    section.lines().find_map(|line| {
        line.strip_prefix(field)?
            .strip_prefix(" = \"")?
            .strip_suffix('"')
    })
}

fn package_section<'a>(lock: &'a str, name: &str, version: Option<&str>) -> &'a str {
    let mut matches = lock.split("[[package]]").skip(1).filter(|section| {
        quoted_field(section, "name") == Some(name)
            && version.is_none_or(|version| quoted_field(section, "version") == Some(version))
    });
    let section = matches
        .next()
        .unwrap_or_else(|| panic!("Cargo.lock has no selected {name} package {version:?}"));
    assert!(
        matches.next().is_none(),
        "Cargo.lock has ambiguous {name} package {version:?}"
    );
    section
}

// PNG's high-compression encoder uses flate2. Other consumers can retain a
// different miniz_oxide version without changing this encoder backend.
fn flate_miniz_version(lock: &str) -> &str {
    let flate = package_section(lock, "flate2", None);
    let mut references = flate
        .lines()
        .skip_while(|line| line.trim() != "dependencies = [")
        .skip(1)
        .take_while(|line| line.trim() != "]")
        .filter_map(|line| line.trim().strip_prefix('"')?.strip_suffix("\","))
        .filter(|reference| reference.split_ascii_whitespace().next() == Some("miniz_oxide"));
    let reference = references
        .next()
        .expect("locked flate2 must identify its miniz_oxide backend");
    assert!(
        references.next().is_none(),
        "locked flate2 has ambiguous miniz_oxide backends"
    );
    let version = reference.split_ascii_whitespace().nth(1);
    quoted_field(package_section(lock, "miniz_oxide", version), "version")
        .expect("locked miniz_oxide version")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_BACKENDS: &str = r#"
[[package]]
name = "flate2"
version = "1.1.10"
dependencies = [
 "miniz_oxide 0.9.1",
]
[[package]]
name = "miniz_oxide"
version = "0.8.9"
[[package]]
name = "miniz_oxide"
version = "0.9.1"
"#;

    #[test]
    fn encoder_identity_follows_the_flate_dependency_not_an_unrelated_version() {
        assert_eq!(flate_miniz_version(TWO_BACKENDS), "0.9.1");
    }

    #[test]
    fn an_unqualified_dependency_requires_one_locked_version() {
        let lock = r#"
[[package]]
name = "flate2"
version = "1.1.9"
dependencies = [
 "miniz_oxide",
]
[[package]]
name = "miniz_oxide"
version = "0.8.9"
"#;
        assert_eq!(flate_miniz_version(lock), "0.8.9");
    }

    #[test]
    #[should_panic(expected = "ambiguous miniz_oxide package")]
    fn an_unqualified_dependency_cannot_select_from_two_versions() {
        flate_miniz_version(&TWO_BACKENDS.replace("miniz_oxide 0.9.1", "miniz_oxide"));
    }

    #[test]
    #[should_panic(expected = "no selected miniz_oxide package")]
    fn the_qualified_dependency_must_exist_in_the_lock() {
        flate_miniz_version(&TWO_BACKENDS.replace("miniz_oxide 0.9.1", "miniz_oxide 0.9.2"));
    }

    #[test]
    #[should_panic(expected = "must identify its miniz_oxide backend")]
    fn a_missing_backend_is_not_replaced_with_an_unrelated_package() {
        flate_miniz_version(&TWO_BACKENDS.replace("miniz_oxide 0.9.1", "other_decoder"));
    }
}
