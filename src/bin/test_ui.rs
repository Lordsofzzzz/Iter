use std::time::Duration;
use termimad::MadSkin;

fn main() {
    let mut current_text_block = String::new();
    let mut last_rendered_lines: Vec<String> = Vec::new();
    let skin = MadSkin::default();

    let chunks = vec![
        "Here is a **bold** statement.\n",
        "And here is a list:\n",
        "- Item 1\n",
        "- Item 2\n",
        "Let's see if this wraps properly across the screen... ",
        "We are adding more text to ensure it wraps around the edge of the terminal.\n",
        "```rust\n",
        "fn main() {\n",
        "    println!(\"Hello World\");\n",
        "}\n",
        "```\n",
        "Done!"
    ];

    for chunk in chunks {
        current_text_block.push_str(chunk);
        let width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80) as usize;
        let fmt_text = skin.text(&current_text_block, Some(width));
        let rendered = format!("{}", fmt_text);
        
        let new_lines: Vec<String> = if rendered.is_empty() {
            Vec::new()
        } else {
            rendered
                .strip_suffix('\n')
                .unwrap_or(&rendered)
                .split('\n')
                .map(|s| s.to_string())
                .collect()
        };

        let mut common_len = 0;
        for (old, new) in last_rendered_lines.iter().zip(new_lines.iter()) {
            if old == new {
                common_len += 1;
            } else {
                break;
            }
        }

        let go_up = last_rendered_lines.len().saturating_sub(common_len);
        if go_up > 0 {
            print!("\x1b[{}A", go_up);
        }
        
        if go_up > 0 || common_len < new_lines.len() {
            print!("\r\x1b[0J");
            for line in new_lines.iter().skip(common_len) {
                println!("{}", line);
            }
            std::io::Write::flush(&mut std::io::stdout()).ok();
        }
        last_rendered_lines = new_lines;
        
        std::thread::sleep(Duration::from_millis(500));
    }
}
