use termimad::MadSkin;

fn main() {
    let skin = MadSkin::default();
    let text1 = "Line 1";
    let text2 = "Line 1\n";
    let text3 = "Line 1\nLine 2";
    
    println!("text1: {} lines", skin.text(text1, Some(80)).lines.len());
    println!("text2: {} lines", skin.text(text2, Some(80)).lines.len());
    println!("text3: {} lines", skin.text(text3, Some(80)).lines.len());
    
    let fmt1 = format!("{}", skin.text(text1, Some(80)));
    let fmt2 = format!("{}", skin.text(text2, Some(80)));
    println!("fmt1 ends with newline? {}", fmt1.ends_with('\n'));
    println!("fmt2 ends with newline? {}", fmt2.ends_with('\n'));
}
