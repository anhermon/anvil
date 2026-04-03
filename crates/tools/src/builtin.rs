/// Built-in tools registered by default.
///
/// Currently a stub - real implementations added as needed.
use crate::registry::{ToolHandler, ToolOutput};
use crate::schema::ToolSchema;
use async_trait::async_trait;
use serde_json::Value;

/// Echo tool - useful for testing the tool pipeline.
pub struct EchoTool;

#[async_trait]
impl ToolHandler for EchoTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema::simple("echo", "Echo the input message back", &["message"])
    }

    async fn call(&self, input: Value) -> ToolOutput {
        let msg = input["message"].as_str().unwrap_or("(empty)");
        ToolOutput::ok(msg)
    }
}

/// Spawn a sub-agent to handle a sub-task.
///
/// The actual execution is handled by the [`Agent`] loop in `harness-cli` —
/// it intercepts calls to this tool name and runs a nested Agent instance.
/// This struct exists only to declare the schema so the LLM sees the tool.
pub struct SpawnSubagentTool;

#[async_trait]
impl ToolHandler for SpawnSubagentTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "spawn_subagent".to_string(),
            description: "Spawn a sub-agent to handle a delegated sub-task. \
                 The sub-agent shares the same provider and tool set as the main agent. \
                 Returns the sub-agent's final response text. \
                 Maximum nesting depth is 4 (MAX_SUBAGENT_DEPTH); calls beyond that depth \
                 are rejected with an error."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "description": "The goal or task for the sub-agent to accomplish."
                    },
                    "context": {
                        "type": "string",
                        "description": "Optional background context for the sub-agent."
                    }
                },
                "required": ["goal"]
            }),
        }
    }

    async fn call(&self, _input: Value) -> ToolOutput {
        // The Agent handles spawn_subagent before it reaches the registry.
        ToolOutput::err("spawn_subagent must be handled by the agent loop, not the registry")
    }
}

/// Read the UTF-8 contents of a file at a given path.
///
/// # Security
///
/// To prevent agents from reading arbitrary host files, this tool:
/// - Rejects absolute paths
/// - Rejects any path component that is `..` (parent-directory traversal)
///
/// Only relative paths that stay within the working directory are permitted.
pub struct ReadFileTool;

#[async_trait]
impl ToolHandler for ReadFileTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema::simple("read_file", "Read the UTF-8 contents of a file", &["path"])
    }

    async fn call(&self, input: Value) -> ToolOutput {
        let path = match input["path"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return ToolOutput::err("missing required field: path"),
        };

        let p = std::path::Path::new(&path);
        if p.is_absolute() || path.starts_with('/') {
            return ToolOutput::err("absolute paths are not allowed");
        }
        if p.components().any(|c| c == std::path::Component::ParentDir) {
            return ToolOutput::err("path traversal (..) is not allowed");
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => ToolOutput::ok(contents),
            Err(e) => ToolOutput::err(format!("read_file failed for {path}: {e}")),
        }
    }
}

/// Executes a shell command and returns stdout/stderr.
/// Security: only commands whose first token appears in ALLOWED_COMMANDS are permitted.
pub struct BashExecTool;

const ALLOWED_COMMANDS: &[&str] = &[
    "cargo", "rustfmt", "rustc", "git", "ls", "cat", "echo", "pwd", "env", "which",
];

