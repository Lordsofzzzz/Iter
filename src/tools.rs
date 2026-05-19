//! Tool implementations: read_file, write_file, edit, run_command, list_files, search_files

use std::time::Duration;

use rig_core::completion::ToolDefinition;
use rig_core::tool::Tool;
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::agent_event::AgentEvent;

/// Maximum bytes buffered from a single run_command invocation.
/// Commands producing more output are truncated with a notice.
const RUN_COMMAND_MAX_OUTPUT: usize = 1024 * 1024; // 1 MB

/// Wall-clock timeout for run_command. Commands that do not exit within
/// this duration are killed and return a timeout error to the LLM.
const RUN_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
}

#[derive(Debug, thiserror::Error)]
#[error("read_file error: {0}")]
pub struct ReadFileError(pub String);

pub struct ReadFile;

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";

    type Error = ReadFileError;
    type Args = ReadFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Read the contents of a file at the given path".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // FIX: use tokio::fs to avoid blocking the single-threaded runtime.
        tokio::fs::read_to_string(&args.path)
            .await
            .map_err(|e| ReadFileError(e.to_string()))
    }
}

#[derive(Deserialize)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
}

#[derive(Debug, thiserror::Error)]
#[error("write_file error: {0}")]
pub struct WriteFileError(pub String);

pub struct WriteFile;

impl Tool for WriteFile {
    const NAME: &'static str = "write_file";

    type Error = WriteFileError;
    type Args = WriteFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Write content to a file at the given path. Creates parent directories if needed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file"
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

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // FIX: use tokio::fs throughout.
        if let Some(parent) = std::path::Path::new(&args.path).parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| WriteFileError(e.to_string()))?;
        }
        tokio::fs::write(&args.path, &args.content)
            .await
            .map_err(|e| WriteFileError(e.to_string()))?;
        Ok(format!("wrote {} bytes to {}", args.content.len(), args.path))
    }
}

#[derive(Deserialize)]
pub struct EditArgs {
    pub path: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, thiserror::Error)]
#[error("edit error: {0}")]
pub struct EditError(pub String);

pub struct Edit;

impl Tool for Edit {
    const NAME: &'static str = "edit";

    type Error = EditError;
    type Args = EditArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Replace first occurrence of `old` text with `new` text in a file".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "old": { "type": "string", "description": "Text to find (first occurrence)" },
                    "new": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old", "new"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // FIX: use tokio::fs throughout.
        let content = tokio::fs::read_to_string(&args.path)
            .await
            .map_err(|e| EditError(format!("read: {e}")))?;
        if !content.contains(&args.old) {
            return Err(EditError("old text not found".into()));
        }
        let result = content.replacen(&args.old, &args.new, 1);
        tokio::fs::write(&args.path, &result)
            .await
            .map_err(|e| EditError(format!("write: {e}")))?;
        Ok(format!("edited {}", args.path))
    }
}

#[derive(Deserialize)]
pub struct RunCommandArgs {
    pub command: String,
}

#[derive(Debug, thiserror::Error)]
#[error("run_command error: {0}")]
pub struct RunCommandError(pub String);

pub struct RunCommand {
    event_tx: Option<mpsc::Sender<AgentEvent>>,
}

impl RunCommand {
    pub fn new(event_tx: Option<mpsc::Sender<AgentEvent>>) -> Self {
        Self { event_tx }
    }
}

impl Tool for RunCommand {
    const NAME: &'static str = "run_command";

    type Error = RunCommandError;
    type Args = RunCommandArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Run a shell command. Use with caution.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // FIX: wrap the entire command execution in a timeout so that blocking
        // commands (sleep, cat /dev/urandom, etc.) cannot hang the agent forever.
        let result = tokio::time::timeout(RUN_COMMAND_TIMEOUT, self.run_inner(args)).await;
        match result {
            Ok(inner) => inner,
            Err(_elapsed) => Err(RunCommandError(format!(
                "command timed out after {}s",
                RUN_COMMAND_TIMEOUT.as_secs()
            ))),
        }
    }
}

