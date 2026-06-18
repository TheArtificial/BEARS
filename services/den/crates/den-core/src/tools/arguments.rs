use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DenToolChannelContext {
    pub family: Option<String>,
    pub client: Option<String>,
    pub protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetConversationTitleArguments {
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct MemoryCreateWorkSurfaceScaffoldArguments {
    pub work_surface_slug: String,
    pub work_surface_name: String,
    pub overview: String,
    #[serde(default)]
    pub glossary: Option<String>,
    #[serde(default)]
    pub current_understanding: Option<String>,
}
