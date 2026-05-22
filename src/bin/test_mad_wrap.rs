use termimad::MadSkin;
fn main() {
    let skin = MadSkin::default();
    let text = "This is a very long text that should wrap. ".repeat(3);
    let fmt = skin.text(&text, Some(50));
    let rendered = format!("{}", fmt);
    let lines: Vec<&str> = rendered.strip_suffix('\n').unwrap_or(&rendered).split('\n').collect();
    println!("width 50, len {}: {:?}", lines.len(), lines);
}