impl RunCommand {
    async fn run_inner(&self, args: RunCommandArgs) -> Result<String, RunCommandError> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&args.command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| RunCommandError(e.to_string()))?;

        let stdout = child.stdout.take()
            .ok_or_else(|| RunCommandError("no stdout".into()))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| RunCommandError("no stderr".into()))?;

        let mut stdout_lines = tokio::io::BufReader::new(stdout).lines();
        let mut stderr_lines = tokio::io::BufReader::new(stderr).lines();

        let mut result = String::new();
        let tx = self.event_tx.clone();

        // Interleave stdout and stderr as they arrive to preserve ordering.
        let mut stdout_done = false;
        let mut stderr_done = false;
        let mut truncated = false;

        while !stdout_done || !stderr_done {
            tokio::select! {
                line = stdout_lines.next_line(), if !stdout_done => {
                    match line.map_err(|e| RunCommandError(e.to_string()))? {
                        Some(l) => {
                            // FIX: cap output to avoid OOM on runaway commands.
                            if result.len() >= RUN_COMMAND_MAX_OUTPUT {
                                if !truncated {
                                    result.push_str("\n[output truncated — limit reached]");
                                    truncated = true;
                                }
                                // Keep draining so the child process doesn't block
                                // on a full pipe buffer, but don't store more data.
                            } else {
                                result.push_str(&l);
                                result.push('\n');
                            }
                            if let Some(ref tx) = tx {
                                let _ = tx.send(AgentEvent::ToolOutput { delta: l }).await;
                            }
                        }
                        None => stdout_done = true,
                    }
                }
                line = stderr_lines.next_line(), if !stderr_done => {
                    match line.map_err(|e| RunCommandError(e.to_string()))? {
                        Some(l) => {
                            if result.len() < RUN_COMMAND_MAX_OUTPUT {
                                result.push_str(&l);
                                result.push('\n');
                            }
                            if let Some(ref tx) = tx {
                                let _ = tx.send(AgentEvent::ToolOutput { delta: l }).await;
                            }
                        }
                        None => stderr_done = true,
                    }
                }
            }
        }

        let status = child.wait().await
            .map_err(|e| RunCommandError(e.to_string()))?;
        if !status.success() {
            result.push_str(&format!("\nexit code: {}", status));
        }
        Ok(result)
    }
}

#[derive(Deserialize)]
pub struct ListFilesArgs {
    pub path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("list_files error: {0}")]
pub struct ListFilesError(pub String);

pub struct ListFiles;

impl Tool for ListFiles {
    const NAME: &'static str = "list_files";

    type Error = ListFilesError;
    type Args = ListFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "List files and directories at the given path (defaults to current directory)".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (defaults to current directory)"
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = args.path.unwrap_or_else(|| ".".to_string());

        // FIX: use tokio::fs::read_dir to avoid blocking the runtime.
        let mut read_dir = tokio::fs::read_dir(&path)
            .await
            .map_err(|e| ListFilesError(e.to_string()))?;

        let mut entries: Vec<(String, bool)> = Vec::new();
        while let Some(entry) = read_dir.next_entry().await.map_err(|e| ListFilesError(e.to_string()))? {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().await
                .map(|t| t.is_dir())
                .unwrap_or(false);
            entries.push((name, is_dir));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut out = String::new();
        for (name, is_dir) in entries {
            let kind = if is_dir { "dir " } else { "file" };
            out.push_str(&format!(" {kind}  {name}\n"));
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
pub struct SearchFilesArgs {
    pub pattern: String,
    pub path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("search_files error: {0}")]
pub struct SearchFilesError(pub String);

pub struct SearchFiles;

impl Tool for SearchFiles {
    const NAME: &'static str = "search_files";

    type Error = SearchFilesError;
    type Args = SearchFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Recursively search for files matching a pattern in a directory".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Filename pattern to search for (supports globs like *.rs)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (defaults to current directory)"
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = args.path.unwrap_or_else(|| ".".to_string());
        let pattern = args.pattern;
        let glob_pattern = format!("{}/**/{}", path.trim_end_matches('/'), pattern);

        // glob::glob is synchronous but typically fast (pure filesystem metadata).
        // Offload to a blocking thread to keep the async runtime responsive.
        let results = tokio::task::spawn_blocking(move || {
            let mut out = String::new();
            let Ok(entries) = glob::glob(&glob_pattern) else {
                return out;
            };
            for entry in entries.flatten() {
                out.push_str(&format!("{}\n", entry.display()));
            }
            out
        })
        .await
        .map_err(|e| SearchFilesError(e.to_string()))?;

        if results.is_empty() {
            return Ok("no matches found".into());
        }
        Ok(results)
    }
}

#[derive(Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    pub path: Option<String>,
    pub glob: Option<String>,
    pub case_sensitive: Option<bool>,
    pub fixed_strings: Option<bool>,
    pub context: Option<usize>,
    pub before_context: Option<usize>,
    pub after_context: Option<usize>,
    pub max_matches: Option<usize>,
    pub output_mode: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("grep error: {0}")]
pub struct GrepError(pub String);

pub struct Grep;

impl Tool for Grep {
    const NAME: &'static str = "grep";

