use nightshade::prelude::freecs;
use nightshade::prelude::serde::{Deserialize, Serialize};

/// Marker component for template-specific entities. Replace, rename, or add
/// more as your game grows; every component earns a `field: Type => CONST`
/// line in the schema below.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
#[serde(crate = "nightshade::prelude::serde")]
pub struct Marker;

freecs::dynamic_schema! {
    pub fn register_template_components {
        marker: Marker => MARKER,
    }
}

/// App-wide state. Systems read and mutate these fields directly; grow the
/// struct, or split it into more resources, as your game does.
#[derive(Default)]
pub struct TemplateResources {
    pub example: ExampleState,
}

/// Example resource. Resources are global per-app state that systems read and
/// mutate. Replace, rename, or add more as your game grows.
#[derive(Default)]
pub struct ExampleState {
    pub frame_count: u64,
    pub elapsed_seconds: f32,
}
