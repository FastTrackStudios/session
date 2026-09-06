//! Test-only REAPER extension for the `session` library crate.
//!
//! This is not a production sidecar. It is a small in-process REAPER host used
//! by this repo's integration tests to mount session without loading the full
//! `fts-extensions` plugin.
//!
//! It mounts session through [`session::host`] — the same three calls
//! `fts-extensions` makes — so what a test drives is what production drives.
//! Everything else here is test scaffolding with no production counterpart:
//! the `FTS_SESSION_EXT` health beacon and the LAN `/vox` server on an
//! OS-assigned port.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use daw::module;
use daw::service::ExtState;
use daw_extension_runtime::ExtensionRuntime;
use fragile::Fragile;
use reaper_low::PluginContext;
use reaper_macros::reaper_extension_plugin;
use tracing::{info, warn};

thread_local! {
    static APP: OnceCell<Fragile<TestExtension>> = const { OnceCell::new() };
}

type ActionHandler = Arc<dyn Fn() + Send + Sync>;

struct TestExtension {
    runtime: ExtensionRuntime,
    action_rx: crossbeam_channel::Receiver<String>,
    action_handlers: HashMap<String, ActionHandler>,
}

impl TestExtension {
    fn new(context: PluginContext) -> eyre::Result<Self> {
        let runtime = ExtensionRuntime::new(context)?;

        // `runtime.build_daw()` mounts only the generic daw services
        // (`create_daw_handler()`, unmodified). `session::host::layer_router`
        // layers session's own RPC surface on top — `SetlistServiceImpl<
        // daw_reaper::Reaper>` plus the mode / take-ranking / record-control
        // surfaces — so this test extension stands in for `fts-extensions`
        // without pulling in any of its other (tempo/mirror/expression-editor/
        // ...) modules. Published on the same `/tmp/fts-daw-{pid}.sock`
        // `daw::test` already waits on, so a `#[reaper_test]`'s `ctx.daw`
        // reaches these services, and a raw `SetlistServiceClient`/
        // `SetlistServiceStreamClient` (opened the same way
        // `session-desktop`'s Recording Mode does) reaches them too.
        //
        // It has to happen inside this one `block_on`: the control surfaces
        // spawn a pump task (`tokio::spawn`) while building
        // `SessionModeServiceImpl`, which needs a tokio runtime *entered* —
        // plugin_main's bare OS thread has none, and building the router
        // outside panicked with "there is no reactor running".
        info!(
            main_thread_executor_installed = daw::main_thread::is_installed(),
            "session test extension: after ExtensionRuntime::new"
        );
        let _daw = runtime
            .handle()
            .block_on(async {
                let handler = daw_reaper::create_daw_handler();
                let handler = session::host::layer_router(handler, daw_reaper::Reaper);
                daw_reaper::socket_publisher::publish_extension_socket(handler.clone());
                // LAN test server: the exact `architect::axum_ws::serve_router`
                // path `session-desktop --engine` uses, serving this SAME
                // router — proves the real WebSocket path works against real
                // REAPER, not just the unix socket `daw::test` normally uses.
                // Port 0 (OS-assigned) so parallel test runs never collide;
                // the actual port is published via ExtState for the test to
                // discover, the same way the health beacon below announces
                // pid/status.
                spawn_lan_test_server(&runtime, handler.clone());
                daw_reaper::build_extension_daw_with(handler).await
            })
            .map_err(|e| eyre::eyre!("{e}"))?;
        info!(
            main_thread_executor_installed = daw::main_thread::is_installed(),
            "session test extension: after build_extension_daw_with"
        );

        // `session::host` names session's whole host surface in one place, so
        // this test host and `fts-extensions` mount the same thing by
        // construction. They used to assemble it independently, and had
        // already drifted: this extension never registered session's
        // `architect::action` surface, so every test ran against a strictly
        // smaller REAPER than production without anything saying so.
        let modules = session::host::modules(daw_reaper::Reaper);
        session::host::register_actions(&daw_reaper::Reaper, daw_reaper::Reaper);
        let module_ctx = runtime.module_context();
        module::init_all(&modules, &module_ctx);
        let action_defs = module::collect_actions(&modules);

        let mut action_handlers = HashMap::new();
        for (id, _, handler, _, _) in &action_defs {
            action_handlers.insert(id.clone(), handler.clone());
        }

        let (action_tx, action_rx) = crossbeam_channel::unbounded();
        for (command_id, display_name, _, show_in_menu, toggleable) in action_defs {
            let cmd_id = daw_reaper::action_registry::register_action_main_thread(
                &command_id,
                &display_name,
                show_in_menu,
                toggleable,
            );

            if cmd_id > 0 {
                info!(command_id = %command_id, cmd_id, "session test action registered");
            } else {
                warn!(command_id = %command_id, "session test action registration returned 0");
            }
        }

        let _ = ExtState::set(
            &daw_reaper::Reaper,
            "FTS_SESSION_EXT",
            "status",
            "ready",
            false,
        );
        let _ = ExtState::set(
            &daw_reaper::Reaper,
            "FTS_SESSION_EXT",
            "pid",
            &std::process::id().to_string(),
            false,
        );

        runtime.spawn(async move {
            let mut events = daw_reaper::action_registry::subscribe_action_broadcasts();
            loop {
                match events.recv().await {
                    Ok(command_name) => {
                        let _ = action_tx.send(command_name);
                    }
                    Err(e) => {
                        warn!("session test extension action stream error: {e}");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            runtime,
            action_rx,
            action_handlers,
        })
    }

    fn timer(&self) {
        self.runtime.process_tasks();
        while let Ok(command_name) = self.action_rx.try_recv() {
            if let Some(handler) = self.action_handlers.get(&command_name) {
                handler();
            }
        }
    }
}

/// Bind a real axum `/vox` WebSocket server on `127.0.0.1:0` (OS-assigned
/// port, so parallel test runs never collide) serving `handler` — the
/// exact `architect::axum_ws::serve_router` path `session-desktop --engine`
/// uses for its LAN control surface. Publishes the bound port to ExtState
/// (`FTS_SESSION_EXT`/`lan_port`) so a `daw::test` can discover it and
/// connect a real `vox_websocket::WsLink` client, proving the WebSocket
/// path itself works against real REAPER — not just that the router works
/// over the unix socket `daw::test` normally uses.
fn spawn_lan_test_server(runtime: &ExtensionRuntime, handler: daw::LayerRouter) {
    runtime.spawn(async move {
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(e) => {
                warn!("session test extension: LAN test server bind failed: {e}");
                return;
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let _ = ExtState::set(
            &daw_reaper::Reaper,
            "FTS_SESSION_EXT",
            "lan_port",
            &port.to_string(),
            false,
        );
        info!(port, "session test extension: LAN test server listening");

        let app = axum::Router::new().route(
            "/vox",
            axum::routing::get(move |ws: axum::extract::ws::WebSocketUpgrade| {
                let handler = handler.clone();
                async move {
                    ws.on_upgrade(move |socket| async move {
                        architect::axum_ws::serve_router(socket, handler).await;
                    })
                }
            }),
        );
        if let Err(e) = axum::serve(listener, app).await {
            warn!("session test extension: LAN test server error: {e}");
        }
    });
}

extern "C" fn timer_callback() {
    APP.with(|cell| {
        if let Some(app) = cell.get() {
            app.get().timer();
        }
    });
}

#[reaper_extension_plugin]
fn plugin_main(context: PluginContext) -> Result<(), Box<dyn Error>> {
    init_tracing();
    info!("session test extension starting");

    let app = TestExtension::new(context)?;
    app.runtime.add_timer(timer_callback)?;

    let stored = APP.with(|cell| cell.set(Fragile::new(app)).is_ok());
    if !stored {
        return Err("session test extension already initialized".into());
    }

    info!("session test extension loaded");
    Ok(())
}

fn init_tracing() {
    let Ok(log_file) = std::fs::File::create("/tmp/session-extension.log") else {
        return;
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}
