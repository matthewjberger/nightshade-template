mod components;
mod resources;

pub use components::*;
pub use resources::*;

use nightshade::prelude::freecs;

freecs::ecs! {
    TemplateWorld {
        marker: Marker => MARKER,
    }
    Tags {
    }
    Events {
    }
    Resources {
        example: ExampleState,
    }
}
