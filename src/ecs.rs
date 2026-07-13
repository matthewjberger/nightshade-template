mod components;
mod resources;

pub use components::*;
pub use resources::*;

use nightshade::prelude::freecs;

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
