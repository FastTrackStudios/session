//! Test-only REAPER extension for the `session` library crate.
//!
//! This is not a production sidecar. It is a small in-process REAPER host used
//! by this repo's integration tests to load `session::daw_module::module()`
//! without loading the full `fts-extensions` plugin.

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
        let _daw = runtime.build_daw()?;

        let modules = vec![session::daw_module::module_with_daw(daw_reaper::Reaper)];
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
