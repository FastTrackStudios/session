//! DawModule implementation for session.

use crate::keyflow::actions as keyflow_actions;
use crate::modes as mode_actions;
use crate::session_actions;
// Generic DAW actions, no longer session's: pre-roll, record control,
// grouping, take ranking, auto-colour.
use daw_actions::{auto_color, groups, preroll, record, take_ranking};
use daw::module::{ActionDef, DawModule, ModuleContext};
use daw::service::transport::service::Transport as TransportService;
use daw::service::{ActionRegistration, Markers, Projects, Regions, TempoMap};

pub trait SessionDaw:
    Projects
    + TransportService
    + Markers
    + Regions
    + TempoMap
    + ActionRegistration
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<T> SessionDaw for T where
    T: Projects
        + TransportService
        + Markers
        + Regions
        + TempoMap
        + ActionRegistration
        + Clone
        + Send
        + Sync
        + 'static
{
}

pub struct SessionModule<D> {
    daw: D,
}

fn template_module() -> Box<dyn DawModule> {
    dynamic_template::daw_module::module()
}

impl<D> DawModule for SessionModule<D>
where
    D: SessionDaw,
{
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
                let toggleable = preroll::is_toggle_action(&action_id);
                let daw = self.daw.clone();
                let action = ActionDef::new(cmd, name, move || {
                    tracing::info!("[session] Action: {}", cmd2);
                    if let Some(action) = auto_color::action_for_id(&action_id) {
                        auto_color::dispatch(action);
                    } else if let Some(action) = preroll::action_for_id(&action_id) {
                        preroll::dispatch(&daw, action);
                    } else if let Some(action) = take_ranking::action_for_id(&action_id) {
                        take_ranking::dispatch(action);
                    } else if let Some(action) = record::action_for_id(&action_id) {
                        record::dispatch(action);
                    } else if let Some(action) = groups::action_for_id(&action_id) {
                        groups::dispatch(action);
                    } else if dynamic_template::daw_module::dispatch_session_command(&cmd2) {
                        tracing::debug!("[session] Dispatched template action for {}", cmd2);
                    } else {
                        tracing::debug!("[session] No DAW handler registered for {}", cmd2);
                    }
                });
                if toggleable {
                    action.toggleable()
                } else {
                    action
                }
            })
            .collect()
    }

    fn init(&self, ctx: &ModuleContext) {
        keyflow_actions::init(ctx);
        auto_color::init(ctx);
        preroll::init(&self.daw);
        mode_actions::init(ctx);
        template_module().init(ctx);
    }

    fn subscribe(&self, ctx: &ModuleContext) {
        auto_color::subscribe(ctx);
        template_module().subscribe(ctx);
    }
}

pub fn module_with_daw<D>(daw: D) -> Box<dyn DawModule>
where
    D: SessionDaw,
{
    Box::new(SessionModule { daw })
}
