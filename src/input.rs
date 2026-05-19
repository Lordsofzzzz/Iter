//! Inline bordered input box.
//!
//! Draws a 3-row box wherever the cursor currently is and redraws in-place.
//! No alternate screen, no scroll regions, no pinning to bottom.
//! Output flows naturally below the box after submit.
//!
//! Slash-command mode: when the buffer starts with `/`, an overlay is rendered
//! above the input box listing matching commands. Arrow keys / Tab navigate;
//! Enter or Tab completes; Escape cancels without clearing the buffer.

use std::io::{self, Write};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    style::{self, Color, Stylize},
    terminal, QueueableCommand,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::state::State;

const PROMPT: &str = "❯ ";
const BOX_ROWS: u16 = 2;

// ── Slash commands ────────────────────────────────────────────────────────────

/// (command, description)
const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/model",    "switch the active model"),
    ("/provider", "switch the active provider"),
    ("/clear",    "clear conversation history"),
    ("/abort",    "abort the current request"),
    ("/help",     "show available commands"),
];

/// Maximum number of items shown in the overlay at once.
const MAX_VISIBLE: usize = 5;

/// Free models available via OpenRouter.
/// (model_id, display_name, context_window)
const FREE_MODELS: &[(&str, &str, &str)] = &[
    ("deepseek/deepseek-v4-flash:free",               "DeepSeek V4 Flash",       "64k"),
    ("deepseek/deepseek-r1:free",                     "DeepSeek R1",             "64k"),
    ("deepseek/deepseek-r1-0528:free",                "DeepSeek R1 0528",        "64k"),
    ("google/gemini-2.5-flash-preview:free",          "Gemini 2.5 Flash",        "1M"),
    ("google/gemini-2.0-flash-thinking-exp:free",     "Gemini 2.0 Flash Think",  "1M"),
    ("meta-llama/llama-4-scout:free",                 "Llama 4 Scout",           "128k"),
    ("meta-llama/llama-4-maverick:free",               "Llama 4 Maverick",        "128k"),
    ("meta-llama/llama-3.3-70b-instruct:free",        "Llama 3.3 70B",           "128k"),
    ("mistralai/mistral-7b-instruct:free",            "Mistral 7B",              "32k"),
    ("qwen/qwen3-235b-a22b:free",                     "Qwen3 235B",              "128k"),
    ("qwen/qwen3-30b-a3b:free",                       "Qwen3 30B",               "128k"),
    ("microsoft/phi-4-reasoning-plus:free",           "Phi-4 Reasoning+",        "32k"),
];

/// Available providers with their display names and env key names.
const PROVIDERS: &[(&str, &str, &str)] = &[
    ("openrouter", "OpenRouter",  "OPENROUTER_API_KEY"),
    ("anthropic",  "Anthropic",   "ANTHROPIC_API_KEY"),
    ("openai",     "OpenAI",      "OPENAI_API_KEY"),
    ("google",     "Google",      "GOOGLE_API_KEY"),
    ("deepseek",   "DeepSeek",    "DEEPSEEK_API_KEY"),
    ("groq",       "Groq",        "GROQ_API_KEY"),
];

// ── Public types ──────────────────────────────────────────────────────────────

pub enum InputResult {
    Submit(String),
    Quit,
    Abort,
}

pub struct InputBox {
    buf:    String,
    cursor: usize,
    /// Slash-command picker state. `None` when not in slash mode.
    slash:  Option<SlashState>,
    /// Number of overlay rows drawn in the last frame (used to erase stale overlay).
    prev_overlay_rows: u16,
    /// Provider picker / API key entry mode.
    mode: InputMode,
}

#[derive(Clone, Default)]
enum InputMode {
    #[default]
    Normal,
    /// Showing the provider list picker.
    ProviderPicker(ProviderPickerState),
    /// Typing the API key for a chosen provider.
    ApiKeyEntry { provider_id: &'static str, provider_name: &'static str, buf: String },
    /// Showing the model list picker.
    ModelPicker(ModelPickerState),
}

// ── Private types ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ProviderPickerState {
    selected: usize,
}

impl ProviderPickerState {
    fn new() -> Self { Self { selected: 0 } }

    fn move_up(&mut self) {
        if self.selected == 0 {
            self.selected = PROVIDERS.len() - 1;
        } else {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        self.selected = (self.selected + 1) % PROVIDERS.len();
    }

    fn selected_provider(&self) -> (&'static str, &'static str, &'static str) {
        PROVIDERS[self.selected]
    }
}

#[derive(Clone)]
struct ModelPickerState {
    selected: usize,
    scroll:   usize,
}

impl ModelPickerState {
    fn new() -> Self { Self { selected: 0, scroll: 0 } }

    fn move_up(&mut self) {
        if self.selected == 0 {
            self.selected = FREE_MODELS.len() - 1;
        } else {
            self.selected -= 1;
        }
        self.sync_scroll();
    }

