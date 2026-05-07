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

    pub fn read(&mut self, _hint: &str) -> io::Result<InputResult> {
        let _raw = RawModeGuard::new()?;

        self.reserve()?;

        let result = self.event_loop()?;

        match &result {
            InputResult::Submit(_) => {
                self.draw_locked()?;
                self.move_below_box()?;
            }
            _ => {
                self.erase()?;
                self.buf.clear();
                self.cursor = 0;
            }
        }

        Ok(result)
    }

    fn reserve(&self) -> io::Result<()> {
        let mut out = io::stderr();
        for _ in 0..BOX_ROWS {
            out.queue(style::Print("\n"))?;
        }
        out.queue(cursor::MoveUp(BOX_ROWS))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(cursor::SavePosition)?;
        out.flush()?;
        self.draw()
    }

    fn event_loop(&mut self) -> io::Result<InputResult> {
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
                    self.draw()?;
                }
                Event::Resize(_, _) => {
                    self.draw()?;
                }
                _ => {}
            }
        }
    }

    fn draw(&self) -> io::Result<()> {
        let mut out = io::stderr();
        let cols = usize::from(terminal::size().unwrap_or((80, 24)).0).max(1);
        let content_width = cols.saturating_sub(4);
        let input_width = content_width.saturating_sub(UnicodeWidthStr::width(PROMPT));
        let (visible, cursor_col) = self.visible_slice(input_width);
        let visible_width = UnicodeWidthStr::width(visible.as_str());
        let prompt_width = UnicodeWidthStr::width(PROMPT);

        out.queue(cursor::RestorePosition)?;

        out.queue(cursor::MoveToColumn(0))?;
        out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', "", cols).with(Color::DarkGreen),
        ))?;

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        out.queue(style::PrintStyledContent("│ ".with(Color::DarkGreen)))?;
        out.queue(style::PrintStyledContent(PROMPT.with(Color::Green)))?;
        out.queue(style::Print(&visible))?;
        out.queue(style::Print(" ".repeat(input_width.saturating_sub(visible_width))))?;
        out.queue(style::PrintStyledContent(" │".with(Color::DarkGreen)))?;

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', "", cols).with(Color::DarkGrey),
        ))?;

        out.queue(cursor::MoveUp(1))?;
        let cursor_x = (2 + prompt_width + cursor_col).min(cols.saturating_sub(1)) as u16;
        out.queue(cursor::MoveToColumn(cursor_x))?;
        out.flush()
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
        out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        out.queue(style::PrintStyledContent(
            border_line('╭', '╮', "", cols).with(Color::DarkGrey),
        ))?;

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        out.queue(style::PrintStyledContent("│ ".with(Color::DarkGrey)))?;
        out.queue(style::PrintStyledContent(PROMPT.with(Color::DarkGrey)))?;
        out.queue(style::PrintStyledContent(visible.with(Color::DarkGrey)))?;
        out.queue(style::Print(" ".repeat(input_width.saturating_sub(visible_width))))?;
        out.queue(style::PrintStyledContent(" │".with(Color::DarkGrey)))?;

        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        out.queue(style::PrintStyledContent(
            border_line('╰', '╯', "", cols).with(Color::DarkGrey),
        ))?;

        out.flush()
    }

    fn move_below_box(&self) -> io::Result<()> {
        let mut out = io::stderr();
        out.queue(cursor::MoveDown(1))?;
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(style::Print("\n"))?;
        out.flush()
    }

    fn erase(&self) -> io::Result<()> {
        let mut out = io::stderr();
        out.queue(cursor::RestorePosition)?;
        for _ in 0..BOX_ROWS {
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