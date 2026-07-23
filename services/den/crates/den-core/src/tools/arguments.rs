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
pub struct PrepareRustDependenciesArguments {
    pub manifest_path: String,
    pub package: String,
    pub resolution: RustDependencyResolution,
    pub preparation: RustDependencyPreparation,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustDependencyResolution {
    Locked,
    UpdateLockfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustDependencyPreparation {
    Check,
    TestNoRun,
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