    fn move_down(&mut self) {
        self.selected = (self.selected + 1) % FREE_MODELS.len();
        self.sync_scroll();
    }

    fn sync_scroll(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + MAX_VISIBLE {
            self.scroll = self.selected - MAX_VISIBLE + 1;
        }
    }

    fn selected_model(&self) -> (&'static str, &'static str, &'static str) {
        FREE_MODELS[self.selected]
    }
}

#[derive(Clone)]
struct SlashState {
    /// Currently highlighted index within `matches`.
    selected: usize,
    /// Filtered list of (command, description) matching the current prefix.
    matches:  Vec<(&'static str, &'static str)>,
}

impl SlashState {
    fn new(prefix: &str) -> Self {
        Self {
            selected: 0,
            matches:  Self::filter(prefix),
        }
    }

    fn update(&mut self, prefix: &str) {
        self.matches  = Self::filter(prefix);
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }

    fn filter(prefix: &str) -> Vec<(&'static str, &'static str)> {
        let p = prefix.to_ascii_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|(cmd, _)| cmd.to_ascii_lowercase().starts_with(p.as_str()))
            .copied()
            .collect()
    }

    fn selected_cmd(&self) -> Option<&'static str> {
        self.matches.get(self.selected).map(|(cmd, _)| *cmd)
    }

    fn move_up(&mut self) {
        if !self.matches.is_empty() {
            if self.selected == 0 {
                self.selected = self.matches.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    fn move_down(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
        }
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn new() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

// ── InputBox impl ─────────────────────────────────────────────────────────────

impl InputBox {
    pub fn new() -> Self {
        Self {
            buf:    String::new(),
            cursor: 0,
            slash:  None,
            prev_overlay_rows: 0,
            mode: InputMode::Normal,
        }
    }

    pub fn read(&mut self, _hint: &str, state: &State) -> io::Result<InputResult> {
        let _raw = RawModeGuard::new()?;

        self.reserve(state)?;

        let result = self.event_loop(state)?;

        match &result {
            InputResult::Submit(_) => {
                // Clear slash overlay rows before locking the box.
                self.erase_slash_overlay()?;

                let mut out = io::stderr();
                out.queue(cursor::RestorePosition)?;
                out.queue(cursor::MoveDown(BOX_ROWS + 1))?;
                out.queue(cursor::MoveToColumn(0))?;
                out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
                out.flush()?;

                self.draw_locked()?;
                self.move_below_box()?;
                self.buf.clear();
                self.cursor = 0;
                self.slash = None;
                self.mode = InputMode::Normal;
                self.prev_overlay_rows = 0;
            }
            _ => {
                self.erase_slash_overlay()?;
                self.erase()?;
                self.buf.clear();
                self.cursor = 0;
                self.slash = None;
                self.mode = InputMode::Normal;
                self.prev_overlay_rows = 0;
            }
        }

        let mut out = io::stderr();
        out.queue(cursor::SetCursorStyle::DefaultUserShape)?;
        out.flush()?;

        Ok(result)
    }

    fn reserve(&mut self, state: &State) -> io::Result<()> {
        let mut out = io::stderr();
        // Reserve rows for: box (3) + status (1) + largest possible overlay.
        // Provider list = PROVIDERS.len() + 2; model list = min(FREE_MODELS.len(), MAX_VISIBLE) + 2;
        // slash = MAX_VISIBLE + 2; API key = 4.
        let model_count = (FREE_MODELS.len() as u16).min(MAX_VISIBLE as u16);
        let max_overlay = (PROVIDERS.len() as u16 + 2)
            .max(model_count + 2)
            .max(MAX_VISIBLE as u16 + 2)
            .max(4);
        let total_rows = BOX_ROWS + 1 + max_overlay;
        for _ in 0..total_rows {
            out.queue(style::Print("\n"))?;
        }
        out.queue(cursor::MoveUp(total_rows))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(cursor::SavePosition)?;
        out.queue(cursor::Show)?;
        out.queue(cursor::SetCursorStyle::SteadyBlock)?;
        out.flush()?;
        self.draw(Some(state))?;
        Ok(())
    }

    // ── Event loop ────────────────────────────────────────────────────────────

    fn event_loop(&mut self, state: &State) -> io::Result<InputResult> {
        loop {
            match event::read()? {
                Event::Key(KeyEvent { code, modifiers, .. }) => {

                    // ── Provider picker mode ───────────────────────────────
                    if let InputMode::ProviderPicker(_) = &self.mode {
                        match (code, modifiers) {
                            (KeyCode::Esc, _) => {
                                self.mode = InputMode::Normal;
                            }
                            (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                                if let InputMode::ProviderPicker(ref mut p) = self.mode {
                                    p.move_up();
                                }
                            }
                            (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                                if let InputMode::ProviderPicker(ref mut p) = self.mode {
                                    p.move_down();
                                }
                            }
                            (KeyCode::Enter, _) | (KeyCode::Tab, _) => {
                                if let InputMode::ProviderPicker(ref p) = self.mode {
                                    let (id, name, _env) = p.selected_provider();
                                    self.mode = InputMode::ApiKeyEntry {
                                        provider_id:   id,
                                        provider_name: name,
                                        buf:           String::new(),
                                    };
                                }
                            }
                            (KeyCode::Char('c'), KeyModifiers::CONTROL)
                            | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                return Ok(InputResult::Quit);
                            }
                            _ => {}
                        }
                        self.draw(Some(state))?;
                        continue;
                    }

                    // ── API key entry mode ─────────────────────────────────
                    if let InputMode::ApiKeyEntry { .. } = &self.mode {
                        match (code, modifiers) {
                            (KeyCode::Esc, _) => {
                                // Back to provider picker.
                                self.mode = InputMode::ProviderPicker(ProviderPickerState::new());
                            }
                            (KeyCode::Enter, _) => {
                                if let InputMode::ApiKeyEntry { provider_id, buf, .. } =
                                    std::mem::replace(&mut self.mode, InputMode::Normal)
                                {
                                    let key = buf.trim().to_string();
                                    if !key.is_empty() {
                                        return Ok(InputResult::Submit(
                                            format!("/provider {} {}", provider_id, key),
                                        ));
                                    }
                                    // Empty key — go back to picker.
                                    self.mode = InputMode::ProviderPicker(ProviderPickerState::new());
                                }
                            }
                            (KeyCode::Backspace, _) => {
                                if let InputMode::ApiKeyEntry { ref mut buf, .. } = self.mode {
                                    buf.pop();
                                }
                            }
                            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                                if let InputMode::ApiKeyEntry { ref mut buf, .. } = self.mode {
                                    buf.clear();
                                }
                            }
                            (KeyCode::Char('c'), KeyModifiers::CONTROL)
                            | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                return Ok(InputResult::Quit);
                            }
                            (KeyCode::Char(c), mods)
                                if !mods.contains(KeyModifiers::CONTROL)
                                    && !mods.contains(KeyModifiers::ALT) =>
                            {
                                if let InputMode::ApiKeyEntry { ref mut buf, .. } = self.mode {
                                    buf.push(c);
                                }
                            }
                            _ => {}
                        }
                        self.draw(Some(state))?;
                        continue;
                    }

                    // ── Model picker mode ──────────────────────────────────
                    if let InputMode::ModelPicker(_) = &self.mode {
                        match (code, modifiers) {
                            (KeyCode::Esc, _) => {
                                self.mode = InputMode::Normal;
                            }
                            (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                                if let InputMode::ModelPicker(ref mut m) = self.mode {
                                    m.move_up();
                                }
                            }
                            (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                                if let InputMode::ModelPicker(ref mut m) = self.mode {
                                    m.move_down();
                                }
                            }
                            (KeyCode::Enter, _) | (KeyCode::Tab, _) => {
                                if let InputMode::ModelPicker(ref m) = self.mode {
                                    let (id, _name, _ctx) = m.selected_model();
                                    return Ok(InputResult::Submit(format!("/model {}", id)));
                                }
                            }
                            (KeyCode::Char('c'), KeyModifiers::CONTROL)
                            | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                return Ok(InputResult::Quit);
                            }
                            _ => {}
                        }
                        self.draw(Some(state))?;
                        continue;
                    }

                    match (code, modifiers) {
                        // ── Slash-mode navigation ──────────────────────────
                        (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL)
                            if self.slash.is_some() =>
                        {
                            self.slash.as_mut().unwrap().move_up();
                        }

                        (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL)
                            if self.slash.is_some() =>
                        {
                            self.slash.as_mut().unwrap().move_down();
                        }

                        (KeyCode::Tab, _) | (KeyCode::Enter, _)
                            if self.slash.is_some()
                                && self.slash.as_ref().unwrap().matches.len() > 0 =>
                        {
                            // Complete the selected command.
                            if let Some(cmd) = self.slash.as_ref().unwrap().selected_cmd() {
                                if cmd == "/provider" {
                                    // Launch provider picker instead of normal completion.
                                    self.buf.clear();
                                    self.cursor = 0;
                                    self.slash = None;
                                    self.mode = InputMode::ProviderPicker(ProviderPickerState::new());
                                } else if cmd == "/model" {
                                    // Launch model picker instead of normal completion.
                                    self.buf.clear();
                                    self.cursor = 0;
                                    self.slash = None;
                                    self.mode = InputMode::ModelPicker(ModelPickerState::new());
                                } else {
                                    // Replace buffer with completed command + space.
                                    self.buf = format!("{cmd} ");
                                    self.cursor = self.buf.len();
                                    self.slash = None;
                                }
                            }
                        }

                        (KeyCode::Esc, _) if self.slash.is_some() => {
                            // Dismiss overlay, keep buffer as-is.
                            self.slash = None;
                        }

                        // ── Normal Enter (submit) ──────────────────────────
                        (KeyCode::Enter, _) => {
                            let text = self.buf.trim().to_string();
                            if text.is_empty() {
                                continue;
                            }
                            return Ok(InputResult::Submit(text));
                        }

                        // ── Quit / abort ───────────────────────────────────
                        (KeyCode::Char('c'), KeyModifiers::CONTROL)
                        | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                            return Ok(InputResult::Quit);
                        }
                        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                            return Ok(InputResult::Abort);
                        }

                        // ── Editing ────────────────────────────────────────
                        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                            self.buf.clear();
                            self.cursor = 0;
                            self.slash = None;
                        }
                        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                            self.delete_word_back();
                            self.sync_slash();
                        }
                        (KeyCode::Char('a'), KeyModifiers::CONTROL) | (KeyCode::Home, _) => {
                            self.cursor = 0;
                        }
                        (KeyCode::Char('e'), KeyModifiers::CONTROL) | (KeyCode::End, _) => {
                            self.cursor = self.buf.len();
                        }
                        (KeyCode::Left, _) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                            self.move_left();
                        }
                        (KeyCode::Right, _) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                            self.move_right();
                        }
                        (KeyCode::Backspace, _) => {
                            self.backspace();
                            self.sync_slash();
                        }
                        (KeyCode::Delete, _) => {
                            self.delete_forward();
                            self.sync_slash();
                        }
                        (KeyCode::Char(c), mods)
                            if !mods.contains(KeyModifiers::CONTROL)
                                && !mods.contains(KeyModifiers::ALT) =>
                        {
                            self.insert(c);
                            self.sync_slash();
                        }
                        _ => {}
                    }
                    self.draw(Some(state))?;
                }
                Event::Resize(_, _) => {
                    self.draw(Some(state))?;
                }
                _ => {}
            }
        }
    }

    // ── Slash sync ────────────────────────────────────────────────────────────

    /// After every buffer mutation, re-sync slash mode.
    fn sync_slash(&mut self) {
        if self.buf.starts_with('/') {
            match self.slash.as_mut() {
                Some(s) => s.update(&self.buf),
                None    => self.slash = Some(SlashState::new(&self.buf)),
            }
        } else {
            self.slash = None;
        }
    }

    // ── Drawing ───────────────────────────────────────────────────────────────

    fn draw(&mut self, state: Option<&State>) -> io::Result<()> {
        let t0 = std::time::Instant::now();
        let mut out = io::stderr();
        let term_size_start = std::time::Instant::now();
        let cols          = usize::from(terminal::size().unwrap_or((80, 24)).0).max(1);
        let term_size_us = term_size_start.elapsed().as_micros();
        let content_width = cols.saturating_sub(4);
        let input_width   = content_width.saturating_sub(UnicodeWidthStr::width(PROMPT));
        let (visible, cursor_col) = self.visible_slice(input_width);
        let visible_width  = UnicodeWidthStr::width(visible.as_str());
        let prompt_width   = UnicodeWidthStr::width(PROMPT);

        out.queue(cursor::Hide)?;
        out.queue(cursor::RestorePosition)?;

        // ── Top border ────────────────────────────────────────────────────
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', "", cols).with(Color::DarkGreen),
        ))?;

        // ── Input row ─────────────────────────────────────────────────────
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent("│ ".with(Color::DarkGreen)))?;
        out.queue(style::PrintStyledContent(PROMPT.with(Color::Green)))?;
        if visible.is_empty() {
            let hint        = "describe your task…";
            let hint_width  = UnicodeWidthStr::width(hint).min(input_width);
            let cut         = hint.char_indices()
                .nth(hint_width)
                .map(|(i, _)| i)
                .unwrap_or(hint.len());
            out.queue(style::PrintStyledContent(hint[..cut].with(Color::DarkGrey)))?;
            out.queue(style::Print(" ".repeat(input_width.saturating_sub(hint_width))))?;
        } else {
            out.queue(style::Print(&visible))?;
            out.queue(style::Print(" ".repeat(input_width.saturating_sub(visible_width))))?;
        }
        out.queue(style::PrintStyledContent(" │".with(Color::DarkGreen)))?;

        // ── Bottom border ─────────────────────────────────────────────────
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', "", cols).with(Color::DarkGrey),
        ))?;

        // ── Status line (row: RestorePosition + 3) ────────────────────────
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        if let Some(st) = state {
            self.draw_status_line_internal(&mut out, st)?;
        }

        // ── Slash overlay (rows: RestorePosition + 4 … +4+N) ─────────────
        // Erase previous overlay rows (below status line), then redraw if still active.
        let old_overlay_rows = self.prev_overlay_rows;
        if old_overlay_rows > 0 {
            for _ in 0..old_overlay_rows {
                out.queue(cursor::MoveDown(1))?;
                out.queue(cursor::MoveToColumn(0))?;
                out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
            }
            out.queue(cursor::MoveUp(old_overlay_rows))?;
            out.queue(cursor::MoveToColumn(0))?;
        }

        let new_overlay_rows = if let InputMode::ProviderPicker(ref picker) = self.mode {
            self.draw_provider_overlay(&mut out, picker, cols)?;
            PROVIDERS.len() as u16 + 2
        } else if let InputMode::ApiKeyEntry { provider_name, ref buf, .. } = self.mode {
            self.draw_apikey_overlay(&mut out, provider_name, buf, cols)?;
            4u16 // top border + prompt row + input row + bottom border
        } else if let InputMode::ModelPicker(ref picker) = self.mode {
            self.draw_model_overlay(&mut out, picker, cols)?;
            (FREE_MODELS.len() as u16).min(MAX_VISIBLE as u16) + 2
        } else if let Some(slash) = self.slash.clone() {
            let count = slash.matches.len().min(MAX_VISIBLE);
            if count > 0 {
                // Cursor is at status line row; draw_slash_overlay starts one MoveDown below.
                self.draw_slash_overlay(&mut out, &slash, cols)?;
                count as u16 + 2 // top border + items + bottom border
            } else {
                0
            }
        } else {
            0
        };
        self.prev_overlay_rows = new_overlay_rows;

        // ── Restore cursor into input row ────────────────────────────────
        // Cursor is at: status line row + new_overlay_rows (0 if no overlay drawn).
        // Input row is RestorePosition+1 = status line row - 2.
        out.queue(cursor::MoveUp(2 + new_overlay_rows))?;
        let cursor_x = (2 + prompt_width + cursor_col).min(cols.saturating_sub(1)) as u16;
        out.queue(cursor::MoveToColumn(cursor_x))?;
        out.queue(cursor::Show)?;
        out.flush()?;
        let draw_us = t0.elapsed().as_micros();
        if std::env::var("ITER_PROFILE").is_ok() {
            let _ = writeln!(std::io::stderr(), "[profile] input::draw total={}µs terminal::size={}µs",
                draw_us, term_size_us);
        }
        Ok(())
    }

    /// Draw the slash-command picker overlay below the input box.
    ///
    /// Called with cursor at the status line row (RestorePosition + 3).
    /// Draws the overlay immediately below that row.
    /// Returns with cursor still at the status line row.
    fn draw_slash_overlay(
        &self,
        out:   &mut io::Stderr,
        slash: &SlashState,
        cols:  usize,
    ) -> io::Result<()> {
        let matches = &slash.matches;
        if matches.is_empty() {
            return Ok(());
        }

        let visible_count = matches.len().min(MAX_VISIBLE);

        let scroll_start = if slash.selected >= visible_count {
            slash.selected - visible_count + 1
        } else {
            0
        };
        let visible_slice = &matches[scroll_start..scroll_start + visible_count];

        let cmd_col_width = visible_slice
            .iter()
            .map(|(cmd, _)| UnicodeWidthStr::width(*cmd))
            .max()
            .unwrap_or(8);

        // Top border — one row below status line
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', " commands ", cols).with(Color::DarkCyan),
        ))?;

        for (i, (cmd, desc)) in visible_slice.iter().enumerate() {
            let abs_idx     = scroll_start + i;
            let is_selected = abs_idx == slash.selected;

            out.queue(cursor::MoveDown(1))?;
            out.queue(cursor::MoveToColumn(0))?;

            let inner      = cols.saturating_sub(4);
            let cmd_width  = UnicodeWidthStr::width(*cmd);
            let gap        = cmd_col_width.saturating_sub(cmd_width) + 2;
            let desc_avail = inner.saturating_sub(cmd_width + gap);
            let desc_trunc = truncate_str(desc, desc_avail);
            let desc_width = UnicodeWidthStr::width(desc_trunc.as_str());
            let pad        = inner.saturating_sub(cmd_width + gap + desc_width);

            out.queue(style::PrintStyledContent("│ ".with(Color::DarkCyan)))?;
            if is_selected {
                out.queue(style::PrintStyledContent(cmd.with(Color::White).bold()))?;
                out.queue(style::Print(" ".repeat(gap)))?;
                out.queue(style::PrintStyledContent(desc_trunc.as_str().with(Color::DarkGrey)))?;
                out.queue(style::Print(" ".repeat(pad)))?;
                out.queue(style::PrintStyledContent(" │".with(Color::DarkCyan)))?;
            } else {
                out.queue(style::PrintStyledContent(cmd.with(Color::Cyan)))?;
                out.queue(style::Print(" ".repeat(gap)))?;
                out.queue(style::PrintStyledContent(desc_trunc.as_str().with(Color::DarkGrey)))?;
                out.queue(style::Print(" ".repeat(pad)))?;
                out.queue(style::PrintStyledContent(" │".with(Color::DarkCyan)))?;
            }
        }

        // Bottom border
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', "", cols).with(Color::DarkCyan),
        ))?;
        // Cursor is now at bottom of overlay. draw() will MoveUp back to input row.

        Ok(())
    }

    /// Draw the provider picker overlay.
    fn draw_provider_overlay(
        &self,
        out:    &mut io::Stderr,
        picker: &ProviderPickerState,
        cols:   usize,
    ) -> io::Result<()> {
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', " select provider ", cols).with(Color::Magenta),
        ))?;

        for (i, (id, name, env_key)) in PROVIDERS.iter().enumerate() {
            let is_selected = i == picker.selected;
            out.queue(cursor::MoveDown(1))?;
            out.queue(cursor::MoveToColumn(0))?;

            let inner  = cols.saturating_sub(4);
            let id_w   = UnicodeWidthStr::width(*id);
            let name_w = UnicodeWidthStr::width(*name);
            let env_w  = UnicodeWidthStr::width(*env_key);
            let gap1   = 14usize.saturating_sub(id_w);
            let gap2   = 2usize;
            let pad    = inner.saturating_sub(id_w + gap1 + name_w + gap2 + env_w);

            out.queue(style::PrintStyledContent("│ ".with(Color::Magenta)))?;
            if is_selected {
                out.queue(style::PrintStyledContent(id.with(Color::White).bold()))?;
                out.queue(style::Print(" ".repeat(gap1)))?;
                out.queue(style::PrintStyledContent(name.with(Color::White)))?;
                out.queue(style::Print(" ".repeat(gap2)))?;
                out.queue(style::PrintStyledContent(env_key.with(Color::DarkGrey)))?;
                out.queue(style::Print(" ".repeat(pad)))?;
                out.queue(style::PrintStyledContent(" │".with(Color::Magenta)))?;
            } else {
                out.queue(style::PrintStyledContent(id.with(Color::DarkMagenta)))?;
                out.queue(style::Print(" ".repeat(gap1)))?;
                out.queue(style::PrintStyledContent(name.with(Color::Grey)))?;
                out.queue(style::Print(" ".repeat(gap2)))?;
                out.queue(style::PrintStyledContent(env_key.with(Color::DarkGrey)))?;
                out.queue(style::Print(" ".repeat(pad)))?;
                out.queue(style::PrintStyledContent(" │".with(Color::Magenta)))?;
            }
        }

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', " ↑↓ select  ↵ confirm  esc cancel ", cols).with(Color::DarkMagenta),
        ))?;

        Ok(())
    }

    /// Draw the API key entry overlay.
    fn draw_apikey_overlay(
        &self,
        out:           &mut io::Stderr,
        provider_name: &str,
        key_buf:       &str,
        cols:          usize,
    ) -> io::Result<()> {
        let title = format!(" {} API key ", provider_name);

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', &title, cols).with(Color::Magenta),
        ))?;

        // Hint row
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        let hint  = "paste or type your API key, then press ↵";
        let hint_w = UnicodeWidthStr::width(hint);
        let inner  = cols.saturating_sub(4);
        let pad    = inner.saturating_sub(hint_w);
        out.queue(style::PrintStyledContent("│ ".with(Color::Magenta)))?;
        out.queue(style::PrintStyledContent(hint.with(Color::DarkGrey)))?;
        out.queue(style::Print(" ".repeat(pad)))?;
        out.queue(style::PrintStyledContent(" │".with(Color::Magenta)))?;

        // Key input row — show last 4 chars, rest masked
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        if key_buf.is_empty() {
            let ph   = "sk-…";
            let ph_w = UnicodeWidthStr::width(ph);
            let pad2 = inner.saturating_sub(ph_w);
            out.queue(style::PrintStyledContent("│ ".with(Color::Magenta)))?;
            out.queue(style::PrintStyledContent(ph.with(Color::DarkGrey)))?;
            out.queue(style::Print(" ".repeat(pad2)))?;
            out.queue(style::PrintStyledContent(" │".with(Color::Magenta)))?;
        } else {
            let suffix: String = key_buf.chars().rev().take(4).collect::<String>()
                                         .chars().rev().collect();
            let masked_count = key_buf.len().saturating_sub(4);
            let masked       = "•".repeat(masked_count);
            let full_w       = masked_count + UnicodeWidthStr::width(suffix.as_str());
            let pad2         = inner.saturating_sub(full_w);
            out.queue(style::PrintStyledContent("│ ".with(Color::Magenta)))?;
            out.queue(style::PrintStyledContent(masked.as_str().with(Color::DarkGrey)))?;
            out.queue(style::PrintStyledContent(suffix.as_str().with(Color::White)))?;
            out.queue(style::Print(" ".repeat(pad2)))?;
            out.queue(style::PrintStyledContent(" │".with(Color::Magenta)))?;
        }

        // Bottom border
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', " ↵ confirm  esc back ", cols).with(Color::DarkMagenta),
        ))?;

        Ok(())
    }

    /// Draw the model picker overlay.
    fn draw_model_overlay(
        &self,
        out:    &mut io::Stderr,
        picker: &ModelPickerState,
        cols:   usize,
    ) -> io::Result<()> {
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', " select model ", cols).with(Color::Cyan),
        ))?;

        let visible_count = FREE_MODELS.len().min(MAX_VISIBLE);
        let visible_slice = &FREE_MODELS[picker.scroll..picker.scroll + visible_count];

        let model_col_width = visible_slice
            .iter()
            .map(|(_, name, _)| UnicodeWidthStr::width(*name))
            .max()
            .unwrap_or(16);

        for (i, (_id, name, ctx)) in visible_slice.iter().enumerate() {
            let abs_idx     = picker.scroll + i;
            let is_selected = abs_idx == picker.selected;

            out.queue(cursor::MoveDown(1))?;
            out.queue(cursor::MoveToColumn(0))?;

            let inner  = cols.saturating_sub(4);
            let name_w = UnicodeWidthStr::width(*name);
            let ctx_w  = UnicodeWidthStr::width(*ctx);
            let gap    = model_col_width.saturating_sub(name_w) + 2;
            let pad    = inner.saturating_sub(name_w + gap + ctx_w);

            out.queue(style::PrintStyledContent("│ ".with(Color::Cyan)))?;
            if is_selected {
                out.queue(style::PrintStyledContent(name.with(Color::White).bold()))?;
                out.queue(style::Print(" ".repeat(gap)))?;
                out.queue(style::PrintStyledContent(ctx.with(Color::DarkGrey)))?;
                out.queue(style::Print(" ".repeat(pad)))?;
                out.queue(style::PrintStyledContent(" │".with(Color::Cyan)))?;
            } else {
                out.queue(style::PrintStyledContent(name.with(Color::DarkCyan)))?;
                out.queue(style::Print(" ".repeat(gap)))?;
                out.queue(style::PrintStyledContent(ctx.with(Color::DarkGrey)))?;
                out.queue(style::Print(" ".repeat(pad)))?;
                out.queue(style::PrintStyledContent(" │".with(Color::Cyan)))?;
            }
        }

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', " ↑↓ select  ↵ confirm  esc cancel ", cols).with(Color::DarkCyan),
        ))?;

        Ok(())
    }

    /// Erase any rows the slash overlay occupies below the box.
    fn erase_slash_overlay(&self) -> io::Result<()> {
        if self.prev_overlay_rows == 0 {
            return Ok(());
        }
        let mut out = io::stderr();
        // RestorePosition = box top border. Status line = +3. Overlay starts at +4.
        out.queue(cursor::RestorePosition)?;
        out.queue(cursor::MoveDown(4))?;
        for _ in 0..self.prev_overlay_rows {
            out.queue(cursor::MoveToColumn(0))?;
            out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
            out.queue(cursor::MoveDown(1))?;
        }
        out.flush()
    }

    fn draw_status_line_internal(&self, out: &mut std::io::Stderr, state: &State) -> io::Result<()> {
        let ctx_color = if state.context_pct > 80.0 {
            Color::DarkRed
        } else if state.context_pct > 50.0 {
            Color::DarkYellow
        } else {
            Color::DarkGreen
        };

        let bar_width = 10usize;
        let filled = ((state.context_pct / 100.0) * bar_width as f32).round() as usize;
        let filled = filled.min(bar_width);
        let bar: String = format!("[{}{}]",
            "█".repeat(filled),
            "─".repeat(bar_width - filled),
        );

        write!(
            out,
            "  {} {} {} {}  {} {} {} {}  {} {} {:.0}%  {} ${}  {} {}",
            "in".with(Color::DarkGrey),
            format_tokens(state.tokens_input).with(Color::White),
            "out".with(Color::DarkGrey),
            format_tokens(state.tokens_output).with(Color::White),
            "↑".with(Color::DarkGrey),
            format_tokens(state.tokens_cache_write).with(Color::DarkCyan),
            "↓".with(Color::DarkGrey),
            format_tokens(state.tokens_cache_read).with(Color::Cyan),
            "ctx".with(Color::DarkGrey),
            bar.with(ctx_color),
            state.context_pct,
            "cost".with(Color::DarkGrey),
            format!("{:.4}", state.cost).with(Color::White),
            "turn".with(Color::DarkGrey),
            state.turns.to_string().with(Color::White),
        )?;
        Ok(())
    }

    fn draw_locked(&self) -> io::Result<()> {
        let mut out    = io::stderr();
        let cols          = usize::from(terminal::size().unwrap_or((80, 24)).0).max(1);
        let content_width = cols.saturating_sub(4);
        let input_width   = content_width.saturating_sub(UnicodeWidthStr::width(PROMPT));
        let (visible, _)  = self.visible_slice(input_width);
        let visible_width  = UnicodeWidthStr::width(visible.as_str());

        out.queue(cursor::RestorePosition)?;

        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', "", cols).with(Color::DarkGrey),
        ))?;

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent("│ ".with(Color::DarkGrey)))?;
        out.queue(style::PrintStyledContent(PROMPT.with(Color::DarkGrey)))?;
        out.queue(style::PrintStyledContent(visible.with(Color::DarkGrey)))?;
        out.queue(style::Print(" ".repeat(input_width.saturating_sub(visible_width))))?;
        out.queue(style::PrintStyledContent(" │".with(Color::DarkGrey)))?;

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', "", cols).with(Color::DarkGrey),
        ))?;

        out.flush()
    }

    fn move_below_box(&self) -> io::Result<()> {
        let mut out = io::stderr();
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.flush()
    }

    fn erase(&self) -> io::Result<()> {
        let mut out = io::stderr();
        out.queue(cursor::RestorePosition)?;
        for _ in 0..(BOX_ROWS + 2) {
            out.queue(cursor::MoveToColumn(0))?;
            out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
            out.queue(cursor::MoveDown(1))?;
        }
        out.queue(cursor::RestorePosition)?;
        out.flush()
    }

    // ── Text editing ──────────────────────────────────────────────────────────

    fn visible_slice(&self, avail: usize) -> (String, usize) {
        let chars: Vec<(usize, usize, char)> = self
            .buf
            .char_indices()
            .map(|(byte, ch)| (byte, UnicodeWidthChar::width(ch).unwrap_or(0), ch))
            .collect();
        let total_width: usize      = chars.iter().map(|(_, w, _)| *w).sum();
        let cursor_display_col: usize = chars
            .iter()
            .take_while(|(b, _, _)| *b < self.cursor)
            .map(|(_, w, _)| *w)
            .sum();

        if total_width <= avail {
            return (self.buf.clone(), cursor_display_col);
        }

        let scroll_start_col = cursor_display_col.saturating_sub(avail * 2 / 3);
        let mut source_col   = 0usize;
        let mut display_col  = 0usize;
        let mut visible      = String::new();
        let mut cursor_col   = 0usize;
        let mut cursor_set   = false;

        for (byte, width, ch) in &chars {
            let char_start = source_col;
            source_col += width;
            if char_start < scroll_start_col { continue; }
            if display_col + width > avail   { break; }
            if *byte == self.cursor && !cursor_set {
                cursor_col = display_col;
                cursor_set = true;
            }
            visible.push(*ch);
            display_col += width;
        }
        if !cursor_set { cursor_col = display_col; }
        (visible, cursor_col)
    }

    fn insert(&mut self, c: char) {
        self.buf.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 { return; }
        let ch = self.char_before_cursor();
        self.cursor -= ch.len_utf8();
        self.buf.remove(self.cursor);
    }

    fn delete_forward(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
        }
    }

    fn move_left(&mut self) {
        if self.cursor == 0 { return; }
        self.cursor -= self.char_before_cursor().len_utf8();
    }

    fn move_right(&mut self) {
        if let Some(ch) = self.buf[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
        }
    }

    fn delete_word_back(&mut self) {
        while self.cursor > 0 && self.char_before_cursor().is_whitespace() {
            let len = self.char_before_cursor().len_utf8();
            self.cursor -= len;
            self.buf.remove(self.cursor);
        }
        while self.cursor > 0 && !self.char_before_cursor().is_whitespace() {
            let len = self.char_before_cursor().len_utf8();
            self.cursor -= len;
            self.buf.remove(self.cursor);
        }
    }

    fn char_before_cursor(&self) -> char {
        self.buf[..self.cursor].chars().next_back().unwrap()
    }
}

// ── Free functions ─────────────────────────────────────────────────────────────

pub fn format_tokens(n: u32) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn border_line(left: char, right: char, label: &str, width: usize) -> String {
    if width <= 1 { return left.to_string(); }
    let label_width = UnicodeWidthStr::width(label);
    let inner = width.saturating_sub(2);
    if inner <= 1 { return format!("{left}{right}"); }
    if label_width + 1 >= inner {
        return format!("{left}{}{right}", "─".repeat(inner));
    }
    format!("{left}─{label}{}{right}", "─".repeat(inner.saturating_sub(label_width + 1)))
}

/// Truncate `s` to at most `max_display_width` terminal columns.
fn truncate_str(s: &str, max_display_width: usize) -> String {
    let mut width  = 0usize;
    let mut result = String::new();
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + w > max_display_width { break; }
        result.push(ch);
        width += w;
    }
    result
}