#[async_trait]
impl ToolHandler for BashExecTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "bash_exec".into(),
            description: "Execute a shell command and return stdout+stderr. \
                Only allowlisted commands are permitted: cargo, rustfmt, rustc, git, ls, cat, echo, pwd, env, which. \
                Timeout: 30 seconds."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, input: Value) -> ToolOutput {
        let command = match input["command"].as_str() {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => return ToolOutput::err("command is required"),
        };

        // Allowlist check: only permit commands whose first token is in ALLOWED_COMMANDS
        let first_token = command.split_whitespace().next().unwrap_or("");
        if !ALLOWED_COMMANDS.contains(&first_token) {
            return ToolOutput::err(format!(
                "Command not allowed: only [{}] are permitted",
                ALLOWED_COMMANDS.join(", ")
            ));
        }

        use std::time::Duration;

        let output = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::task::spawn_blocking(move || {
                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .output()
            }),
        )
        .await;

        match output {
            Ok(Ok(Ok(out))) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let exit_code = out.status.code().unwrap_or(-1);
                let result = if stderr.is_empty() {
                    format!("exit_code: {exit_code}\n{stdout}")
                } else {
                    format!("exit_code: {exit_code}\nstdout:\n{stdout}\nstderr:\n{stderr}")
                };
                if out.status.success() {
                    ToolOutput::ok(result)
                } else {
                    ToolOutput::err(result)
                }
            }
            Ok(Ok(Err(e))) => ToolOutput::err(format!("failed to spawn process: {e}")),
            Ok(Err(e)) => ToolOutput::err(format!("task panic: {e}")),
            Err(_) => ToolOutput::err("command timed out after 30 seconds"),
        }
    }
}

/// Writes content to a file. Rejects absolute paths and traversal.
pub struct WriteFileTool;

