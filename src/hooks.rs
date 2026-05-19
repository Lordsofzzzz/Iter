use tiktoken_rs::cl100k_base_singleton;

use rig_core::completion::message::{
    AssistantContent, Message, ReasoningContent, ToolCall, ToolResultContent, UserContent,
};
use rig_core::completion::request::ToolDefinition;

/// Mutable context passed to `on_before_llm`.
pub struct BeforeLlmCtx {
    pub messages: Vec<Message>,
    pub tool_defs: Vec<ToolDefinition>,
}

/// Optional hooks the agent loop calls at key phases.
/// All methods are no-ops by default — override only what you need.
pub trait AgentHook: Send + Sync {
    /// Called before each LLM request. Modify messages/tools in place.
    fn on_before_llm(&self, _ctx: &mut BeforeLlmCtx) -> Result<(), String> {
        Ok(())
    }

    /// Called before each tool execution.
    fn on_before_tool(&self, _call: &ToolCall) -> Result<(), String> {
        Ok(())
    }

    /// Called after each tool execution with the result and elapsed time.
    fn on_after_tool(&self, _call: &ToolCall, _result: &str, _elapsed_ms: u64) {}
}

/// Estimate the number of tokens for a list of messages using tiktoken.
pub fn estimate_tokens(messages: &[Message]) -> usize {
    let bpe = cl100k_base_singleton();
    let bpe = bpe.lock();
    let mut total = 0;

    for msg in messages {
        total += 4;
        match msg {
            Message::System { content } => {
                total += bpe.encode_with_special_tokens(content).len();
            }
            Message::User { content } => {
                for c in content.iter() {
                    match c {
                        UserContent::Text(t) => {
                            total += bpe.encode_with_special_tokens(&t.text).len();
                        }
                        UserContent::ToolResult(tr) => {
                            for rc in tr.content.iter() {
                                match rc {
                                    ToolResultContent::Text(t) => {
                                        total += bpe.encode_with_special_tokens(&t.text).len();
                                    }
                                    _ => total += 50,
                                }
                            }
                        }
                        _ => total += 50,
                    }
                }
            }
            Message::Assistant { content, .. } => {
                for c in content.iter() {
                    match c {
                        AssistantContent::Text(t) => {
                            total += bpe.encode_with_special_tokens(&t.text).len();
                        }
                        AssistantContent::ToolCall(tc) => {
                            total += bpe.encode_with_special_tokens(&tc.function.name).len();
                            total += bpe
                                .encode_with_special_tokens(&tc.function.arguments.to_string())
                                .len();
                            total += 8;
                        }
                        AssistantContent::Reasoning(r) => {
                            for rc in &r.content {
                                match rc {
                                    ReasoningContent::Text { text, .. } => {
                                        total += bpe.encode_with_special_tokens(text).len();
                                    }
                                    ReasoningContent::Summary(s) => {
                                        total += bpe.encode_with_special_tokens(s).len();
                                    }
                                    _ => total += 20,
                                }
                            }
                        }
                        _ => total += 50,
                    }
                }
            }
        }
    }

    total
}

/// Built-in hook that prunes messages when estimated tokens exceed a threshold.
/// Replaces the old `Context::compact_if_needed()`.
pub struct CompactContextHook {
    pub context_window: u32,
    pub usage_pct_threshold: f32,
    pub keep_front: usize,
    pub keep_back: usize,
}

impl CompactContextHook {
    pub fn new(context_window: u32) -> Self {
        Self {
            context_window,
            usage_pct_threshold: 75.0,
            keep_front: 1,
            keep_back: 4,
        }
    }
}

impl AgentHook for CompactContextHook {
    fn on_before_llm(&self, ctx: &mut BeforeLlmCtx) -> Result<(), String> {
        let threshold = (self.context_window as f32 * self.usage_pct_threshold / 100.0) as usize;
        if estimate_tokens(&ctx.messages) < threshold {
            return Ok(());
        }

        let dropped = ctx.messages.len().saturating_sub(self.keep_front + self.keep_back);
        if dropped == 0 {
            return Ok(());
        }

        let mut compacted: Vec<Message> = ctx.messages.drain(..self.keep_front).collect();
        let back = ctx.messages.split_off(ctx.messages.len().saturating_sub(self.keep_back));

        compacted.push(Message::user(format!(
            "[{} earlier messages omitted for context length]",
            dropped
        )));
        compacted.push(Message::assistant(
            "Understood. Continuing from the recent context.",
        ));
        compacted.extend(back);

        ctx.messages = compacted;
        Ok(())
    }
}
