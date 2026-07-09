use std::path::PathBuf;
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("install") => install(),
        Some("uninstall") => uninstall(),
        Some("status") => fts_devtools::status(),
        Some("ci") => {
            let cfg = fts_repo::XtaskConfig {
                nextest_profile: "ci".to_string(),
                run_doctests: false,
                run_tracey: false,
                ..fts_repo::XtaskConfig::default()
            };
            if let Err(e) = fts_repo::run_ci(&cfg) {
                eprintln!("ERROR: {e:#}");
                std::process::exit(1);
            }
        }
        Some("check") => {
            if let Err(e) = fts_repo::run_check() {
                eprintln!("ERROR: {e:#}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("usage: cargo xtask <command>");
            eprintln!();
            eprintln!("commands:");
            eprintln!("  install     Build and symlink dynamic-template-extension into REAPER");
            eprintln!("  uninstall   Remove dynamic-template-extension symlink from REAPER");
            eprintln!("  status      Show installed extensions and plugins");
            eprintln!("  ci          Run the shared FTS CI gate (fmt + clippy + check + nextest)");
            eprintln!("  check       cargo check --workspace --all-targets");
            std::process::exit(1);
        }
    }
}

fn install() {
    // Build the extension
    let status = Command::new("cargo")
        .args(["build", "-p", "dynamic-template-extension"])
        .status()
        .expect("failed to run cargo build");

    if !status.success() {
        eprintln!("cargo build failed");
        std::process::exit(1);
    }

    // Find the built binary
    let binary = target_dir().join("dynamic-template");
    fts_devtools::install_extension(&binary, "dynamic-template")
        .expect("failed to install dynamic-template extension");
}

fn uninstall() {
    fts_devtools::uninstall_extension("dynamic-template");
}

fn target_dir() -> PathBuf {
    // Walk up from xtask dir to workspace root, then into target/debug
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().join("target").join("debug")
}
