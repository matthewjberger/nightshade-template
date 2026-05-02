use nightshade::prelude::serde::{Deserialize, Serialize};

/// Marker component for template-specific entities. Replace, rename, or add
/// to this list as your game grows.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
#[serde(crate = "nightshade::prelude::serde")]
pub struct Marker;
