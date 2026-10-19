//! The golden-trace harness without the `dev` feature: no recorder (the
//! engine's `golden` is always `None`), the recording calls do nothing.

/// Never constructed without the `dev` feature.
pub enum Golden {}

pub mod record {
    use crate::host::EngineHost;
    use tt_game::world::Event;
    use tt_gameplay::server::ClientId;
    use tt_gameplay::{HudMsg, ServerMsg};

    pub fn on_message(_: &mut EngineHost, _: Option<ClientId>, _: &ServerMsg) {}
    pub fn on_received(_: &mut EngineHost, _: &ServerMsg) {}
    pub fn on_received_list(_: &mut EngineHost, _: &str, _: &[(u32, String)]) {}
    pub fn on_hud(_: &mut EngineHost, _: Option<ClientId>, _: &HudMsg) {}
    pub fn on_world_events(_: &mut EngineHost, _: &[Event]) {}
}
