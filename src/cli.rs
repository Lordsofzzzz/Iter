//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "iter")]
#[command(bin_name = "iter")]
#[command(version, about = "Command-line AI coding agent", long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOptions,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args, Clone)]
pub struct GlobalOptions {
    /// Model id to use for this session.
    #[arg(short, long, env = "MODEL_NAME", global = true)]
    pub model: Option<String>,

    /// Working directory to run from.
    #[arg(short = 'C', long, value_name = "DIR", global = true)]
    pub workdir: Option<PathBuf>,

    /// Print model thinking deltas when the backend emits them.
    #[arg(long, global = true, default_value_t = true)]
    pub show_thinking: bool,

    /// TypeScript agent entry point.
    #[arg(long, value_name = "PATH", default_value = "agent/src/index.ts", global = true)]
    pub agent_entry: PathBuf,

    /// Directory for backend stderr logs.
    #[arg(long, value_name = "DIR", default_value = "agent/logs", global = true)]
    pub log_dir: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Send a prompt, or start interactive mode when no prompt is provided.
    Ask {
        /// Prompt text. Multiple words are joined with spaces.
        #[arg(required = false, trailing_var_arg = true)]
        prompt: Vec<String>,
    },
}
