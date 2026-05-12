//! Syntax highlighting for fenced code blocks using syntect.
//!
//! Exposes `highlight_code(lang, code)` which returns an ANSI-escaped string.
//! Falls back to plain text if the language is unknown or highlighting fails.

use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::{as_24_bit_terminal_escaped, LinesWithEndings};

use std::sync::OnceLock;

struct Highlighter {
    ss: SyntaxSet,
    ts: ThemeSet,
}

static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();

fn get_highlighter() -> &'static Highlighter {
    HIGHLIGHTER.get_or_init(|| Highlighter {
        ss: SyntaxSet::load_defaults_newlines(),
        ts: ThemeSet::load_defaults(),
    })
}

/// Highlight `code` for the given `lang` identifier (e.g. `"rust"`, `"python"`).
///
/// Returns ANSI-escaped lines joined by `\n`. Falls back to plain `code` if
/// the syntax is not found or highlighting fails.
pub fn highlight_code(lang: &str, code: &str) -> String {
    let h = get_highlighter();

    // Find syntax by token name or file extension.
    let syntax = h
        .ss
        .find_syntax_by_token(lang)
        .or_else(|| h.ss.find_syntax_by_extension(lang))
        .unwrap_or_else(|| h.ss.find_syntax_plain_text());

    // Use "base16-ocean.dark" — ships with syntect, looks good on dark terminals.
    // Fall back to the first available theme if somehow missing.
    let theme = h
        .ts
        .themes
        .get("base16-ocean.dark")
        .or_else(|| h.ts.themes.values().next())
        .unwrap();

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut out = String::new();

    for line in LinesWithEndings::from(code) {
        match highlighter.highlight_line(line, &h.ss) {
            Ok(ranges) => {
                out.push_str(&as_24_bit_terminal_escaped(&ranges[..], false));
            }
            Err(_) => {
                out.push_str(line);
            }
        }
    }

    // Reset ANSI at end so subsequent text is unaffected.
    if !out.is_empty() {
        out.push_str("\x1b[0m");
    }

    out
}