#[async_trait]
impl ToolHandler for WriteFileTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_file".into(),
            description: "Write content to a file at the given relative path. \
                Creates parent directories as needed. \
                Rejects absolute paths and \'..\' traversal."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(&self, input: Value) -> ToolOutput {
        let raw = match input["path"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return ToolOutput::err("path is required"),
        };
        let content = input["content"].as_str().unwrap_or("").to_string();

        let p = std::path::Path::new(&raw);
        if p.is_absolute() || raw.starts_with('/') {
            return ToolOutput::err("absolute paths are not allowed");
        }
        if p.components().any(|c| c == std::path::Component::ParentDir) {
            return ToolOutput::err("path traversal (..) is not allowed");
        }

        // Create parent directories if needed
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return ToolOutput::err(format!("failed to create directories: {e}"));
                }
            }
        }

        match std::fs::write(&raw, &content) {
            Ok(()) => ToolOutput::ok(format!("wrote {} bytes to {raw}", content.len())),
            Err(e) => ToolOutput::err(format!("write_file failed for {raw}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn bash_exec_echo() {
        let tool = BashExecTool;
        let out = tool.call(json!({"command": "echo hello"})).await;
        assert!(!out.is_error, "unexpected error: {}", out.content);
        assert!(
            out.content.contains("hello"),
            "expected hello in: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_exec_sudo_not_allowed() {
        let tool = BashExecTool;
        let out = tool.call(json!({"command": "sudo rm -rf /"})).await;
        assert!(out.is_error);
        assert!(
            out.content.contains("not allowed"),
            "expected 'not allowed' in: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_exec_cargo_version_allowed() {
        let tool = BashExecTool;
        let out = tool.call(json!({"command": "cargo --version"})).await;
        assert!(
            !out.is_error,
            "expected cargo --version to succeed: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_exec_nonzero_exit_is_error() {
        let tool = BashExecTool;
        // ls on a non-existent path exits non-zero; ls is in the allowlist
        let out = tool
            .call(json!({"command": "ls /this_path_does_not_exist_xyz_12345"}))
            .await;
        assert!(out.is_error, "expected error for non-zero exit");
    }

    #[tokio::test]
    async fn bash_exec_missing_command_is_error() {
        let tool = BashExecTool;
        let out = tool.call(json!({})).await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn write_file_creates_file() {
        let tool = WriteFileTool;
        let out = tool
            .call(json!({"path": "test_write_output.txt", "content": "hello world"}))
            .await;
        let _ = std::fs::remove_file("test_write_output.txt");
        assert!(!out.is_error, "write failed: {}", out.content);
        assert!(out.content.contains("11 bytes"), "got: {}", out.content);
    }

    #[tokio::test]
    async fn write_file_rejects_traversal() {
        let tool = WriteFileTool;
        let out = tool
            .call(json!({"path": "../../evil.txt", "content": "bad"}))
            .await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn write_file_rejects_absolute() {
        let tool = WriteFileTool;
        let out = tool
            .call(json!({"path": "/tmp/evil.txt", "content": "bad"}))
            .await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn read_file_rejects_traversal() {
        let tool = ReadFileTool;
        let out = tool.call(json!({"path": "../../.env"})).await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn read_file_rejects_absolute() {
        let tool = ReadFileTool;
        let out = tool.call(json!({"path": "/etc/passwd"})).await;
        assert!(out.is_error);
    }
}


// ---------------------------------------------------------------------------
// Skill evolution tools
// ---------------------------------------------------------------------------
fn skills_dir() -> std::path::PathBuf {
    if let Ok(override_dir) = std::env::var("ANVIL_SKILLS_DIR") {
        return std::path::PathBuf::from(override_dir);
    }
    let base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base).join(".anvil").join("skills")
}
fn parse_uses(c: &str) -> u64 {
    for line in c.lines() {
        if let Some(r) = line.trim().strip_prefix("uses:") {
            if let Ok(n) = r.trim().parse::<u64>() { return n; }
        }
    }
    0
}
fn parse_version(c: &str) -> u64 {
    for line in c.lines() {
        if let Some(r) = line.trim().strip_prefix("version:") {
            if let Ok(n) = r.trim().parse::<u64>() { return n; }
        }
    }
    1
}
fn parse_description(c: &str) -> String {
    for line in c.lines() {
        if let Some(r) = line.trim().strip_prefix("description:") {
            return r.trim().to_string();
        }
    }
    String::new()
}
fn update_frontmatter_field(content: &str, key: &str, new_val: u64) -> String {
    let prefix = format!("{key}:");
    content
        .lines()
        .map(|line| {
            if line.trim().starts_with(&prefix) {
                format!("{key}: {new_val}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let mut d = days;
    let mut y = 1970u64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let days_in_year = if leap { 366 } else { 365 };
        if d < days_in_year { break; }
        d -= days_in_year;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in_month = [31u64, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1u64;
    for &dim in &days_in_month {
        if d < dim { break; }
        d -= dim;
        mo += 1;
    }
    (y, mo, d + 1)
}
fn utc_date_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}")
}
fn build_skill_file(name: &str, description: &str, content: &str, version: u64, uses: u64, created: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nversion: {version}\ncreated: {created}\nuses: {uses}\n---\n\n{content}\n"
    )
}

/// List all skills stored in ~/.anvil/skills/
pub struct ListSkillsTool;
#[async_trait]
impl ToolHandler for ListSkillsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_skills".into(),
            description: "List all skills in the skill library. Returns a JSON array of objects with name, description, version, and uses fields.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{},"required":[]}),
        }
    }
    async fn call(&self, _input: Value) -> ToolOutput {
        let dir = skills_dir();
        if !dir.exists() {
            return ToolOutput::ok("[]");
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => return ToolOutput::err(format!("list_skills failed: {e}")),
        };
        let mut skills: Vec<serde_json::Value> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if name.is_empty() { continue; }
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let desc = parse_description(&content);
            let ver = parse_version(&content);
            let uses = parse_uses(&content);
            skills.push(serde_json::json!({"name": name, "description": desc, "version": ver, "uses": uses}));
        }
        skills.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
        match serde_json::to_string(&skills) {
            Ok(s) => ToolOutput::ok(s),
            Err(e) => ToolOutput::err(format!("serialization failed: {e}")),
        }
    }
}

/// Read a skill from ~/.anvil/skills/<name>.md and increment its uses counter.
pub struct ReadSkillTool;
#[async_trait]
impl ToolHandler for ReadSkillTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema::simple("read_skill", "Read a skill by name from the skill library. Increments the uses counter. Returns the skill content.", &["name"])
    }
    async fn call(&self, input: Value) -> ToolOutput {
        let name = match input["name"].as_str() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => return ToolOutput::err("name is required"),
        };
        let path = skills_dir().join(format!("{name}.md"));
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return ToolOutput::err(format!("skill not found: {name}")),
        };
        let new_uses = parse_uses(&content) + 1;
        let updated = update_frontmatter_field(&content, "uses", new_uses);
        let _ = std::fs::write(&path, &updated);
        ToolOutput::ok(updated)
    }
}

/// Save or update a skill in ~/.anvil/skills/<name>.md
pub struct SaveSkillTool;
#[async_trait]
impl ToolHandler for SaveSkillTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "save_skill".into(),
            description: "Save a new skill or update an existing one. New skills start at version 1; updates bump the version. Returns JSON with saved, version, and path.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Skill identifier (kebab-case)"},
                    "description": {"type": "string", "description": "One-line description"},
                    "content": {"type": "string", "description": "Skill body in Markdown"}
                },
                "required": ["name", "description", "content"]
            }),
        }
    }
    async fn call(&self, input: Value) -> ToolOutput {
        let name = match input["name"].as_str() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => return ToolOutput::err("name is required"),
        };
        let description = input["description"].as_str().unwrap_or("").to_string();
        let content = input["content"].as_str().unwrap_or("").to_string();
        let dir = skills_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return ToolOutput::err(format!("failed to create skills dir: {e}"));
        }
        let path = dir.join(format!("{name}.md"));
        let (version, uses, created) = if path.exists() {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            let old_ver = parse_version(&existing);
            let old_uses = parse_uses(&existing);
            let old_created = existing.lines()
                .find(|l| l.trim().starts_with("created:"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_else(utc_date_string);
            (old_ver + 1, old_uses, old_created)
        } else {
            (1, 0, utc_date_string())
        };
        let file_content = build_skill_file(&name, &description, &content, version, uses, &created);
        match std::fs::write(&path, &file_content) {
            Ok(()) => {
                let result = serde_json::json!({"saved": true, "version": version, "path": path.display().to_string()});
                ToolOutput::ok(result.to_string())
            }
            Err(e) => ToolOutput::err(format!("save_skill failed: {e}")),
        }
    }
}

