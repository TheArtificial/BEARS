//! Prompt-memory domain types (runtime prompt blocks, distinct from semantic memory).
//!
//! Shared by the `prompt_memory` tool executors here and the Postgres-backed
//! store + prompt-assembly code in the `den` crate.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMemoryBlockType {
    RoleGuidance,
    WorkSurfaceContext,
    SessionFocus,
    UserInstruction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMemoryBlockScope {
    BearWide,
    RoleLocal,
    WorkSurface,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMemoryBlockState {
    Draft,
    Active,
    Superseded,
    Archived,
}

/// A stored prompt-memory block as projected for listing/compilation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptMemoryBlock {
    pub id: String,
    pub block_type: PromptMemoryBlockType,
    pub scope: PromptMemoryBlockScope,
    pub state: PromptMemoryBlockState,
    pub role: Option<String>,
    pub work_surface: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub body: String,
    pub priority: i32,
}

/// Full write (insert/upsert) of a prompt-memory block.
#[derive(Debug, Clone)]
pub struct PromptMemoryBlockWrite {
    pub block_id: String,
    pub bear_id: Option<Uuid>,
    pub profile_slug: Option<String>,
    pub scope: PromptMemoryBlockScope,
    pub block_type: PromptMemoryBlockType,
    pub state: PromptMemoryBlockState,
    pub work_surface: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub body: String,
    pub priority: i32,
    pub created_by_user_id: Option<i32>,
    pub supersedes_block_id: Option<String>,
    pub metadata: Value,
}

/// In-place patch of an existing prompt-memory block.
#[derive(Debug, Clone)]
pub struct PromptMemoryBlockPatch {
    pub state: PromptMemoryBlockState,
    pub title: String,
    pub body: String,
    pub priority: i32,
    pub supersedes_block_id: Option<String>,
    pub metadata: Value,
}
