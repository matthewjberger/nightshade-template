mod components;
mod resources;

pub use components::*;
pub use resources::*;

use nightshade::prelude::freecs;

/// Index of this app's member world in `world.ecs.worlds`, after the
/// engine's core (0) and retained-UI (1) worlds. Game components live
/// here on the same entities that carry the engine's render components.
pub const GAME: usize = 2;

freecs::dynamic_schema! {
    pub fn register_template_components {
        marker: Marker => MARKER,
    }
}

/// App-wide state. Systems read and mutate these fields directly; grow
/// the struct as your game does.
#[derive(Default)]
pub struct TemplateResources {
    pub example: ExampleState,
}
