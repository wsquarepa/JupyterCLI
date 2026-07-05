use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

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
                    op: 0,
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
    let area = frame.area();
    match dialog {
        Dialog::Start(start) => {
            let height = (start.entries.len() as u16).saturating_add(4);
            let rect = super::render::centered_rect(60, height, area);
            frame.render_widget(Clear, rect);
            let title = match &start.server {
                Some(server) => format!(" Start {server} "),
                None => " Start the default server ".to_string(),
            };
            let width = usize::from(rect.width.saturating_sub(2));
            let mut lines: Vec<Line> = vec![Line::from("")];
            lines.extend(
                start
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
                        let text = format!(" {name}{detail}");
                        if index == start.selected {
                            Line::from(Span::styled(
                                format!("{text:<width$}"),
                                Style::default()
                                    .fg(crate::tui::theme::SELECTION_FG)
                                    .bg(crate::tui::theme::SELECTION_BG),
                            ))
                        } else {
                            Line::from(text)
                        }
                    }),
            );
            let block = super::render::dialog_block(&title);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            frame.render_widget(Paragraph::new(lines), inner);
            super::render::render_hints_below_dialog(
                frame,
                rect,
                area,
                " Up/Down: navigate  Enter: start  Esc: cancel ",
            );
        }
        Dialog::Confirm(confirm) => {
            let rect = super::render::centered_rect(60, 5, area);
            frame.render_widget(Clear, rect);
            let block = super::render::dialog_block(" Confirm ");
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(confirm.message.clone()).centered(),
                ]),
                inner,
            );
            super::render::render_hints_below_dialog(
                frame,
                rect,
                area,
                " Enter/y: confirm  Esc/n: cancel ",
            );
        }
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
                ..
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
                effect: Effect::Stop {
                    op: 0,
                    server: None,
                },
            })
        };
        let mut dialog = make();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Enter)),
            Outcome::Commit(Effect::Stop { server: None, .. })
        ));
        let mut dialog = make();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Char('y'))),
            Outcome::Commit(Effect::Stop { server: None, .. })
        ));
        let mut dialog = make();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Esc)),
            Outcome::Close
        ));
    }

    #[test]
    fn confirm_hints_render_below_the_dialog() {
        let dialog = Dialog::Confirm(ConfirmDialog {
            message: "Stop default?".to_string(),
            effect: Effect::Stop {
                op: 0,
                server: None,
            },
        });
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_dialog(frame, &dialog))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        assert!(text.contains("Enter/y: confirm"), "buffer was:\n{text}");
        assert!(text.contains("Esc/n: cancel"), "buffer was:\n{text}");
        assert!(text.contains(" Confirm "));
    }

    #[test]
    fn start_dialog_lists_presets_with_hints() {
        let dialog = Dialog::Start(StartDialog::new(None, &presets()));
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_dialog(frame, &dialog))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        assert!(text.contains("Start the default server"));
        assert!(text.contains("hub defaults"));
        assert!(text.contains("a100"));
        assert!(text.contains("Enter: start"));
    }
}
