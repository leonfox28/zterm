//! Compile-time release identity owned by the shared product crate.

fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown-target".to_owned());
    // Only the reviewed native-release workflow may opt a build into managed
    // distribution lifecycle operations. `GITHUB_SHA` is ambient in ordinary
    // CI and therefore is not evidence that a binary is an official Release.
    let source_commit =
        std::env::var("ZTERM_SOURCE_COMMIT").unwrap_or_else(|_| "development".to_owned());
    let classification = std::env::var("CARGO_PKG_VERSION").map_or("development", |version| {
        if version.contains('-') {
            "prerelease"
        } else {
            "stable"
        }
    });

    println!("cargo:rerun-if-env-changed=ZTERM_SOURCE_COMMIT");
    println!("cargo:rustc-env=ZTERM_BUILD_TARGET={target}");
    println!("cargo:rustc-env=ZTERM_SOURCE_COMMIT={source_commit}");
    println!("cargo:rustc-env=ZTERM_RELEASE_CLASSIFICATION={classification}");
}
