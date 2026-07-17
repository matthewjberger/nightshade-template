use nightshade::prelude::serde::{Deserialize, Serialize};

/// Marker component for template-specific entities. Replace, rename, or add
/// more as your game grows; every component earns a `field: Type => CONST`
/// line in the `register_template_components` schema in `ecs.rs`.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
#[serde(crate = "nightshade::prelude::serde")]
pub struct Marker;
