use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures::future::join_all;
use futures::StreamExt;
use tokio::sync::mpsc;

use rig_core::client::CompletionClient;
use rig_core::completion::message::{
    AssistantContent, Message, ReasoningContent, Text, ToolCall, ToolResult, ToolResultContent,
    UserContent,
};
use rig_core::completion::request::CompletionRequest;
use rig_core::completion::{CompletionModel, GetTokenUsage};
use rig_core::one_or_many::OneOrMany;
use rig_core::providers::openrouter;
use rig_core::streaming::StreamedAssistantContent;
use rig_core::tool::Tool;

use crate::agent_event::{AgentEvent, TuiCommand};
use crate::context::Context;
use crate::tools;

pub struct AgentConfig {
    pub model: String,
    pub api_key: String,
    pub system_prompt: String,
}

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
            TuiCommand::Abort => {}
            TuiCommand::SetModel(model) => {
                current_model = model;
            }
            TuiCommand::SetProvider(provider) => {
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
    let t_build = Instant::now();
    let completion_model = client.completion_model(model.to_string());
    crate::trace!("agent built in {}ms", t_build.elapsed().as_millis());

    let mut success = true;

    // Build initial message history: system + prior history + user prompt
    let mut messages: Vec<Message> = Vec::new();
    messages.push(Message::system(system_prompt.to_string()));
    messages.extend(context.messages.clone());
    messages.push(Message::user(prompt));

    loop {
        if abort.load(Ordering::Acquire) {
            success = false;
            break;
        }

        // Collect tool definitions
        let mut tool_defs = Vec::new();
        let tools_event_tx = event_tx.clone();
        tool_defs.push(tools::ReadFile.definition(prompt.to_string()).await);
        tool_defs.push(tools::WriteFile.definition(prompt.to_string()).await);
        tool_defs.push(tools::Edit.definition(prompt.to_string()).await);
        tool_defs.push(
            tools::RunCommand::new(Some(tools_event_tx))
                .definition(prompt.to_string())
                .await,
        );
        tool_defs.push(tools::ListFiles.definition(prompt.to_string()).await);
        tool_defs.push(tools::SearchFiles.definition(prompt.to_string()).await);

        let t_stream = Instant::now();

        let request = CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::many(messages.clone())
                .expect("chat_history cannot be empty"),
            documents: vec![],
            tools: tool_defs,
            temperature: Some(0.7),
            max_tokens: Some(8192),
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        };

        let mut stream = match completion_model.stream(request).await {
            Ok(s) => s,
            Err(e) => {
                let _ = event_tx
                    .send(AgentEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
                success = false;
                break;
            }
        };
        crate::trace!("stream_chat returned in {}ms", t_stream.elapsed().as_millis());

        // Stream the assistant response
        while let Some(item) = stream.next().await {
            if abort.load(Ordering::Acquire) {
                success = false;
                break;
            }

            match item {
                Ok(StreamedAssistantContent::Text(text)) => {
                    let _ = event_tx.send(AgentEvent::TextDelta(text.text)).await;
                }
                Ok(StreamedAssistantContent::Reasoning(reasoning)) => {
                    let text = reasoning_text(&reasoning.content);
                    let _ = event_tx
                        .send(AgentEvent::ThinkingDelta(text))
                        .await;
                }
                Ok(StreamedAssistantContent::ReasoningDelta { reasoning, .. }) => {
                    let _ = event_tx
                        .send(AgentEvent::ThinkingDelta(reasoning))
                        .await;
                }
                Ok(StreamedAssistantContent::ToolCall { tool_call, .. }) => {
                    let _ = event_tx
                        .send(AgentEvent::ToolCall {
                            name: tool_call.function.name,
                            input: tool_call.function.arguments.to_string(),
                        })
                        .await;
                }
                Ok(StreamedAssistantContent::Final(response)) => {
                    if let Some(usage) = response.token_usage() {
                        let pct =
                            (usage.total_tokens as f32 / context.context_window as f32) * 100.0;
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

        if !success || abort.load(Ordering::Acquire) {
            if !success {
                success = false;
            }
            break;
        }

        // Add assistant message to history using the aggregated choice
        messages.push(Message::Assistant {
            id: None,
            content: stream.choice.clone(),
        });

        // Check if there are tool calls to execute
        let has_tool_calls = stream.choice.iter().any(|c| matches!(c, AssistantContent::ToolCall(_)));

        if !has_tool_calls {
            // No tool calls — done with this turn
            context.messages = messages;
            break;
        }

        // Collect tool calls from the choice
        let tool_calls: Vec<ToolCall> = stream
            .choice
            .iter()
            .filter_map(|c| {
                if let AssistantContent::ToolCall(tc) = c {
                    Some(tc.clone())
                } else {
                    None
                }
            })
            .collect();

        // Execute all tool calls in parallel
        let mut handles = Vec::new();
        for tc in tool_calls {
            let tx = event_tx.clone();

            handles.push(tokio::spawn(async move {
                let start = Instant::now();
                let result = execute_tool(&tc.function.name, &tc.function.arguments, Some(tx)).await;
                let elapsed = start.elapsed().as_millis() as u64;
                (tc, result, elapsed)
            }));
        }

        let results = join_all(handles).await;

        for result in results {
            match result {
                Ok((tool_call, Ok(output), elapsed_ms)) => {
                    let _ = event_tx
                        .send(AgentEvent::ToolResult {
                            name: tool_call.function.name.clone(),
                            output: output.clone(),
                            elapsed_ms,
                        })
                        .await;

                    messages.push(Message::User {
                        content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                            id: tool_call.id.clone(),
                            call_id: tool_call.call_id.clone(),
                            content: OneOrMany::one(ToolResultContent::Text(Text {
                                text: output,
                            })),
                        })),
                    });
                }
                Ok((tool_call, Err(error), elapsed_ms)) => {
                    let _ = event_tx
                        .send(AgentEvent::ToolResult {
                            name: tool_call.function.name.clone(),
                            output: error.clone(),
                            elapsed_ms,
                        })
                        .await;

                    messages.push(Message::User {
                        content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                            id: tool_call.id.clone(),
                            call_id: tool_call.call_id.clone(),
                            content: OneOrMany::one(ToolResultContent::Text(Text {
                                text: error,
                            })),
                        })),
                    });
                }
                Err(e) => {
                    let _ = event_tx
                        .send(AgentEvent::Error {
                            message: e.to_string(),
                        })
                        .await;
                    success = false;
                }
            }
        }

        // Loop back to send assistant another turn with tool results
    }

    success
}

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

async fn execute_tool(
    name: &str,
    args: &serde_json::Value,
    event_tx: Option<mpsc::Sender<AgentEvent>>,
) -> Result<String, String> {
    match name {
        "read_file" => {
            let a: tools::ReadFileArgs =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            tools::ReadFile.call(a).await.map_err(|e| e.to_string())
        }
        "write_file" => {
            let a: tools::WriteFileArgs =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            tools::WriteFile.call(a).await.map_err(|e| e.to_string())
        }
        "edit" => {
            let a: tools::EditArgs =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            tools::Edit.call(a).await.map_err(|e| e.to_string())
        }
        "run_command" => {
            let a: tools::RunCommandArgs =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            tools::RunCommand::new(event_tx)
                .call(a)
                .await
                .map_err(|e| e.to_string())
        }
        "list_files" => {
            let a: tools::ListFilesArgs =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            tools::ListFiles.call(a).await.map_err(|e| e.to_string())
        }
        "search_files" => {
            let a: tools::SearchFilesArgs =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            tools::SearchFiles.call(a).await.map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown tool: {}", name)),
    }
}
