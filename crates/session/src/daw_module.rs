//! DawModule implementation for session.

use crate::{
    auto_color_actions, keyflow_actions, mode_actions, mode_defs, preroll_actions, session_actions,
    track_manager_actions,
};
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
            .chain(mode_defs::definitions())
            .map(|def| {
                let cmd = def.id.to_command_id();
                let name = def.display_name();
                let action_id = def.id.as_str().to_string();
                let cmd2 = cmd.clone();
                let toggleable = preroll_actions::is_toggle_action(&action_id);
                let daw = self.daw.clone();
                let action = ActionDef::new(cmd, name, move || {
                    tracing::info!("[session] Action: {}", cmd2);
                    if let Some(action) = keyflow_actions::action_for_id(&action_id) {
                        keyflow_actions::dispatch(&daw, action);
                    } else if let Some(action) = auto_color_actions::action_for_id(&action_id) {
                        auto_color_actions::dispatch(action);
                    } else if let Some(action) = track_manager_actions::action_for_id(&action_id) {
                        track_manager_actions::dispatch(action);
                    } else if let Some(action) = preroll_actions::action_for_id(&action_id) {
                        preroll_actions::dispatch(&daw, action);
                    } else if let Some(action) = mode_actions::action_for_id(&action_id) {
                        mode_actions::dispatch(action);
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
        auto_color_actions::init(ctx);
        track_manager_actions::init(ctx);
        preroll_actions::init(&self.daw);
        mode_actions::init(ctx);
        template_module().init(ctx);
    }

    fn subscribe(&self, ctx: &ModuleContext) {
        auto_color_actions::subscribe(ctx);
        template_module().subscribe(ctx);
    }
}

pub fn module_with_daw<D>(daw: D) -> Box<dyn DawModule>
where
    D: SessionDaw,
{
    Box::new(SessionModule { daw })
}
