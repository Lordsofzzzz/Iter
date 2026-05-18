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
}

// ── Private types ─────────────────────────────────────────────────────────────

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
                self.prev_overlay_rows = 0;
            }
            _ => {
                self.erase_slash_overlay()?;
                self.erase()?;
                self.buf.clear();
                self.cursor = 0;
                self.slash = None;
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
        for _ in 0..(BOX_ROWS + 1) {
            out.queue(style::Print("\n"))?;
        }
        out.queue(cursor::MoveUp(BOX_ROWS + 1))?;
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
                                // Replace buffer with completed command + space.
                                self.buf = format!("{cmd} ");
                                self.cursor = self.buf.len();
                                self.slash = None;
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
        let mut out = io::stderr();
        let cols          = usize::from(terminal::size().unwrap_or((80, 24)).0).max(1);
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

        let new_overlay_rows = if let Some(slash) = self.slash.clone() {
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
        out.flush()
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

fn format_tokens(n: u32) -> String {
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