/// Append refinement notes to an existing skill and bump its version.
pub struct RefineSkillTool;
#[async_trait]
impl ToolHandler for RefineSkillTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "refine_skill".into(),
            description: "Append feedback/refinement notes to an existing skill and bump its version. Returns the updated skill content.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Skill name to refine"},
                    "feedback": {"type": "string", "description": "Refinement notes to append"}
                },
                "required": ["name", "feedback"]
            }),
        }
    }
    async fn call(&self, input: Value) -> ToolOutput {
        let name = match input["name"].as_str() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => return ToolOutput::err("name is required"),
        };
        let feedback = match input["feedback"].as_str() {
            Some(f) if !f.is_empty() => f.to_string(),
            _ => return ToolOutput::err("feedback is required"),
        };
        let path = skills_dir().join(format!("{name}.md"));
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return ToolOutput::err(format!("skill not found: {name}")),
        };
        let old_ver = parse_version(&content);
        let new_ver = old_ver + 1;
        let mut updated = update_frontmatter_field(&content, "version", new_ver);
        updated.push_str(&format!("\n## Refinement Notes (v{new_ver})\n\n{feedback}\n"));
        match std::fs::write(&path, &updated) {
            Ok(()) => ToolOutput::ok(updated),
            Err(e) => ToolOutput::err(format!("refine_skill failed: {e}")),
        }
    }
}

