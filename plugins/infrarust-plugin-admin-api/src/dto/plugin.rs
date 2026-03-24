use infrarust_api::services::plugin_registry::PluginInfo;
use serde::Serialize;

#[derive(Serialize)]
pub struct PluginResponse {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub state: String,
    pub dependencies: Vec<PluginDependencyResponse>,
}

impl PluginResponse {
    pub fn from_info(info: PluginInfo) -> Self {
        Self {
            id: info.id,
            name: info.name,
            version: info.version,
            authors: info.authors,
            description: info.description,
            state: info.state,
            dependencies: info
                .dependencies
                .into_iter()
                .map(|d| PluginDependencyResponse {
                    id: d.id,
                    optional: d.optional,
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub struct PluginDependencyResponse {
    pub id: String,
    pub optional: bool,
}
