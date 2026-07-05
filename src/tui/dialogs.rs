use std::collections::BTreeMap;
use std::time::Instant;

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
    CreateNamed(CreateNamedDialog),
}

#[derive(Debug)]
pub enum CreateStep {
    Name,
    Preset,
    Starting,
}

#[derive(Debug)]
pub struct CreateNamedDialog {
    pub input: super::input::LineInput,
    pub step: CreateStep,
    pub picker: StartDialog,
    pub op: Option<u64>,
    pub error: Option<String>,
    pub flash: Option<Instant>,
}

impl CreateNamedDialog {
    pub fn new(presets: &BTreeMap<String, JsonMap>) -> Self {
        Self {
            input: super::input::LineInput::new(false),
            step: CreateStep::Name,
            picker: StartDialog::new(None, presets),
            op: None,
            error: None,
            flash: None,
        }
    }
}

#[derive(Debug)]
pub struct StartDialog {
    pub server: Option<String>,
    pub entries: Vec<(String, JsonMap)>,
    pub selected: usize,
}

fn preset_entries(presets: &BTreeMap<String, JsonMap>) -> Vec<(String, JsonMap)> {
    let mut entries = vec![("hub defaults".to_string(), JsonMap::new())];
    entries.extend(presets.iter().map(|(k, v)| (k.clone(), v.clone())));
    entries
}

impl StartDialog {
    pub fn new(server: Option<String>, presets: &BTreeMap<String, JsonMap>) -> Self {
        Self {
            server,
            entries: preset_entries(presets),
            selected: 0,
        }
    }

    pub fn reload_presets(&mut self, presets: &BTreeMap<String, JsonMap>, select: &str) {
        self.entries = preset_entries(presets);
        self.selected = self
            .entries
            .iter()
            .position(|(name, _)| name == select)
            .unwrap_or(0);
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
    Spawn(Effect),
}

enum PickerOutcome {
    Stay,
    Close,
    Commit(JsonMap),
}

fn handle_picker_key(picker: &mut StartDialog, key: &KeyEvent) -> PickerOutcome {
    match key.code {
        KeyCode::Up => {
            picker.selected = picker.selected.saturating_sub(1);
            PickerOutcome::Stay
        }
        KeyCode::Down => {
            picker.selected = (picker.selected + 1).min(picker.entries.len() - 1);
            PickerOutcome::Stay
        }
        KeyCode::Enter => PickerOutcome::Commit(picker.entries[picker.selected].1.clone()),
        KeyCode::Esc => PickerOutcome::Close,
        _ => PickerOutcome::Stay,
    }
}

fn picker_row(text: &str, selected: bool, width: usize) -> Line<'static> {
    if selected {
        Line::from(Span::styled(
            format!("{text:<width$}"),
            Style::default()
                .fg(crate::tui::theme::SELECTION_FG)
                .bg(crate::tui::theme::SELECTION_BG),
        ))
    } else {
        Line::from(text.to_string())
    }
}

fn render_picker(picker: &StartDialog, width: usize) -> Vec<Line<'static>> {
    picker
        .entries
        .iter()
        .enumerate()
        .map(|(index, (name, _))| {
            picker_row(&preset_entry_text(name), index == picker.selected, width)
        })
        .collect()
}

