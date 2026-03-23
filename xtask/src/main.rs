use std::path::{Path, PathBuf};
use std::process::Command;

use reaper_test::runner::{self, TestPackage, TestRunner};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("install") => install(),
        Some("uninstall") => uninstall(),
        Some("status") => fts_devtools::status(),
        Some("reaper-test") => {
            let filter = args.get(1).cloned();
            let keep_open = args.iter().any(|a| a == "--keep-open");
            if let Err(e) = reaper_test(filter, keep_open) {
                eprintln!("reaper-test failed: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("usage: cargo xtask <command>");
            eprintln!();
            eprintln!("commands:");
            eprintln!("  install       Build and symlink session-extension into REAPER");
            eprintln!("  uninstall     Remove session-extension symlink from REAPER");
            eprintln!("  status        Show installed extensions and plugins");
            eprintln!("  reaper-test   Run REAPER integration tests (session-extension only)");
            std::process::exit(1);
        }
    }
}

fn install() {
    // Build the extension
    let status = Command::new("cargo")
        .args(["build", "-p", "session-extension"])
        .status()
        .expect("failed to run cargo build");

    if !status.success() {
        eprintln!("cargo build failed");
        std::process::exit(1);
    }

    // Find the built binary
    let binary = target_dir().join("session-extension");
    fts_devtools::install_extension(&binary, "session")
        .expect("failed to install session extension");
}

fn uninstall() {
    fts_devtools::uninstall_extension("session");
}

fn target_dir() -> PathBuf {
    // Walk up from xtask dir to workspace root, then into target/debug
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().join("target").join("debug")
}

fn reaper_test(filter: Option<String>, keep_open: bool) -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let ci = std::env::var("CI").is_ok();
    let timeout_secs: u64 = std::env::var("REAPER_TEST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let resources_dir = runner::fts_reaper_resources();

    let runner = TestRunner {
        resources_dir: resources_dir.clone(),
        extension_log: PathBuf::from("/tmp/daw-bridge.log"),
        timeout_secs,
        keep_open,
        ci,
        // Only load session-extension — skip signal-extension, sync-extension, etc.
        extension_whitelist: vec!["session-extension".into()],
    };

    // ── Step 1: Build session-extension ──────────────────────────────
    runner::section(ci, "reaper-test: build session-extension");
    println!("Building session-extension...");
    let status = Command::new("cargo")
        .args(["build", "-p", "session-extension"])
        .current_dir(workspace_root)
        .status()?;
    if !status.success() {
        return Err("Failed to build session-extension".into());
    }
    runner::end_section(ci);

    // ── Step 2: Install into fts-extensions/ ─────────────────────────
    runner::section(ci, "reaper-test: install session-extension");
    let user_plugins_dir = resources_dir.join("UserPlugins");
    let fts_ext_dir = user_plugins_dir.join("fts-extensions");
    std::fs::create_dir_all(&fts_ext_dir)?;

    let ext_src = workspace_root.join("target/debug/session-extension");
    if ext_src.exists() {
        let ext_dst = fts_ext_dir.join("session-extension");
        std::fs::copy(&ext_src, &ext_dst)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ext_dst, std::fs::Permissions::from_mode(0o755))?;
        }
        println!("  Installed session-extension -> {}", ext_dst.display());
    } else {
        return Err(format!(
            "session-extension binary not found at {}",
            ext_src.display()
        )
        .into());
    }
    runner::end_section(ci);

    // ── Step 3: Build test binaries ──────────────────────────────────
    runner::section(ci, "reaper-test: build test binaries");
    println!("Building test binaries...");
    let status = Command::new("cargo")
        .args(["test", "-p", "session", "--no-run"])
        .current_dir(workspace_root)
        .status()?;
    if !status.success() {
        return Err("Failed to build session test binaries".into());
    }
    runner::end_section(ci);

    // ── Step 4: Clean, pre-warm, patch INI ───────────────────────────
    runner.clean_stale_sockets();
    runner.prewarm_reaper();
    runner.patch_ini();

    // ── Step 5: Spawn REAPER ─────────────────────────────────────────
    let mut reaper = runner.spawn_reaper()?;
    reaper.wait_for_socket(&runner)?;

    // ── Step 6: Run tests ────────────────────────────────────────────
    let packages = vec![TestPackage {
        package: "session".into(),
        features: vec![],
        test_threads: 1,
        default_skips: vec![],
    }];

    let tests_passed = runner.run_tests(&mut reaper, &packages, filter.as_deref())?;

    // ── Step 7: Cleanup and report ───────────────────────────────────
    if !tests_passed {
        reaper.report_failure(&runner);
        reaper.stop(&runner);
        return Err("Some tests failed".into());
    }

    reaper.stop(&runner);
    println!("\nAll session tests passed!");
    Ok(())
}
