use termimad::MadSkin;
fn main() {
    let skin = MadSkin::default();
    let text = "This is a very long text that should wrap. ".repeat(10);
    let fmt = skin.text(&text, Some(50));
    println!("lines.len(): {}", fmt.lines.len());
    let fmt_str = format!("{}", fmt);
    let newlines = fmt_str.chars().filter(|&c| c == '\n').count();
    println!("newlines in display: {}", newlines);
}