#[cfg(test)]
mod skill_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::sync::{Mutex, MutexGuard};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static ENV_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    fn get_env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }
    async fn unique_skills_dir() -> (std::path::PathBuf, MutexGuard<'static, ()>) {
        let guard = get_env_lock().lock().await;
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("anvil_skill_test_{}_{}", id, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("ANVIL_SKILLS_DIR", &dir);
        (dir, guard)
    }
    #[tokio::test]
    async fn save_skill_creates_new_file() {
        let (tmp, _guard) = unique_skills_dir().await;
        let out = SaveSkillTool.call(json!({"name":"test-save","description":"A test skill","content":"# Test"})).await;
        assert!(!out.is_error, "save failed: {}", out.content);
        let v: serde_json::Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(v["saved"], true); assert_eq!(v["version"], 1);
        let c = std::fs::read_to_string(tmp.join("test-save.md")).unwrap();
        assert!(c.contains("version: 1") && c.contains("description: A test skill"));
    }
    #[tokio::test]
    async fn save_skill_bumps_version_on_update() {
        let (tmp, _guard) = unique_skills_dir().await;
        SaveSkillTool.call(json!({"name":"vs","description":"first","content":"original"})).await;
        let out = SaveSkillTool.call(json!({"name":"vs","description":"updated","content":"improved"})).await;
        assert!(!out.is_error);
        let v: serde_json::Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(v["version"], 2);
        let c = std::fs::read_to_string(tmp.join("vs.md")).unwrap();
        assert!(c.contains("version: 2") && c.contains("improved"));
    }
    #[tokio::test]
    async fn save_skill_missing_name_is_error() {
        let (_tmp, _guard) = unique_skills_dir().await;
        let out = SaveSkillTool.call(json!({"description":"x","content":"y"})).await;
        assert!(out.is_error);
    }
    #[tokio::test]
    async fn read_skill_increments_uses() {
        let (tmp, _guard) = unique_skills_dir().await;
        SaveSkillTool.call(json!({"name":"rm","description":"r","content":"content"})).await;
        let out = ReadSkillTool.call(json!({"name":"rm"})).await;
        assert!(!out.is_error);
        let c = std::fs::read_to_string(tmp.join("rm.md")).unwrap();
        assert!(c.contains("uses: 1"), "uses not incremented: {c}");
    }
    #[tokio::test]
    async fn read_skill_missing_returns_error() {
        let (_tmp, _guard) = unique_skills_dir().await;
        let out = ReadSkillTool.call(json!({"name":"no-such-skill"})).await;
        assert!(out.is_error);
    }
    #[tokio::test]
    async fn list_skills_returns_empty_array_when_no_dir() {
        let (_base_tmp, _guard) = unique_skills_dir().await;
        let tmp = std::env::temp_dir().join(format!("anvil_skill_empty_{}_{}", COUNTER.fetch_add(1, Ordering::Relaxed), std::process::id()));
        std::env::set_var("ANVIL_SKILLS_DIR", &tmp);
        let out = ListSkillsTool.call(json!({})).await;
        assert!(!out.is_error); assert_eq!(out.content.trim(), "[]");
    }
    #[tokio::test]
    async fn list_skills_includes_saved_skills() {
        let (_tmp, _guard) = unique_skills_dir().await;
        SaveSkillTool.call(json!({"name":"skill-a","description":"alpha","content":"a"})).await;
        SaveSkillTool.call(json!({"name":"skill-b","description":"beta","content":"b"})).await;
        let out = ListSkillsTool.call(json!({})).await;
        assert!(!out.is_error);
        let arr: Vec<serde_json::Value> = serde_json::from_str(&out.content).unwrap();
        assert_eq!(arr.len(), 2);
        let has_a = arr.iter().any(|e| e["name"] == "skill-a");
        let has_b = arr.iter().any(|e| e["name"] == "skill-b");
        assert!(has_a && has_b);
    }
    #[tokio::test]
    async fn refine_skill_appends_notes() {
        let (_tmp, _guard) = unique_skills_dir().await;
        SaveSkillTool.call(json!({"name":"ref","description":"r","content":"body"})).await;
        let out = RefineSkillTool.call(json!({"name":"ref","feedback":"Add more examples."})).await;
        assert!(!out.is_error);
        assert!(out.content.contains("Refinement Notes") && out.content.contains("Add more examples."));
    }
    #[tokio::test]
    async fn refine_skill_missing_returns_error() {
        let (_tmp, _guard) = unique_skills_dir().await;
        let out = RefineSkillTool.call(json!({"name":"ghost","feedback":"improve"})).await;
        assert!(out.is_error);
    }
    #[tokio::test]
    async fn refine_skill_missing_feedback_is_error() {
        let (_tmp, _guard) = unique_skills_dir().await;
        SaveSkillTool.call(json!({"name":"nofb","description":"x","content":"y"})).await;
        let out = RefineSkillTool.call(json!({"name":"nofb"})).await;
        assert!(out.is_error);
    }
}
