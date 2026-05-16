//! DawModule implementation for session.

use crate::{keyflow_actions, session_actions};
use daw::module::{ActionDef, DawModule, ModuleContext};

pub struct SessionModule;

impl DawModule for SessionModule {
    fn name(&self) -> &str {
        "session"
    }
    fn display_name(&self) -> &str {
        "Session Control"
    }

    fn actions(&self) -> Vec<ActionDef> {
        session_actions::definitions()
            .into_iter()
            .map(|def| {
                let cmd = def.id.to_command_id();
                let name = def.display_name();
                let action_id = def.id.as_str().to_string();
                let cmd2 = cmd.clone();
                ActionDef::new(cmd, name, move || {
                    tracing::info!("[session] Action: {}", cmd2);
                    if let Some(action) = keyflow_actions::action_for_id(&action_id) {
                        keyflow_actions::dispatch(action);
                    } else {
                        tracing::debug!("[session] No DAW handler registered for {}", cmd2);
                    }
                })
            })
            .collect()
    }

    fn init(&self, ctx: &ModuleContext) {
        keyflow_actions::init(ctx);
    }
}

pub fn module() -> Box<dyn DawModule> {
    Box::new(SessionModule)
}
