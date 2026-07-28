// Test code intentionally uses unwrap/expect/panic: a failed assertion should abort the test.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
pub mod builtin;
pub mod registry;
pub mod schema;

pub use builtin::{
    BashExecTool, EchoTool, ListSkillsTool, ReadFileTool, ReadSkillTool, RefineSkillTool,
    SaveSkillTool, SpawnSubagentTool, WriteFileTool,
};
pub use registry::{ToolCallContext, ToolHandler, ToolOutput, ToolRegistry};
pub use schema::ToolSchema;
