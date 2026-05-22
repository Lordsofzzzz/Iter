use rig_core::message::Message;

pub struct Context {
    pub messages: Vec<Message>,
    pub context_window: u32,
}

impl Context {
    pub fn new(context_window: u32) -> Self {
        Self { messages: Vec::new(), context_window }
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }
}
