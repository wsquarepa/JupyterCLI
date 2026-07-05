use crossterm::event::{KeyCode, KeyEvent};

pub struct LineInput {
    value: String,
    cursor: usize,
    masked: bool,
}

impl LineInput {
    pub fn new(masked: bool) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            masked,
        }
    }

    fn byte_at(&self, char_index: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_index)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len())
    }

    fn char_count(&self) -> usize {
        self.value.chars().count()
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(ch) => {
                let at = self.byte_at(self.cursor);
                self.value.insert(at, ch);
                self.cursor += 1;
                true
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let at = self.byte_at(self.cursor - 1);
                    self.value.remove(at);
                    self.cursor -= 1;
                }
                true
            }
            KeyCode::Delete => {
                if self.cursor < self.char_count() {
                    let at = self.byte_at(self.cursor);
                    self.value.remove(at);
                }
                true
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                true
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.char_count());
                true
            }
            KeyCode::Home => {
                self.cursor = 0;
                true
            }
            KeyCode::End => {
                self.cursor = self.char_count();
                true
            }
            _ => false,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// Cursor position as a char index, valid for indexing `display()`.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn display(&self) -> String {
        if self.masked {
            "*".repeat(self.char_count())
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

    fn type_text(input: &mut LineInput, text: &str) {
        for ch in text.chars() {
            assert!(input.on_key(&press(KeyCode::Char(ch))));
        }
    }

    #[test]
    fn typing_appends_and_backspace_removes() {
        let mut input = LineInput::new(false);
        type_text(&mut input, "https://x");
        assert_eq!(input.value(), "https://x");
        assert_eq!(input.cursor(), 9);
        assert!(input.on_key(&press(KeyCode::Backspace)));
        assert_eq!(input.value(), "https://");
        assert_eq!(input.display(), "https://");
    }

    #[test]
    fn mid_string_editing_follows_the_cursor() {
        let mut input = LineInput::new(false);
        type_text(&mut input, "hllo");
        input.on_key(&press(KeyCode::Home));
        input.on_key(&press(KeyCode::Right));
        input.on_key(&press(KeyCode::Char('e')));
        assert_eq!(input.value(), "hello");
        assert_eq!(input.cursor(), 2);
        input.on_key(&press(KeyCode::Delete));
        assert_eq!(input.value(), "helo");
        input.on_key(&press(KeyCode::End));
        assert_eq!(input.cursor(), 4);
        input.on_key(&press(KeyCode::Backspace));
        assert_eq!(input.value(), "hel");
    }

    #[test]
    fn cursor_clamps_at_both_ends() {
        let mut input = LineInput::new(false);
        input.on_key(&press(KeyCode::Left));
        assert_eq!(input.cursor(), 0);
        type_text(&mut input, "ab");
        input.on_key(&press(KeyCode::Right));
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn masked_display_hides_value() {
        let mut input = LineInput::new(true);
        type_text(&mut input, "tok");
        assert_eq!(input.value(), "tok");
        assert_eq!(input.display(), "***");
    }

    #[test]
    fn multibyte_editing_stays_on_char_boundaries() {
        let mut input = LineInput::new(false);
        type_text(&mut input, "éxé");
        input.on_key(&press(KeyCode::Home));
        input.on_key(&press(KeyCode::Delete));
        assert_eq!(input.value(), "xé");
        input.on_key(&press(KeyCode::End));
        input.on_key(&press(KeyCode::Backspace));
        assert_eq!(input.value(), "x");
    }

    #[test]
    fn unrelated_keys_are_not_consumed() {
        let mut input = LineInput::new(false);
        assert!(!input.on_key(&press(KeyCode::Enter)));
        assert!(!input.on_key(&press(KeyCode::Esc)));
        assert!(input.is_empty());
    }
}
