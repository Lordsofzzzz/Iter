use termimad::MadSkin;
fn main() {
    let skin = MadSkin::default();
    let text1 = "Hello **world**";
    let text2 = "Hello **world**\nThis is new.";
    let fmt1 = skin.text(text1, Some(50));
    let fmt2 = skin.text(text2, Some(50));
    
    let rendered1 = format!("{}", fmt1);
    let rendered2 = format!("{}", fmt2);
    
    // Split by newline, but note that Display adds a trailing newline, so we strip it first.
    let lines1: Vec<&str> = rendered1.strip_suffix('\n').unwrap_or(&rendered1).split('\n').collect();
    let lines2: Vec<&str> = rendered2.strip_suffix('\n').unwrap_or(&rendered2).split('\n').collect();
    
    println!("lines1 (len {}): {:?}", lines1.len(), lines1);
    println!("lines2 (len {}): {:?}", lines2.len(), lines2);
}
