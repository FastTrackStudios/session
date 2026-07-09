//! `kf` — thin binary over the `keyflow-cli` library.
//!
//! The same command tree is mounted at `fts keyflow <...>` (alias
//! `fts kf`) by the unified `fts` CLI.

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    keyflow_cli::cli_main(std::env::args());
}
