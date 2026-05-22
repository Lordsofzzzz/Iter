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

/// Estimate token count using a conservative character-based heuristic.
///
/// tiktoken's cl100k is GPT-4 specific and diverges 15-30% for DeepSeek,
/// Gemini, Mistral, etc. A flat chars/3.5 estimate is less precise but
/// uniformly safe across all model families. Rounds up to avoid undercount.
pub fn estimate_tokens(messages: &[Message]) -> usize {
    let mut chars = 0usize;

    for msg in messages {
        chars += 4; // per-message overhead
        match msg {
            Message::System { content } => {
                chars += content.len();
            }
            Message::User { content } => {
                for c in content.iter() {
                    match c {
                        UserContent::Text(t) => chars += t.text.len(),
                        UserContent::ToolResult(tr) => {
                            for rc in tr.content.iter() {
                                match rc {
                                    ToolResultContent::Text(t) => chars += t.text.len(),
                                    _ => chars += 50,
                                }
                            }
                        }
                        _ => chars += 50,
                    }
                }
            }
            Message::Assistant { content, .. } => {
                for c in content.iter() {
                    match c {
                        AssistantContent::Text(t) => chars += t.text.len(),
                        AssistantContent::ToolCall(tc) => {
                            chars += tc.function.name.len();
                            chars += tc.function.arguments.to_string().len();
                            chars += 8;
                        }
                        AssistantContent::Reasoning(r) => {
                            for rc in &r.content {
                                match rc {
                                    ReasoningContent::Text { text, .. } => chars += text.len(),
                                    ReasoningContent::Summary(s) => chars += s.len(),
                                    _ => chars += 20,
                                }
                            }
                        }
                        _ => chars += 50,
                    }
                }
            }
        }
    }

    // 3.5 chars/token is a safe cross-model average; ceiling to avoid undercount.
    (chars * 2).div_ceil(7)
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

        let mut compacted: Vec<Message> = ctx.messages.drain(..self.keep_front).collect();
        let dropped = ctx.messages.len().saturating_sub(self.keep_back);
        if dropped == 0 {
            compacted.append(&mut ctx.messages);
            ctx.messages = compacted;
            return Ok(());
        }

        let back = ctx.messages.split_off(dropped);

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
