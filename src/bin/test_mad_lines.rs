use termimad::MadSkin;
fn main() {
    let skin = MadSkin::default();
    let text = "Hello **world**";
    let fmt = skin.text(text, Some(50));
    println!("Type of fmt.lines: {:?}", fmt.lines.len());
}
