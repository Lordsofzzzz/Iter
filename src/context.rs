use rig_core::message::Message;

pub struct Context {
    pub messages: Vec<Message>,
    pub context_window: u32,
}

impl Context {
    pub fn new(context_window: u32) -> Self {
        Self { messages: Vec::new(), context_window }
    }

    pub fn add_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn compact_if_needed(&mut self, usage_pct: f32) {
        if usage_pct < 80.0 || self.messages.len() < 10 {
            return;
        }

        // Keep first message (usually sets context) and last 4 messages.
        // We drop everything in between, but we need to preserve alternating
        // user/assistant ordering required by the API. Strategy:
        //   - Keep front[0..keep_front]
        //   - Insert a summary *as a user message*
        //   - Insert a synthetic assistant ack so the next kept message
        //     (which may be user or assistant) stays in order
        //   - Keep back[tail]
        //
        // To guarantee ordering we always keep an even number of tail messages
        // so the tail starts on a user turn.
        let keep_front = 1;
        let keep_back = 4;

        if self.messages.len() <= keep_front + keep_back {
            return;
        }

        let dropped = self.messages.len() - keep_front - keep_back;

        let mut compacted: Vec<Message> = self.messages.drain(..keep_front).collect();
        let back = self.messages.split_off(self.messages.len().saturating_sub(keep_back));

        // Summary as user message, followed by an assistant acknowledgment to
        // keep the alternation intact before whatever `back` starts with.
        compacted.push(Message::user(format!(
            "[{} earlier messages omitted for context length]",
            dropped
        )));
        compacted.push(Message::assistant(
            "Understood. Continuing from the recent context."
        ));
        compacted.extend(back);
        self.messages = compacted;
    }
}
