use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize as _;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::config::JsonMap;

use super::app::Effect;

#[derive(Debug)]
pub enum Dialog {
    Start(StartDialog),
    Confirm(ConfirmDialog),
}

#[derive(Debug)]
pub struct StartDialog {
    pub server: Option<String>,
    pub entries: Vec<(String, JsonMap)>,
    pub selected: usize,
}

impl StartDialog {
    pub fn new(server: Option<String>, presets: &BTreeMap<String, JsonMap>) -> Self {
        let mut entries = vec![("hub defaults".to_string(), JsonMap::new())];
        entries.extend(presets.iter().map(|(k, v)| (k.clone(), v.clone())));
        Self {
            server,
            entries,
            selected: 0,
        }
    }
}

#[derive(Debug)]
pub struct ConfirmDialog {
    pub message: String,
    pub effect: Effect,
}

#[derive(Debug)]
pub enum Outcome {
    Stay,
    Close,
    Commit(Effect),
}

pub fn handle_key(dialog: &mut Dialog, key: &KeyEvent) -> Outcome {
    match dialog {
        Dialog::Start(start) => match key.code {
            KeyCode::Up => {
                start.selected = start.selected.saturating_sub(1);
                Outcome::Stay
            }
            KeyCode::Down => {
                start.selected = (start.selected + 1).min(start.entries.len() - 1);
                Outcome::Stay
            }
            KeyCode::Enter => {
                let (_, options) = start.entries[start.selected].clone();
                Outcome::Commit(Effect::Start {
                    server: start.server.clone(),
                    options,
                })
            }
            KeyCode::Esc => Outcome::Close,
            _ => Outcome::Stay,
        },
        Dialog::Confirm(confirm) => match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                // Effect is moved out by replacing with a no-op quit that is
                // never observed: Commit closes the dialog immediately.
                let effect = std::mem::replace(&mut confirm.effect, Effect::Quit);
                Outcome::Commit(effect)
            }
            KeyCode::Esc | KeyCode::Char('n') => Outcome::Close,
            _ => Outcome::Stay,
        },
    }
}

pub fn render_dialog(frame: &mut Frame, dialog: &Dialog) {
    let area = centered(frame.area(), 50, 40);
    frame.render_widget(Clear, area);
    match dialog {
        Dialog::Start(start) => {
            let title = match &start.server {
                Some(server) => format!("Start {server}"),
                None => "Start the default server".to_string(),
            };
            let items: Vec<ListItem> = start
                .entries
                .iter()
                .enumerate()
                .map(|(index, (name, options))| {
                    let detail = if options.is_empty() {
                        String::new()
                    } else {
                        let rendered: Vec<String> =
                            options.iter().map(|(k, v)| format!("{k}={v}")).collect();
                        format!("  {}", rendered.join(" "))
                    };
                    let line = Line::from(format!("{name}{detail}"));
                    if index == start.selected {
                        ListItem::new(line.reversed())
                    } else {
                        ListItem::new(line)
                    }
                })
                .collect();
            let block = Block::new()
                .borders(Borders::ALL)
                .title(title)
                .title_bottom("Enter start  Esc cancel");
            frame.render_widget(List::new(items).block(block), area);
        }
        Dialog::Confirm(confirm) => {
            let block = Block::new()
                .borders(Borders::ALL)
                .title("Confirm")
                .title_bottom("Enter/y confirm  Esc/n cancel");
            frame.render_widget(Paragraph::new(confirm.message.clone()).block(block), area);
        }
    }
}

fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let width = area.width * percent_x / 100;
    let height = (area.height * percent_y / 100).max(4);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn presets() -> std::collections::BTreeMap<String, crate::config::JsonMap> {
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            "a100".to_string(),
            serde_json::from_str(r#"{"resource": "2_a100"}"#).unwrap(),
        );
        map
    }

    #[test]
    fn start_dialog_lists_hub_defaults_first() {
        let dialog = StartDialog::new(None, &presets());
        assert_eq!(dialog.entries[0].0, "hub defaults");
        assert!(dialog.entries[0].1.is_empty());
        assert_eq!(dialog.entries[1].0, "a100");
    }

    #[test]
    fn start_dialog_commits_selected_preset() {
        let mut dialog = Dialog::Start(StartDialog::new(None, &presets()));
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Down)),
            Outcome::Stay
        ));
        match handle_key(&mut dialog, &press(KeyCode::Enter)) {
            Outcome::Commit(Effect::Start {
                server: None,
                options,
            }) => {
                assert_eq!(options["resource"], serde_json::json!("2_a100"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn esc_closes_and_selection_clamps() {
        let mut dialog = Dialog::Start(StartDialog::new(None, &presets()));
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Up)),
            Outcome::Stay
        ));
        if let Dialog::Start(start) = &dialog {
            assert_eq!(start.selected, 0);
        }
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Esc)),
            Outcome::Close
        ));
    }

    #[test]
    fn confirm_commits_on_enter_or_y_and_closes_on_esc() {
        let make = || {
            Dialog::Confirm(ConfirmDialog {
                message: "Stop default?".to_string(),
                effect: Effect::Stop { server: None },
            })
        };
        let mut dialog = make();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Enter)),
            Outcome::Commit(Effect::Stop { server: None })
        ));
        let mut dialog = make();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Char('y'))),
            Outcome::Commit(Effect::Stop { server: None })
        ));
        let mut dialog = make();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Esc)),
            Outcome::Close
        ));
    }

    #[test]
    fn confirm_footer_documents_the_y_and_n_synonyms() {
        let dialog = Dialog::Confirm(ConfirmDialog {
            message: "Stop default?".to_string(),
            effect: Effect::Stop { server: None },
        });
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_dialog(frame, &dialog))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        assert!(text.contains("Enter/y confirm"), "buffer was:\n{text}");
        assert!(text.contains("Esc/n cancel"), "buffer was:\n{text}");
    }
}