pub fn handle_key(dialog: &mut Dialog, key: &KeyEvent, now: Instant) -> Outcome {
    match dialog {
        Dialog::Start(start) => match handle_picker_key(start, key) {
            PickerOutcome::Stay => Outcome::Stay,
            PickerOutcome::Close => Outcome::Close,
            PickerOutcome::Commit(options) => Outcome::Commit(Effect::Start {
                op: 0,
                server: start.server.clone(),
                options,
            }),
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
        Dialog::CreateNamed(create) => match create.step {
            CreateStep::Name => match key.code {
                KeyCode::Enter => {
                    let name = create.input.value().to_string();
                    if name.trim().is_empty() || name.contains('/') || name == "default" {
                        create.flash = Some(now);
                        Outcome::Stay
                    } else {
                        create.picker.server = Some(name);
                        create.error = None;
                        create.step = CreateStep::Preset;
                        Outcome::Stay
                    }
                }
                KeyCode::Esc => Outcome::Close,
                _ => {
                    create.input.on_key(key);
                    create.error = None;
                    Outcome::Stay
                }
            },
            CreateStep::Preset => match handle_picker_key(&mut create.picker, key) {
                PickerOutcome::Stay => Outcome::Stay,
                PickerOutcome::Close => Outcome::Close,
                PickerOutcome::Commit(options) => Outcome::Spawn(Effect::Start {
                    op: 0,
                    server: Some(create.input.value().to_string()),
                    options,
                }),
            },
            CreateStep::Starting => match key.code {
                KeyCode::Esc => Outcome::Close,
                _ => Outcome::Stay,
            },
        },
    }
}

/// A preset row shows only the preset name; the options are deliberately not
/// rendered here (they overflow the dialog and the name already identifies the
/// preset). The committed server's options remain visible in the grid pane's
/// top border.
fn preset_entry_text(name: &str) -> String {
    format!(" {name}")
}

pub fn render_dialog(frame: &mut Frame, dialog: &Dialog, spinner_frame: usize) {
    let area = frame.area();
    match dialog {
        Dialog::Start(start) => {
            let dialog_width = 60u16.min(area.width);
            let width = usize::from(dialog_width.saturating_sub(2));
            let body = render_picker(start, width);
            let height = body.len() as u16 + 4;
            let rect = super::render::centered_rect(60, height, area);
            frame.render_widget(Clear, rect);
            let title = match &start.server {
                Some(server) => format!(" Start {server} "),
                None => " Start the default server ".to_string(),
            };
            let block = super::render::dialog_block(&title);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let mut content = vec![Line::from("")];
            content.extend(body);
            frame.render_widget(Paragraph::new(content), inner);
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
        Dialog::CreateNamed(create) => {
            let mut lines: Vec<Line> = Vec::new();
            match create.step {
                CreateStep::Name => {
                    lines.push(Line::from("Step 1 of 2: name the server"));
                    lines.push(Line::from(""));
                    let mut name_row = super::wizard::input_line("Name", &create.input, true);
                    if create.flash.is_some() {
                        for span in &mut name_row.spans {
                            span.style = span.style.fg(crate::tui::theme::STATUS_ERROR_BG);
                        }
                    }
                    lines.push(name_row);
                    if let Some(error) = &create.error {
                        lines.push(Line::from(""));
                        for wrapped in super::wizard::wrap_text(error, super::wizard::CONTENT_WIDTH)
                        {
                            lines.push(Line::from(wrapped));
                        }
                    }
                }
                CreateStep::Preset => {
                    lines.push(Line::from("Step 2 of 2: choose a preset"));
                    lines.push(Line::from(""));
                    lines.extend(render_picker(&create.picker, super::wizard::CONTENT_WIDTH));
                }
                CreateStep::Starting => {
                    let name = create.input.value();
                    let glyph = crate::tui::app::SPINNER_FRAMES
                        [spinner_frame % crate::tui::app::SPINNER_FRAMES.len()];
                    lines.push(Line::from(format!("starting '{name}'  {glyph}")));
                }
            }
            let hints = match create.step {
                CreateStep::Name => " Enter: continue  Esc: cancel ",
                CreateStep::Preset => " Up/Down: navigate  Enter: start  Esc: cancel ",
                CreateStep::Starting => "",
            };
            let height = lines.len() as u16 + 4;
            let rect = super::render::centered_rect(64, height, area);
            frame.render_widget(Clear, rect);
            let block = super::render::dialog_block(" Create named server ");
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let mut content = vec![Line::from("")];
            content.extend(lines);
            let padded = ratatui::layout::Rect {
                x: inner.x + 1,
                width: inner.width.saturating_sub(2),
                ..inner
            };
            frame.render_widget(Paragraph::new(content), padded);
            if !hints.is_empty() {
                super::render::render_hints_below_dialog(frame, rect, area, hints);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::Instant;

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

    fn create_dialog() -> Dialog {
        Dialog::CreateNamed(CreateNamedDialog::new(&presets()))
    }

    #[test]
    fn empty_and_slash_names_flash_and_do_not_advance() {
        let now = Instant::now();
        let mut dialog = create_dialog();
        // empty name
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Enter), now),
            Outcome::Stay
        ));
        if let Dialog::CreateNamed(d) = &dialog {
            assert!(matches!(d.step, CreateStep::Name));
            assert!(d.flash.is_some());
        }
        // type "a/b" then Enter -> still rejected (contains '/')
        for c in ['a', '/', 'b'] {
            handle_key(&mut dialog, &press(KeyCode::Char(c)), now);
        }
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Enter), now),
            Outcome::Stay
        ));
        if let Dialog::CreateNamed(d) = &dialog {
            assert!(matches!(d.step, CreateStep::Name));
        }
    }

    #[test]
    fn valid_name_advances_to_preset_then_spawns_with_name() {
        let now = Instant::now();
        let mut dialog = create_dialog();
        for c in ['g', 'p', 'u'] {
            handle_key(&mut dialog, &press(KeyCode::Char(c)), now);
        }
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Enter), now),
            Outcome::Stay
        ));
        if let Dialog::CreateNamed(d) = &dialog {
            assert!(matches!(d.step, CreateStep::Preset));
            assert_eq!(d.picker.server.as_deref(), Some("gpu"));
        }
        // Down selects the "a100" preset, Enter spawns.
        handle_key(&mut dialog, &press(KeyCode::Down), now);
        match handle_key(&mut dialog, &press(KeyCode::Enter), now) {
            Outcome::Spawn(Effect::Start {
                server, options, ..
            }) => {
                assert_eq!(server.as_deref(), Some("gpu"));
                assert_eq!(options["resource"], serde_json::json!("2_a100"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn esc_closes_from_name_and_preset() {
        let now = Instant::now();
        let mut dialog = create_dialog();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Esc), now),
            Outcome::Close
        ));
        let mut dialog = create_dialog();
        for c in ['x', 'y'] {
            handle_key(&mut dialog, &press(KeyCode::Char(c)), now);
        }
        handle_key(&mut dialog, &press(KeyCode::Enter), now); // -> Preset
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Esc), now),
            Outcome::Close
        ));
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
            handle_key(&mut dialog, &press(KeyCode::Down), Instant::now()),
            Outcome::Stay
        ));
        match handle_key(&mut dialog, &press(KeyCode::Enter), Instant::now()) {
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
            handle_key(&mut dialog, &press(KeyCode::Up), Instant::now()),
            Outcome::Stay
        ));
        if let Dialog::Start(start) = &dialog {
            assert_eq!(start.selected, 0);
        }
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Esc), Instant::now()),
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
            handle_key(&mut dialog, &press(KeyCode::Enter), Instant::now()),
            Outcome::Commit(Effect::Stop { server: None, .. })
        ));
        let mut dialog = make();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Char('y')), Instant::now()),
            Outcome::Commit(Effect::Stop { server: None, .. })
        ));
        let mut dialog = make();
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Esc), Instant::now()),
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
            .draw(|frame| render_dialog(frame, &dialog, 0))
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
            .draw(|frame| render_dialog(frame, &dialog, 0))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        assert!(text.contains("Start the default server"));
        assert!(text.contains("hub defaults"));
        assert!(text.contains("a100"));
        assert!(
            !text.contains("resource"),
            "preset options must not render in the picker; buffer was:\n{text}"
        );
        assert!(text.contains("Enter: start"));
    }

    fn render_to_text(dialog: &Dialog) -> String {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_dialog(frame, dialog, 0))
            .unwrap();
        crate::tui::render::buffer_text(&terminal)
    }

    #[test]
    fn create_named_name_step_shows_title_step_and_input() {
        let dialog = create_dialog();
        let text = render_to_text(&dialog);
        assert!(text.contains("Create named server"), "buffer was:\n{text}");
        assert!(
            text.contains("Step 1 of 2: name the server"),
            "buffer was:\n{text}"
        );
        assert!(text.contains("Name:"), "buffer was:\n{text}");
        assert!(text.contains("Enter: continue"), "buffer was:\n{text}");
    }

    #[test]
    fn create_named_preset_step_lists_presets() {
        let now = std::time::Instant::now();
        let mut dialog = create_dialog();
        for c in ['g', 'p', 'u'] {
            handle_key(&mut dialog, &press(KeyCode::Char(c)), now);
        }
        handle_key(&mut dialog, &press(KeyCode::Enter), now);
        let text = render_to_text(&dialog);
        assert!(
            text.contains("Step 2 of 2: choose a preset"),
            "buffer was:\n{text}"
        );
        assert!(text.contains("hub defaults"), "buffer was:\n{text}");
        assert!(text.contains("a100"), "buffer was:\n{text}");
        assert!(
            !text.contains("resource"),
            "preset options must not render in the picker; buffer was:\n{text}"
        );
    }

    #[test]
    fn create_named_starting_step_shows_the_name_and_spinner() {
        let now = std::time::Instant::now();
        let mut dialog = create_dialog();
        for c in ['g', 'p', 'u'] {
            handle_key(&mut dialog, &press(KeyCode::Char(c)), now);
        }
        handle_key(&mut dialog, &press(KeyCode::Enter), now); // -> Preset
        if let Dialog::CreateNamed(d) = &mut dialog {
            d.step = CreateStep::Starting;
        }
        let text = render_to_text(&dialog);
        assert!(text.contains("starting 'gpu'"), "buffer was:\n{text}");
    }

    #[test]
    fn starting_step_esc_closes_other_keys_stay() {
        let now = Instant::now();
        let mut dialog = create_dialog();
        if let Dialog::CreateNamed(d) = &mut dialog {
            d.step = CreateStep::Starting;
        }
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Char('x')), now),
            Outcome::Stay
        ));
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Esc), now),
            Outcome::Close
        ));
    }

    #[test]
    fn default_name_is_rejected() {
        let now = Instant::now();
        let mut dialog = create_dialog();
        for c in "default".chars() {
            handle_key(&mut dialog, &press(KeyCode::Char(c)), now);
        }
        assert!(matches!(
            handle_key(&mut dialog, &press(KeyCode::Enter), now),
            Outcome::Stay
        ));
        if let Dialog::CreateNamed(d) = &dialog {
            assert!(matches!(d.step, CreateStep::Name));
            assert!(d.flash.is_some());
        }
    }

    #[test]
    fn create_named_error_wraps_with_spacer_and_full_width_last_column() {
        // Two full-width (60-col) words; the second ends in a unique marker so a
        // clipped last column would drop it (guards the double-pad fix).
        let error = format!("{} {}Z", "A".repeat(60), "B".repeat(59));
        let mut dialog = create_dialog();
        if let Dialog::CreateNamed(d) = &mut dialog {
            d.error = Some(error);
        }
        let text = render_to_text(&dialog);
        let marker = format!("{}Z", "B".repeat(59));
        assert!(text.contains(&marker), "buffer was:\n{text}");
        let rows: Vec<&str> = text.lines().collect();
        let first_error = rows
            .iter()
            .position(|r| r.contains(&"A".repeat(60)))
            .expect("wrapped error line present");
        assert!(
            !rows[first_error - 1].chars().any(|c| c.is_alphanumeric()),
            "expected a blank spacer above the error, buffer was:\n{text}"
        );
    }
}
