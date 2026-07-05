use crossterm::event::{KeyCode, KeyEvent};

pub struct LineInput {
    value: String,
    masked: bool,
}

impl LineInput {
    pub fn new(masked: bool) -> Self {
        Self {
            value: String::new(),
            masked,
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(ch) => {
                self.value.push(ch);
                true
            }
            KeyCode::Backspace => {
                self.value.pop();
                true
            }
            _ => false,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn display(&self) -> String {
        if self.masked {
            "*".repeat(self.value.chars().count())
        } else {
            self.value.clone()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_appends_and_backspace_removes() {
        let mut input = LineInput::new(false);
        for ch in "https://x".chars() {
            assert!(input.on_key(&press(KeyCode::Char(ch))));
        }
        assert_eq!(input.value(), "https://x");
        assert!(input.on_key(&press(KeyCode::Backspace)));
        assert_eq!(input.value(), "https://");
        assert_eq!(input.display(), "https://");
    }

    #[test]
    fn masked_display_hides_value() {
        let mut input = LineInput::new(true);
        for ch in "tok".chars() {
            input.on_key(&press(KeyCode::Char(ch)));
        }
        assert_eq!(input.value(), "tok");
        assert_eq!(input.display(), "***");
    }

    #[test]
    fn unrelated_keys_are_not_consumed() {
        let mut input = LineInput::new(false);
        assert!(!input.on_key(&press(KeyCode::Enter)));
        assert!(!input.on_key(&press(KeyCode::Esc)));
        assert!(!input.on_key(&press(KeyCode::Up)));
        assert!(input.is_empty());
    }

    #[test]
    fn backspace_on_multibyte_removes_one_char() {
        let mut input = LineInput::new(false);
        input.on_key(&press(KeyCode::Char('é')));
        input.on_key(&press(KeyCode::Char('x')));
        input.on_key(&press(KeyCode::Backspace));
        assert_eq!(input.value(), "é");
    }
}
