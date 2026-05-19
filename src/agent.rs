use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

use rig_core::agent::MultiTurnStreamItem;
use rig_core::client::CompletionClient;
use rig_core::providers::openrouter;
use rig_core::streaming::{StreamedAssistantContent, StreamedUserContent, StreamingChat};

use crate::agent_event::{AgentEvent, TuiCommand};
use crate::context::Context;
use crate::tools;

pub struct AgentConfig {
    pub model: String,
    pub api_key: String,
    pub system_prompt: String,
}

/// Build an OpenRouter client. Always uses OpenRouter as the transport;
/// the model string encodes the provider (e.g. "anthropic/claude-3-5-sonnet").
fn make_client(api_key: &str) -> openrouter::Client {
    openrouter::Client::new(api_key)
        .expect("failed to create OpenRouter client")
}

pub async fn run_agent_loop(
    config: AgentConfig,
    event_tx: mpsc::Sender<AgentEvent>,
    mut cmd_rx: mpsc::Receiver<TuiCommand>,
) {
    let mut client = make_client(&config.api_key);

    let mut context = Context::new(128_000);
    let mut current_model = config.model.clone();
    let abort = Arc::new(AtomicBool::new(false));

    let _ = event_tx.send(AgentEvent::AgentStart).await;

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            TuiCommand::Prompt(prompt) => {
                abort.store(false, Ordering::Release);
                let _ = event_tx.send(AgentEvent::TurnStart).await;
                let abort_ref = abort.clone();

                let mut deferred_model: Option<String> = None;
                let mut deferred_clear = false;

                let success = {
                    // Pin the future so we can poll it alongside cmd_rx,
                    // allowing TuiCommand::Abort to be received mid-turn.
                    let mut turn = std::pin::pin!(process_prompt(
                        &client,
                        &current_model,
                        &prompt,
                        &mut context,
                        &event_tx,
                        &config.system_prompt,
                        &abort_ref,
                    ));

                    loop {
                        tokio::select! {
                            biased;
                            result = &mut turn => {
                                break result;
                            }
                            Some(mid_cmd) = cmd_rx.recv() => {
                                match mid_cmd {
                                    TuiCommand::Abort => {
                                        abort_ref.store(true, Ordering::Release);
                                    }
                                    TuiCommand::SetModel(m) => deferred_model = Some(m),
                                    TuiCommand::Clear => deferred_clear = true,
                                    _ => {}
                                }
                            }
                        }
                    }
                };

                if let Some(m) = deferred_model { current_model = m; }
                if deferred_clear { context.clear(); }

                let _ = event_tx.send(AgentEvent::TurnEnd).await;
                let _ = event_tx
                    .send(AgentEvent::AgentEnd {
                        success,
                        error: None,
                    })
                    .await;
            }
            TuiCommand::Abort => {
                // No-op between turns — nothing running to abort.
            }
            TuiCommand::SetModel(model) => {
                current_model = model;
            }
            TuiCommand::SetProvider(provider) => {
                // Look up a provider-specific API key, fall back to the original.
                let env_key = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
                let key = std::env::var(&env_key).unwrap_or_else(|_| config.api_key.clone());
                client = make_client(&key);
                let _ = event_tx
                    .send(AgentEvent::ProviderChanged {
                        provider_id: provider.clone(),
                        provider_name: provider,
                    })
                    .await;
            }
            TuiCommand::Clear => {
                context.clear();
            }
        }
    }
}

async fn process_prompt(
    client: &openrouter::Client,
    model: &str,
    prompt: &str,
    context: &mut Context,
    event_tx: &mpsc::Sender<AgentEvent>,
    system_prompt: &str,
    abort: &AtomicBool,
) -> bool {
    use futures::StreamExt;

    let t_build = Instant::now();
    let agent = client
        .agent(model)
        .preamble(system_prompt)
        .max_tokens(8192)
        .temperature(0.7)
        .default_max_turns(20)
        .tool(tools::ReadFile)
        .tool(tools::WriteFile)
        .tool(tools::Edit)
        .tool(tools::RunCommand::new(Some(event_tx.clone())))
        .tool(tools::ListFiles)
        .tool(tools::SearchFiles)
        .build();
    crate::trace!("agent built in {}ms", t_build.elapsed().as_millis());

    let history = &context.messages;
    let t_stream = Instant::now();
    let mut stream = agent.stream_chat(prompt, history).await;
    crate::trace!("stream_chat returned in {}ms", t_stream.elapsed().as_millis());

    let mut pending_tools: HashMap<String, (String, Instant)> = HashMap::new();
    let mut success = true;

    loop {
        // Check abort before waiting for next item.
        if abort.load(Ordering::Acquire) {
            success = false;
            break;
        }

        let Some(item) = stream.next().await else { break };

        // Check again after the await — abort may have been set while we waited.
        if abort.load(Ordering::Acquire) {
            success = false;
            break;
        }

        match item {
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::Reasoning(text),
            )) => {
                let _ = event_tx
                    .send(AgentEvent::ThinkingDelta(reasoning_text(&text.content)))
                    .await;
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ReasoningDelta { reasoning, .. },
            )) => {
                let _ = event_tx.send(AgentEvent::ThinkingDelta(reasoning)).await;
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::Text(text),
            )) => {
                let _ = event_tx.send(AgentEvent::TextDelta(text.text)).await;
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ToolCall {
                    tool_call,
                    internal_call_id,
                },
            )) => {
                pending_tools.insert(
                    internal_call_id,
                    (tool_call.function.name.clone(), Instant::now()),
                );
                let _ = event_tx
                    .send(AgentEvent::ToolCall {
                        name: tool_call.function.name,
                        input: tool_call.function.arguments.to_string(),
                    })
                    .await;
            }
            Ok(MultiTurnStreamItem::StreamUserItem(
                StreamedUserContent::ToolResult {
                    tool_result,
                    internal_call_id,
                },
            )) => {
                let (name, elapsed_ms) = pending_tools
                    .remove(&internal_call_id)
                    .map(|(n, t)| (n, t.elapsed().as_millis() as u64))
                    .unwrap_or_else(|| (String::new(), 0));
                let output = tool_result_text(&tool_result);
                let _ = event_tx
                    .send(AgentEvent::ToolResult {
                        name,
                        output,
                        elapsed_ms,
                    })
                    .await;
            }
            Ok(MultiTurnStreamItem::FinalResponse(fin)) => {
                if let Some(msgs) = fin.history() {
                    for msg in msgs {
                        context.add_message(msg.clone());
                    }
                }
                let usage = fin.usage();
                let pct = (usage.total_tokens as f32 / context.context_window as f32) * 100.0;
                context.compact_if_needed(pct);
                let _ = event_tx
                    .send(AgentEvent::TokenUsage {
                        input: usage.input_tokens as u32,
                        output: usage.output_tokens as u32,
                        total: usage.total_tokens as u32,
                        cache_read: usage.cached_input_tokens as u32,
                        cache_write: usage.cache_creation_input_tokens as u32,
                        context_pct: pct,
                    })
                    .await;
            }
            Err(e) => {
                let _ = event_tx
                    .send(AgentEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
                success = false;
                break;
            }
            _ => {}
        }
    }

    success
}

use rig_core::completion::message::ReasoningContent;

fn reasoning_text(content: &[ReasoningContent]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            ReasoningContent::Text { text, .. } => Some(text.clone()),
            ReasoningContent::Summary(s) => Some(s.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tool_result_text(tr: &rig_core::message::ToolResult) -> String {
    tr.content
        .iter()
        .filter_map(|c| match c {
            rig_core::message::ToolResultContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