    type Error = GrepError;
    type Args = GrepArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Search file contents with regex. Respects .gitignore. Requires ripgrep (rg) on PATH.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Search pattern (regex or literal string)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search (defaults to current directory)"
                    },
                    "glob": {
                        "type": "string",
                        "description": "File glob filter, e.g. \"*.rs\" or \"**/*.ts\""
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "Case-sensitive search (default: true)"
                    },
                    "fixed_strings": {
                        "type": "boolean",
                        "description": "Treat pattern as literal string instead of regex"
                    },
                    "context": {
                        "type": "number",
                        "description": "Lines of context before and after each match"
                    },
                    "before_context": {
                        "type": "number",
                        "description": "Lines of context before each match"
                    },
                    "after_context": {
                        "type": "number",
                        "description": "Lines of context after each match"
                    },
                    "max_matches": {
                        "type": "number",
                        "description": "Maximum matches per file (default: 100)"
                    },
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files", "count"],
                        "description": "Output format: content (default), files (just paths), count (counts per file)"
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let result = tokio::time::timeout(RUN_COMMAND_TIMEOUT, self.run_inner(args)).await;
        match result {
            Ok(inner) => inner,
            Err(_elapsed) => Err(GrepError(format!(
                "grep timed out after {}s",
                RUN_COMMAND_TIMEOUT.as_secs()
            ))),
        }
    }
}

impl Grep {
    async fn run_inner(&self, args: GrepArgs) -> Result<String, GrepError> {
        let mut cmd = tokio::process::Command::new("rg");
        cmd.arg("--json");
        cmd.arg("--no-heading");

        if let Some(ref g) = args.glob {
            cmd.arg("--glob");
            cmd.arg(g);
        }
        if !args.case_sensitive.unwrap_or(true) {
            cmd.arg("-i");
        }
        if args.fixed_strings.unwrap_or(false) {
            cmd.arg("-F");
        }
        if let Some(c) = args.context {
            cmd.arg("-C");
            cmd.arg(c.to_string());
        }
        if let Some(b) = args.before_context {
            cmd.arg("-B");
            cmd.arg(b.to_string());
        }
        if let Some(a) = args.after_context {
            cmd.arg("-A");
            cmd.arg(a.to_string());
        }
        if let Some(m) = args.max_matches {
            cmd.arg("-m");
            cmd.arg(m.to_string());
        } else {
            cmd.arg("-m");
            cmd.arg("100");
        }

        let output_mode = args.output_mode.as_deref().unwrap_or("content");

        cmd.arg("--");
        cmd.arg(&args.pattern);
        cmd.arg(args.path.as_deref().unwrap_or("."));

        let output = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    GrepError(
                        "ripgrep (rg) not found. Install it: cargo install ripgrep"
                            .into(),
                    )
                } else {
                    GrepError(format!("failed to run rg: {e}"))
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let trimmed = stderr.trim();
            if !trimmed.is_empty() {
                return Err(GrepError(trimmed.to_string()));
            }
        }

        let raw = String::from_utf8(output.stdout)
            .map_err(|e| GrepError(format!("invalid UTF-8: {e}")))?;

        if raw.trim().is_empty() {
            return Ok("no matches found".into());
        }

