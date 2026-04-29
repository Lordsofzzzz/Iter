use super::types::{ChatMessage, MsgKind, ModelStatus};
use std::time::Instant;

pub struct App {
    pub messages:   Vec<ChatMessage>,
    pub scroll:     usize,
    pub input:      String,
    pub streaming:  bool,

    pub tok_input:       u32,
    pub tok_output:      u32,
    pub tok_cache_read:  u32,
    pub tok_cache_write: u32,
    pub tok_total:       u32,
    pub tok_limit:       u32,

    pub context_pct: f32,

    pub turns:      u32,
    pub tool_calls: u32,
    pub cost:       f64,
    pub summarized: u32,

    pub model_name:   String,
    pub model_limit:  u32,
    pub model_temp:   f32,
    pub model_status: ModelStatus,

    pub should_quit: bool,

    pub cooldown_deadline: Option<Instant>,
    pub cooldown_started: Option<Instant>,
    pub cooldown_retries_left: u32,
}

impl App {
    pub fn new() -> Self {
        App {
            messages:   Vec::new(),
            scroll:     0,
            input:      String::new(),
            streaming:  false,

            tok_input:       0,
            tok_output:      0,
            tok_cache_read:  0,
            tok_cache_write: 0,
            tok_total:       0,
            tok_limit:       200_000,

            context_pct: 0.0,

            turns:      0,
            tool_calls: 0,
            cost:       0.0,
            summarized: 0,

            model_name:   "—".into(),
            model_limit:  200_000,
            model_temp:   0.3,
            model_status: ModelStatus::Ready,

            should_quit: false,

            cooldown_deadline: None,
            cooldown_started: None,
            cooldown_retries_left: 0,
        }
    }

    pub fn context_pct_computed(&self) -> u16 {
        ((self.context_pct).min(100.0)) as u16
    }

    pub fn token_pct(&self) -> u16 {
        ((self.tok_total as f64 / self.tok_limit as f64) * 100.0).min(100.0) as u16
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(3);
    }

    pub fn scroll_down(&mut self) {
        self.scroll = (self.scroll + 3).min(usize::MAX);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll = usize::MAX;
    }

    pub fn start_streaming(&mut self) {
        self.streaming    = true;
        self.model_status = ModelStatus::Thinking;
    }

    pub fn end_streaming(&mut self) {
        self.streaming    = false;
        self.model_status = ModelStatus::Ready;
    }

    pub fn set_error(&mut self) {
        self.streaming    = false;
        self.model_status = ModelStatus::Error;
    }

    pub fn push_assistant_delta(&mut self, delta: String) {
        if let Some(last) = self.messages.last_mut() {
            if matches!(last.kind, MsgKind::Assistant) {
                last.content.push_str(&delta);
                return;
            }
        }
        self.messages.push(ChatMessage { kind: MsgKind::Assistant, content: delta });
    }

    pub fn push_system(&mut self, text: String) {
        self.messages.push(ChatMessage { kind: MsgKind::System, content: text });
    }

    pub fn upsert_rate_limit(&mut self, wait_ms: u64, retries_left: u32) {
        let now = Instant::now();
        self.cooldown_deadline     = Some(now + std::time::Duration::from_millis(wait_ms));
        self.cooldown_started      = Some(now);
        self.cooldown_retries_left = retries_left;
        let already_present = self.messages.iter().any(|m| m.kind == MsgKind::RateLimit);
        if !already_present {
            self.messages.push(ChatMessage { kind: MsgKind::RateLimit, content: String::new() });
            self.scroll_to_bottom();
        }
    }

    pub fn clear_rate_limit(&mut self) {
        self.messages.retain(|m| m.kind != MsgKind::RateLimit);
        self.cooldown_deadline     = None;
        self.cooldown_started      = None;
        self.cooldown_retries_left = 0;
    }

    pub fn update_tokens(&mut self, input: u32, output: u32, cache_read: u32, cache_write: u32, total: u32, limit: u32) {
        self.tok_input       = input;
        self.tok_output      = output;
        self.tok_cache_read  = cache_read;
        self.tok_cache_write = cache_write;
        self.tok_total       = total;
        self.tok_limit       = limit;
    }

    pub fn update_model_info(&mut self, name: String, limit: u32, temp: f32) {
        self.model_name  = name;
        self.model_limit = limit;
        self.model_temp  = temp;
    }
}