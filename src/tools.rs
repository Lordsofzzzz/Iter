//! Tool implementations: read_file, write_file, edit, run_command, list_files, search_files

use rig_core::completion::ToolDefinition;
use rig_core::tool::Tool;
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::agent_event::AgentEvent;

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
        std::fs::read_to_string(&args.path).map_err(|e| ReadFileError(e.to_string()))
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
        if let Some(parent) = std::path::Path::new(&args.path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| WriteFileError(e.to_string()))?;
        }
        std::fs::write(&args.path, &args.content).map_err(|e| WriteFileError(e.to_string()))?;
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
        let content = std::fs::read_to_string(&args.path)
            .map_err(|e| EditError(format!("read: {e}")))?;
        if !content.contains(&args.old) {
            return Err(EditError("old text not found".into()));
        }
        let result = content.replacen(&args.old, &args.new, 1);
        std::fs::write(&args.path, &result)
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

        while !stdout_done || !stderr_done {
            tokio::select! {
                line = stdout_lines.next_line(), if !stdout_done => {
                    match line.map_err(|e| RunCommandError(e.to_string()))? {
                        Some(l) => {
                            result.push_str(&l);
                            result.push('\n');
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
                            result.push_str(&l);
                            result.push('\n');
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
        let mut entries: Vec<_> = std::fs::read_dir(&path)
            .map_err(|e| ListFilesError(e.to_string()))?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());

        let mut out = String::new();
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                "dir "
            } else {
                "file"
            };
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

        let mut results = String::new();
        let Ok(entries) = glob::glob(&glob_pattern) else {
            return Ok("no matches found".into());
        };
        for entry in entries.flatten() {
            results.push_str(&format!("{}\n", entry.display()));
        }
        if results.is_empty() {
            return Ok("no matches found".into());
        }
        Ok(results)
    }
}
