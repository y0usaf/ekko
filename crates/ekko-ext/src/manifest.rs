use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionManifest {
    /// Stable identifier, e.g. `"ekko-builtins.sidebar"`. Used by the
    /// `[extensions] disabled` config list.
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
}