        match output_mode {
            "files" => Self::format_files(&raw),
            "count" => Self::format_count(&raw),
            _ => Self::format_content(&raw),
        }
    }

    fn format_content(rg_json: &str) -> Result<String, GrepError> {
        let mut out = String::new();
        let mut truncated = false;

        for line in rg_json.lines() {
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| GrepError(format!("parse: {e}")))?;

            let type_str = v["type"].as_str().unwrap_or("");
            let data = &v["data"];
            let path = data["path"]["text"].as_str().unwrap_or("?");
            let line_num = data["line_number"].as_u64().unwrap_or(0);
            let text = data["lines"]["text"]
                .as_str()
                .unwrap_or("")
                .strip_suffix('\n')
                .unwrap_or("");
            let formatted = match type_str {
                "match" => format!("{path}:{line_num}:{text}\n"),
                "context" => format!("{path}-{line_num}-{text}\n"),
                _ => continue,
            };
            if out.len() + formatted.len() > RUN_COMMAND_MAX_OUTPUT {
                truncated = true;
                break;
            }
            out.push_str(&formatted);
        }

        if truncated {
            out.push_str("\n[output truncated — limit reached]");
        }

        if out.is_empty() {
            return Ok("no matches found".into());
        }
        while out.ends_with('\n') {
            out.pop();
        }
        Ok(out)
    }

    fn format_files(rg_json: &str) -> Result<String, GrepError> {
        let mut files: Vec<String> = Vec::new();

        for line in rg_json.lines() {
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| GrepError(format!("parse: {e}")))?;

            if v["type"].as_str() == Some("match") {
                if let Some(file) = v["data"]["path"]["text"].as_str() {
                    let s = file.to_string();
                    if !files.contains(&s) {
                        files.push(s);
                    }
                }
            }
        }

        if files.is_empty() {
            Ok("no matches found".into())
        } else {
            Ok(files.join("\n"))
        }
    }

    fn format_count(rg_json: &str) -> Result<String, GrepError> {
        let mut entries: Vec<(String, u64)> = Vec::new();
        let mut current_file: Option<String> = None;
        let mut current_count: u64 = 0;

        for line in rg_json.lines() {
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| GrepError(format!("parse: {e}")))?;

            let type_str = v["type"].as_str().unwrap_or("");
            match type_str {
                "begin" => {
                    if let Some(ref f) = current_file {
                        entries.push((f.clone(), current_count));
                    }
                    current_file = v["data"]["path"]["text"].as_str().map(|s| s.to_string());
                    current_count = 0;
                }
                "match" => {
                    current_count += 1;
                }
                _ => {}
            }
        }
        if let Some(ref f) = current_file {
            entries.push((f.clone(), current_count));
        }

        if entries.is_empty() {
            return Ok("no matches found".into());
        }

        let mut out = String::new();
        for (file, count) in entries {
            let noun = if count == 1 { "match" } else { "matches" };
            out.push_str(&format!("{file}: {count} {noun}\n"));
        }
        while out.ends_with('\n') {
            out.pop();
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rg_available() -> bool {
        std::process::Command::new("rg")
            .arg("--version")
            .output()
            .is_ok()
    }

    #[tokio::test]
    async fn test_grep_content_mode() {
        if !test_rg_available() {
            return;
        }
        let tool = Grep;
        let result = tool
            .call(GrepArgs {
                pattern: "impl Tool".into(),
                path: Some("src/tools.rs".into()),
                glob: None,
                case_sensitive: None,
                fixed_strings: None,
                context: None,
                before_context: None,
                after_context: None,
                max_matches: Some(3),
                output_mode: Some("content".into()),
            })
            .await
            .unwrap();
        assert!(result.contains("src/tools.rs:34:impl Tool for ReadFile {"));
        assert!(result.contains("src/tools.rs:78:impl Tool for WriteFile {"));
        assert!(!result.contains("src/tools.rs-"));
    }

    #[tokio::test]
    async fn test_grep_files_mode() {
        if !test_rg_available() {
            return;
        }
        let tool = Grep;
        let result = tool
            .call(GrepArgs {
                pattern: "impl Tool".into(),
                path: Some("src/tools.rs".into()),
                glob: None,
                case_sensitive: None,
                fixed_strings: None,
                context: None,
                before_context: None,
                after_context: None,
                max_matches: Some(3),
                output_mode: Some("files".into()),
            })
            .await
            .unwrap();
        assert_eq!(result, "src/tools.rs");
    }

    #[tokio::test]
    async fn test_grep_count_mode() {
        if !test_rg_available() {
            return;
        }
        let tool = Grep;
        let result = tool
            .call(GrepArgs {
                pattern: "impl Tool".into(),
                path: Some("src/tools.rs".into()),
                glob: None,
                case_sensitive: None,
                fixed_strings: None,
                context: None,
                before_context: None,
                after_context: None,
                max_matches: None,
                output_mode: Some("count".into()),
            })
            .await
            .unwrap();
        assert!(result.contains("src/tools.rs: 12 matches"));
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        if !test_rg_available() {
            return;
        }
        let tool = Grep;
        let result = tool
            .call(GrepArgs {
                pattern: "XYZZY_NONEXISTENT_99999_XYZZY".into(),
                path: Some("Cargo.toml".into()),
                glob: None,
                case_sensitive: None,
                fixed_strings: None,
                context: None,
                before_context: None,
                after_context: None,
                max_matches: None,
                output_mode: None,
            })
            .await
            .unwrap();
        assert_eq!(result, "no matches found");
    }

    #[tokio::test]
    async fn test_grep_context_lines() {
        if !test_rg_available() {
            return;
        }
        let tool = Grep;
        let result = tool
            .call(GrepArgs {
                pattern: "RUN_COMMAND_TIMEOUT".into(),
                path: Some("src/tools.rs".into()),
                glob: None,
                case_sensitive: None,
                fixed_strings: None,
                context: Some(1),
                before_context: None,
                after_context: None,
                max_matches: Some(1),
                output_mode: Some("content".into()),
            })
            .await
            .unwrap();
        assert!(result.contains("RUN_COMMAND_TIMEOUT"));
        // Should have context line (prefix with -)
        assert!(result.contains('-'));
    }

    #[test]
    fn test_parse_content_matches() {
        let json = r#"{"type":"begin","data":{"path":{"text":"src/main.rs"}}}
{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"fn main() {\n"},"line_number":1}}
{"type":"context","data":{"path":{"text":"src/main.rs"},"lines":{"text":"// comment\n"},"line_number":2}}
{"type":"end","data":{"path":{"text":"src/main.rs"}}}
{"type":"summary","data":{}}"#;
        let out = Grep::format_content(json).unwrap();
        assert!(out.contains("src/main.rs:1:fn main() {"));
        assert!(out.contains("src/main.rs-2-// comment"));
    }

    #[test]
    fn test_parse_files_mode() {
        let json = r#"{"type":"begin","data":{"path":{"text":"a.rs"}}}
{"type":"match","data":{"path":{"text":"a.rs"},"lines":{"text":"x\n"},"line_number":1}}
{"type":"end","data":{"path":{"text":"a.rs"}}}
{"type":"begin","data":{"path":{"text":"b.rs"}}}
{"type":"match","data":{"path":{"text":"b.rs"},"lines":{"text":"x\n"},"line_number":1}}
{"type":"end","data":{"path":{"text":"b.rs"}}}
{"type":"summary","data":{}}"#;
        let out = Grep::format_files(json).unwrap();
        assert_eq!(out, "a.rs\nb.rs");
    }

    #[test]
    fn test_parse_count_mode() {
        let json = r#"{"type":"begin","data":{"path":{"text":"a.rs"}}}
{"type":"match","data":{"path":{"text":"a.rs"},"lines":{"text":"x\n"},"line_number":1}}
{"type":"match","data":{"path":{"text":"a.rs"},"lines":{"text":"y\n"},"line_number":2}}
{"type":"end","data":{"path":{"text":"a.rs"}}}
{"type":"begin","data":{"path":{"text":"b.rs"}}}
{"type":"match","data":{"path":{"text":"b.rs"},"lines":{"text":"z\n"},"line_number":1}}
{"type":"end","data":{"path":{"text":"b.rs"}}}
{"type":"summary","data":{}}"#;
        let out = Grep::format_count(json).unwrap();
        assert!(out.contains("a.rs: 2 matches"));
        assert!(out.contains("b.rs: 1 match"));
    }

    #[test]
    fn test_empty_output() {
        let out = Grep::format_content("").unwrap();
        assert_eq!(out, "no matches found");

        let out = Grep::format_files("").unwrap();
        assert_eq!(out, "no matches found");

        let out = Grep::format_count("").unwrap();
        assert_eq!(out, "no matches found");
    }
}
