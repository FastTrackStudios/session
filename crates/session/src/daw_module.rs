//! DawModule implementation for session.

use daw::module::{ActionDef, DawModule, ModuleContext};
use crate::session_actions;

pub struct SessionModule;

impl DawModule for SessionModule {
    fn name(&self) -> &str { "session" }
    fn display_name(&self) -> &str { "Session Control" }

    fn actions(&self) -> Vec<ActionDef> {
        session_actions::definitions()
            .into_iter()
            .map(|def| {
                let cmd = def.id.to_command_id();
                let name = def.display_name();
                let cmd2 = cmd.clone();
                ActionDef::new(cmd, name, move || {
                    tracing::info!("[session] Action: {}", cmd2);
                    // TODO: dispatch to session handlers
                })
            })
            .collect()
    }
}

pub fn module() -> Box<dyn DawModule> {
    Box::new(SessionModule)
}
