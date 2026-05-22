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
use crate::hooks::{AgentHook, BeforeLlmCtx};
use crate::tools;

pub struct AgentConfig {
    pub model: String,
    pub api_key: String,
    pub system_prompt: String,
    /// Context window size for the starting model. Updated on SetProvider if
    /// the agent is extended to look up the new model's config.
    pub context_window: u32,
}

fn make_client(api_key: &str) -> openrouter::Client {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::HeaderName::from_static("http-referer"),
        http::HeaderValue::from_static("https://iter.dev"),
    );
    headers.insert(
        http::HeaderName::from_static("x-openrouter-title"),
        http::HeaderValue::from_static("iter"),
    );
    headers.insert(
        http::HeaderName::from_static("x-openrouter-categories"),
        http::HeaderValue::from_static("cli-agent"),
    );

    openrouter::Client::builder()
        .api_key(api_key)
        .http_headers(headers)
        .build()
        .expect("failed to create OpenRouter client")
}

pub async fn run_agent_loop(
    config: AgentConfig,
    event_tx: mpsc::Sender<AgentEvent>,
    mut cmd_rx: mpsc::Receiver<TuiCommand>,
    hooks: Vec<Box<dyn AgentHook>>,
) {
    let mut client = make_client(&config.api_key);

    // Use the context window passed in from main (looked up from model config).
    let mut context = Context::new(config.context_window);
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
                        &hooks,
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
            TuiCommand::SetProviderWithKey { provider, key } => {
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
    hooks: &[Box<dyn AgentHook>],
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

    let static_tool_defs = {
        let mut defs = Vec::new();
        defs.push(tools::ReadFile.definition(prompt.to_string()).await);
        defs.push(tools::WriteFile.definition(prompt.to_string()).await);
        defs.push(tools::Edit.definition(prompt.to_string()).await);
        defs.push(
            tools::RunCommand::new(Some(event_tx.clone()))
                .definition(prompt.to_string())
                .await,
        );
        defs.push(tools::ListFiles.definition(prompt.to_string()).await);
        defs.push(tools::SearchFiles.definition(prompt.to_string()).await);
        defs.push(tools::Grep.definition(prompt.to_string()).await);
        defs
    };

    'outer: loop {
        if abort.load(Ordering::Acquire) {
            success = false;
            break;
        }

        // Run context hooks before building the request.
        // Clone static_tool_defs so hooks can mutate their copy each iteration.
        let mut hook_ctx = BeforeLlmCtx { messages, tool_defs: static_tool_defs.clone() };
        for hook in hooks {
            if let Err(e) = hook.on_before_llm(&mut hook_ctx) {
                let _ = event_tx.send(AgentEvent::Error { message: e }).await;
                success = false;
                break;
            }
        }
        if !success {
            break;
        }
        messages = hook_ctx.messages;
        let tool_defs = hook_ctx.tool_defs;

        let t_stream = Instant::now();

        // Retry up to 3 times on rate-limit errors with exponential backoff.
        // Uses a single labeled loop that covers both the initial connection
        // *and* streaming — a 429 on the first chunk restarts the whole thing.
        let mut req_retries = 0usize;
        let stream = 'request: loop {
            let request = CompletionRequest {
                model: None,
                preamble: None,
                chat_history: OneOrMany::many(messages.clone())
                    .expect("chat_history cannot be empty"),
                documents: vec![],
                tools: tool_defs.clone(),
                temperature: Some(0.7),
                max_tokens: Some(8192),
                tool_choice: None,
                additional_params: None,
                output_schema: None,
            };
            let mut stream = match completion_model.stream(request).await {
                Ok(s) => s,
                Err(e) => {
                    let msg = e.to_string();
                    if is_rate_limit(&msg) && req_retries < 3 {
                        req_retries += 1;
                        let delay_ms = 500u64 * (1u64 << req_retries);
                        if !abortable_backoff(req_retries, 3, delay_ms, abort, event_tx).await {
                            success = false;
                            break 'outer;
                        }
                        continue 'request;
                    }
                    let _ = event_tx
                        .send(AgentEvent::Error { message: msg }).await;
                    success = false;
                    break 'outer;
                }
            };

            crate::trace!("stream_chat returned in {}ms", t_stream.elapsed().as_millis());

            // Stream the assistant response
            while let Some(item) = stream.next().await {
                if abort.load(Ordering::Acquire) {
                    success = false;
                    break 'outer;
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
                        let msg = e.to_string();
                        if is_rate_limit(&msg) && req_retries < 3 {
                            req_retries += 1;
                            let delay_ms = 500u64 * (1u64 << req_retries);
                            if !abortable_backoff(req_retries, 3, delay_ms, abort, event_tx).await {
                                success = false;
                                break 'outer;
                            }
                            continue 'request;
                        }
                        let _ = event_tx
                            .send(AgentEvent::Error { message: msg })
                            .await;
                        success = false;
                        break 'outer;
                    }
                    _ => {}
                }
            }

            // Stream finished without breaking — success.
            break 'request stream;
        };

        if !success || abort.load(Ordering::Acquire) {
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

        // Determine execution order: sequential if any tool requires it
        if tool_calls.iter().any(|tc| tool_is_sequential(&tc.function.name)) {
            // Run all tool calls sequentially
            for tc in tool_calls {
                let mut hook_blocked = false;
                for hook in hooks {
                    if let Err(e) = hook.on_before_tool(&tc) {
                        let _ = event_tx.send(AgentEvent::Error { message: e }).await;
                        success = false;
                        hook_blocked = true;
                        break;
                    }
                }
                if hook_blocked {
                    break;
                }

                let start = Instant::now();
                let result = execute_tool(&tc.function.name, &tc.function.arguments, Some(event_tx.clone())).await;
                let elapsed = start.elapsed().as_millis() as u64;

                push_tool_result(tc, result, elapsed, &mut messages, hooks, event_tx).await;
            }
        } else {
            // Execute all tool calls in parallel
            let mut handles = Vec::new();
            for tc in tool_calls {
                let mut hook_blocked = false;
                for hook in hooks {
                    if let Err(e) = hook.on_before_tool(&tc) {
                        let _ = event_tx.send(AgentEvent::Error { message: e }).await;
                        success = false;
                        hook_blocked = true;
                        break;
                    }
                }
                if hook_blocked {
                    continue;
                }

                let name = tc.function.name.clone();
                let args = tc.function.arguments.clone();
                let tx = event_tx.clone();

                handles.push(tokio::spawn(async move {
                    let start = Instant::now();
                    let result = execute_tool(&name, &args, Some(tx)).await;
                    let elapsed = start.elapsed().as_millis() as u64;
                    (tc, result, elapsed)
                }));
            }

            let results = join_all(handles).await;

            // Push all completed results first (order matters for LLM context),
            // then surface any panics. Breaking after a panic avoids sending the
            // LLM an incomplete tool-call/result set on the next turn.
            let mut panicked: Vec<String> = Vec::new();
            for result in results {
                match result {
                    Ok((tool_call, output, elapsed_ms)) => {
                        push_tool_result(tool_call, output, elapsed_ms, &mut messages, hooks, event_tx).await;
                    }
                    Err(e) => {
                        panicked.push(format!("tool panicked: {e}"));
                    }
                }
            }
            for msg in panicked {
                let _ = event_tx.send(AgentEvent::Error { message: msg }).await;
                success = false;
            }
            if !success {
                break;
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
        "grep" => {
            let a: tools::GrepArgs =
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
            tools::Grep.call(a).await.map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown tool: {}", name)),
    }
}

fn tool_is_sequential(name: &str) -> bool {
    matches!(name, "write_file" | "edit")
}

async fn push_tool_result(
    tool_call: ToolCall,
    result: Result<String, String>,
    elapsed_ms: u64,
    messages: &mut Vec<Message>,
    hooks: &[Box<dyn AgentHook>],
    event_tx: &mpsc::Sender<AgentEvent>,
) {
    let output = match result {
        Ok(output) => output,
        Err(error) => error,
    };

    for hook in hooks {
        hook.on_after_tool(&tool_call, &output, elapsed_ms);
    }

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

/// Run an abortable exponential-backoff wait, emitting `Retrying` ticks every
/// 100ms so the TUI can animate a countdown bar.
///
/// Returns `true` if the wait completed normally (caller should retry),
/// `false` if abort fired mid-wait (caller should break out of the turn).
async fn abortable_backoff(
    attempt: usize,
    max_attempts: usize,
    delay_ms: u64,
    abort: &AtomicBool,
    event_tx: &mpsc::Sender<AgentEvent>,
) -> bool {
    let tick_ms = 100u64;
    let t_start = Instant::now();

    loop {
        let elapsed_ms = t_start.elapsed().as_millis() as u64;
        let _ = event_tx
            .send(AgentEvent::Retrying {
                attempt:    attempt as u8,
                total:      max_attempts as u8,
                wait_ms:    delay_ms,
                elapsed_ms,
            })
            .await;

        if elapsed_ms >= delay_ms {
            return true;
        }

        let remaining = delay_ms - elapsed_ms;
        let sleep_for = tick_ms.min(remaining);

        tokio::select! {
            biased;
            _ = tokio::time::sleep(std::time::Duration::from_millis(sleep_for)) => {}
            _ = async {
                while !abort.load(Ordering::Acquire) {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            } => { return false; }
        }

        if abort.load(Ordering::Acquire) {
            return false;
        }
    }
}

fn is_rate_limit(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("429")
        || lower.contains("too many requests")
        || lower.contains("too_many_requests")
        || lower.contains("rate_limit")
        || lower.contains("ratelimiterror")
        || lower.contains("rate limit exceeded")
}
