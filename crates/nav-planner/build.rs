use std::path::PathBuf;

const ENCODER_PACKAGES: [(&str, &str); 5] = [
    ("png", "NAV_PNG_VERSION"),
    ("crc32fast", "NAV_CRC32FAST_VERSION"),
    ("fdeflate", "NAV_FDEFLATE_VERSION"),
    ("flate2", "NAV_FLATE2_VERSION"),
    ("miniz_oxide", "NAV_MINIZ_OXIDE_VERSION"),
];

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lock_path = manifest.join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock = std::fs::read_to_string(&lock_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", lock_path.display()));

    for (package, variable) in ENCODER_PACKAGES {
        let versions: Vec<_> = lock
            .split("[[package]]")
            .skip(1)
            .filter(|section| quoted_field(section, "name") == Some(package))
            .filter_map(|section| quoted_field(section, "version"))
            .collect();
        let [version] = versions.as_slice() else {
            panic!(
                "Cargo.lock must contain exactly one {package} package, found {}",
                versions.len()
            );
        };
        println!("cargo:rustc-env={variable}={version}");
    }
}

fn quoted_field<'a>(section: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field} = \"");
    section
        .lines()
        .find_map(|line| line.strip_prefix(&prefix)?.strip_suffix('"'))
}
