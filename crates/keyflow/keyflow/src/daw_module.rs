//! DawModule implementation for keyflow.
//!
//! Keyflow is a pure data library (chart parsing, engraving) — no actions.
//! The module is registered for discoverability and future extension.

use daw_module::{ActionDef, DawModule};

pub struct KeyflowModule;

impl DawModule for KeyflowModule {
    fn name(&self) -> &str {
        "keyflow"
    }
    fn display_name(&self) -> &str {
        "Keyflow"
    }

    fn actions(&self) -> Vec<ActionDef> {
        // Keyflow is a library — no REAPER actions.
        // Chart parsing and engraving are invoked by other modules.
        vec![]
    }
}

pub fn module() -> Box<dyn DawModule> {
    Box::new(KeyflowModule)
}
