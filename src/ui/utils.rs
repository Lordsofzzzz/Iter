/// Word-wraps text to the given width, preserving explicit newlines.
pub fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in raw_line.split_whitespace() {
            let wl = word.chars().count();
            if wl > width {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() >= width {
                        out.push(std::mem::take(&mut chunk));
                    }
                    chunk.push(ch);
                }
                cur = chunk;
            } else if cur.is_empty() {
                cur.push_str(word);
            } else if cur.chars().count() + 1 + wl <= width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                out.push(std::mem::take(&mut cur));
                cur.push_str(word);
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
