//! Inline bordered input box.
//!
//! Draws a 3-row box wherever the cursor currently is and redraws in-place.
//! No alternate screen, no scroll regions, no pinning to bottom.
//! Output flows naturally below the box after submit.

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

pub enum InputResult {
    Submit(String),
    Quit,
    Abort,
}

pub struct InputBox {
    buf: String,
    cursor: usize,
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

impl InputBox {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            cursor: 0,
        }
    }

    pub fn read(&mut self, _hint: &str, state: &State) -> io::Result<InputResult> {
        let _raw = RawModeGuard::new()?;

        self.reserve(state)?;

        let result = self.event_loop(state)?;

        match &result {
            InputResult::Submit(_) => {
                // Clear the status line before locking
                let mut out = io::stderr();
                out.queue(cursor::RestorePosition)?;
                out.queue(cursor::MoveDown(BOX_ROWS + 1))?;
                out.queue(cursor::MoveToColumn(0))?;
                out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
                out.flush()?;
                
                self.draw_locked()?;
                self.move_below_box()?;
            }
            _ => {
                self.erase()?;
                self.buf.clear();
                self.cursor = 0;
            }
        }

        // Restore default cursor style
        let mut out = io::stderr();
        out.queue(cursor::SetCursorStyle::DefaultUserShape)?;
        out.flush()?;

        Ok(result)
    }

fn reserve(&self, state: &State) -> io::Result<()> {
        let mut out = io::stderr();
        // Reserve an extra row for the status line
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

    fn event_loop(&mut self, state: &State) -> io::Result<InputResult> {
        loop {
            match event::read()? {
                Event::Key(KeyEvent { code, modifiers, .. }) => {
                    match (code, modifiers) {
                        (KeyCode::Enter, _) => {
                            let text = self.buf.trim().to_string();
                            if text.is_empty() {
                                continue;
                            }
                            return Ok(InputResult::Submit(text));
                        }
                        (KeyCode::Char('c'), KeyModifiers::CONTROL)
                        | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                            return Ok(InputResult::Quit);
                        }
                        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                            return Ok(InputResult::Abort);
                        }
                        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                            self.buf.clear();
                            self.cursor = 0;
                        }
                        (KeyCode::Char('w'), KeyModifiers::CONTROL) => self.delete_word_back(),
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
                        (KeyCode::Backspace, _) => self.backspace(),
                        (KeyCode::Delete, _) => self.delete_forward(),
                        (KeyCode::Char(c), mods)
                            if !mods.contains(KeyModifiers::CONTROL)
                                && !mods.contains(KeyModifiers::ALT) =>
                        {
                            self.insert(c);
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

    fn draw(&self, state: Option<&State>) -> io::Result<()> {
        let mut out = io::stderr();
        let cols = usize::from(terminal::size().unwrap_or((80, 24)).0).max(1);
        let content_width = cols.saturating_sub(4);
        let input_width = content_width.saturating_sub(UnicodeWidthStr::width(PROMPT));
        let (visible, cursor_col) = self.visible_slice(input_width);
        let visible_width = UnicodeWidthStr::width(visible.as_str());
        let prompt_width = UnicodeWidthStr::width(PROMPT);

        out.queue(cursor::Hide)?;
        out.queue(cursor::RestorePosition)?;

        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', "", cols).with(Color::DarkGreen),
        ))?;

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent("│ ".with(Color::DarkGreen)))?;
        out.queue(style::PrintStyledContent(PROMPT.with(Color::Green)))?;
        if visible.is_empty() {
            let hint = "describe your task…";
            let hint_width = UnicodeWidthStr::width(hint).min(input_width);
            out.queue(style::PrintStyledContent(
                hint[..hint.char_indices().nth(hint_width).map(|(i,_)| i).unwrap_or(hint.len())]
                    .with(Color::DarkGrey)
            ))?;
            out.queue(style::Print(" ".repeat(input_width.saturating_sub(hint_width))))?;
        } else {
            out.queue(style::Print(&visible))?;
            out.queue(style::Print(" ".repeat(input_width.saturating_sub(visible_width))))?;
        }
        out.queue(style::PrintStyledContent(" │".with(Color::DarkGreen)))?;

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', "", cols).with(Color::DarkGrey),
        ))?;

        if let Some(st) = state {
            out.queue(cursor::MoveDown(1))?;
            out.queue(cursor::MoveToColumn(0))?;
            out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
            self.draw_status_line_internal(&mut out, st)?;
            out.queue(cursor::MoveUp(1))?;
        }

        out.queue(cursor::MoveUp(1))?;
        let cursor_x = (2 + prompt_width + cursor_col).min(cols.saturating_sub(1)) as u16;
        out.queue(cursor::MoveToColumn(cursor_x))?;
        out.queue(cursor::Show)?;
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
        let mut out = io::stderr();
        let cols = usize::from(terminal::size().unwrap_or((80, 24)).0).max(1);
        let content_width = cols.saturating_sub(4);
        let input_width = content_width.saturating_sub(UnicodeWidthStr::width(PROMPT));
        let (visible, _) = self.visible_slice(input_width);
        let visible_width = UnicodeWidthStr::width(visible.as_str());

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
        out.queue(cursor::MoveDown(1))?; // start output where status line was
        out.queue(cursor::MoveToColumn(0))?;
        out.flush()
    }

    fn erase(&self) -> io::Result<()> {
        let mut out = io::stderr();
        out.queue(cursor::RestorePosition)?;
        // We need to clear the box (2 rows) + the status line (1 row) = 3 rows
        for _ in 0..(BOX_ROWS + 2) {
            out.queue(cursor::MoveToColumn(0))?;
            out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
            out.queue(cursor::MoveDown(1))?;
        }
        out.queue(cursor::RestorePosition)?;
        out.flush()
    }

    fn visible_slice(&self, avail: usize) -> (String, usize) {
        let chars: Vec<(usize, usize, char)> = self
            .buf
            .char_indices()
            .map(|(byte, ch)| (byte, UnicodeWidthChar::width(ch).unwrap_or(0), ch))
            .collect();
        let total_width: usize = chars.iter().map(|(_, w, _)| *w).sum();
        let cursor_display_col: usize = chars
            .iter()
            .take_while(|(b, _, _)| *b < self.cursor)
            .map(|(_, w, _)| *w)
            .sum();

        if total_width <= avail {
            return (self.buf.clone(), cursor_display_col);
        }

        let scroll_start_col = cursor_display_col.saturating_sub(avail * 2 / 3);
        let mut source_col = 0usize;
        let mut display_col = 0usize;
        let mut visible = String::new();
        let mut cursor_col = 0usize;
        let mut cursor_set = false;

        for (byte, width, ch) in &chars {
            let char_start = source_col;
            source_col += width;
            if char_start < scroll_start_col {
                continue;
            }
            if display_col + width > avail {
                break;
            }
            if *byte == self.cursor && !cursor_set {
                cursor_col = display_col;
                cursor_set = true;
            }
            visible.push(*ch);
            display_col += width;
        }
        if !cursor_set {
            cursor_col = display_col;
        }
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
