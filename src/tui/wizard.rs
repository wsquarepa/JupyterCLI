use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::api::HubClient;
use crate::cli::CliError;
use crate::config::{self, Config, HubConfig, JsonMap};

use super::input::LineInput;

type Identity = (String, Option<JsonMap>);
type IdentityHandle = tokio::task::JoinHandle<Result<Identity, CliError>>;

#[derive(Debug)]
pub enum Step {
    Welcome,
    Url,
    Token,
    Testing,
    PresetOffer,
    Failed,
}

#[derive(Debug)]
pub enum WizardAction {
    None,
    TestConnection,
    SavePreset,
    SkipPreset,
    Abort,
}

pub struct WizardState {
    pub step: Step,
    pub url: LineInput,
    pub token: LineInput,
    pub error: Option<String>,
    pub username: Option<String>,
    pub found_options: Option<JsonMap>,
}

impl Default for WizardState {
    fn default() -> Self {
        Self::new()
    }
}

impl WizardState {
    pub fn new() -> Self {
        Self {
            step: Step::Welcome,
            url: LineInput::new(false),
            token: LineInput::new(true),
            error: None,
            username: None,
            found_options: None,
        }
    }

    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.step = Step::Failed;
    }

    pub fn offer_preset(&mut self, username: String, options: Option<JsonMap>) {
        self.username = Some(username);
        self.found_options = options;
        self.step = Step::PresetOffer;
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> WizardAction {
        if key.kind != KeyEventKind::Press {
            return WizardAction::None;
        }
        match self.step {
            Step::Welcome => match key.code {
                KeyCode::Esc => WizardAction::Abort,
                KeyCode::Enter => {
                    self.step = Step::Url;
                    WizardAction::None
                }
                _ => WizardAction::None,
            },
            Step::Url => match key.code {
                KeyCode::Esc => WizardAction::Abort,
                KeyCode::Enter if !self.url.is_empty() => {
                    self.step = Step::Token;
                    WizardAction::None
                }
                _ => {
                    self.url.on_key(key);
                    WizardAction::None
                }
            },
            Step::Token => match key.code {
                KeyCode::Esc => WizardAction::Abort,
                KeyCode::Enter if !self.token.is_empty() => {
                    self.step = Step::Testing;
                    WizardAction::TestConnection
                }
                _ => {
                    self.token.on_key(key);
                    WizardAction::None
                }
            },
            Step::Testing => WizardAction::None,
            Step::Failed => match key.code {
                KeyCode::Esc => WizardAction::Abort,
                _ => {
                    self.step = Step::Token;
                    WizardAction::None
                }
            },
            Step::PresetOffer => match (key.code, self.found_options.is_some()) {
                (KeyCode::Esc, _) => WizardAction::Abort,
                (KeyCode::Char('y'), true) | (KeyCode::Enter, true) => WizardAction::SavePreset,
                (KeyCode::Char('n'), true) => WizardAction::SkipPreset,
                (KeyCode::Enter, false) => WizardAction::SkipPreset,
                _ => WizardAction::None,
            },
        }
    }
}

/// Text columns available for wizard dialog content: the dialog is 64 wide,
/// less the two block borders and the two columns of interior padding.
const CONTENT_WIDTH: usize = 60;

/// Greedy word wrap to `width` columns; a single word longer than the width
/// breaks mid-word (URLs and error payloads have no convenient spaces).
///
/// Width 0 disables wrapping and returns the text as one line. Non-empty input
/// never yields an empty vec; empty input yields a single empty line so blank
/// spacer rows survive wrapping.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current: Vec<char> = Vec::new();
    for word in text.split(' ') {
        let word_len = word.chars().count();
        if word_len > width {
            if !current.is_empty() {
                lines.push(current.iter().collect());
                current.clear();
            }
            let mut chunk: Vec<char> = Vec::new();
            for ch in word.chars() {
                chunk.push(ch);
                if chunk.len() == width {
                    lines.push(chunk.iter().collect());
                    chunk.clear();
                }
            }
            current = chunk;
            continue;
        }
        let separator = usize::from(!current.is_empty());
        if current.len() + separator + word_len > width {
            lines.push(current.iter().collect());
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.extend(word.chars());
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current.iter().collect());
    }
    lines
}

/// Visible window of a (possibly masked) input value: at most `avail` cells
/// with the cursor cell always inside. Returns (before, at, after) where
/// `at` is the single cursor cell (a space when the cursor sits past the end).
fn input_window(display: &str, cursor: usize, avail: usize) -> (String, String, String) {
    let chars: Vec<char> = display.chars().collect();
    let cursor = cursor.min(chars.len());
    if avail == 0 {
        return (String::new(), String::new(), String::new());
    }
    let scroll = cursor.saturating_sub(avail.saturating_sub(1));
    let window_end = (scroll + avail).min(chars.len());
    let before: String = chars[scroll..cursor].iter().collect();
    let at: String = chars
        .get(cursor)
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".to_string());
    let after_start = (cursor + 1).min(window_end);
    let after: String = chars[after_start..window_end].iter().collect();
    (before, at, after)
}

fn input_line(label: &str, input: &LineInput, active: bool) -> ratatui::text::Line<'static> {
    use ratatui::style::{Modifier, Style};
    use ratatui::text::Span;
    let style = if active {
        Style::default()
            .fg(crate::tui::theme::BORDER_FOCUSED)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let display = input.display();
    if !active {
        return ratatui::text::Line::from(vec![
            Span::styled(format!("  {label}: "), style),
            Span::styled(display, style),
        ]);
    }
    let prefix = format!("> {label}: ");
    let avail = CONTENT_WIDTH.saturating_sub(prefix.chars().count());
    let cursor = input.cursor().min(display.chars().count());
    let (before, at, after) = input_window(&display, cursor, avail);
    ratatui::text::Line::from(vec![
        Span::styled(prefix, style),
        Span::styled(before, style),
        Span::styled(at, style.add_modifier(Modifier::REVERSED)),
        Span::styled(after, style),
    ])
}

/// A wizard content row: `Text` is plain prose to word-wrap; `Indented` wraps
/// two columns narrower and prefixes each resulting line with two spaces;
/// `Row` is an input or spinner row rendered verbatim (its cursor windowing is
/// already sized).
enum Body {
    Text(String),
    Indented(String),
    Row(Line<'static>),
}

pub fn render(
    frame: &mut Frame,
    state: &WizardState,
    backdrop: &crate::tui::app::App,
    spinner_frame: usize,
) {
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use ratatui::text::Span;
    use ratatui::widgets::Clear;

    super::render::draw(frame, backdrop);

    let url = state.url.value().trim_end_matches('/').to_string();
    let (body, hints): (Vec<Body>, &str) = match &state.step {
        Step::Welcome => (
            vec![
                Body::Text("Welcome to JupyterCLI.".to_string()),
                Body::Text("This wizard connects you to a JupyterHub in three steps.".to_string()),
            ],
            " Enter: continue  Esc: quit ",
        ),
        Step::Url => (
            vec![
                Body::Text("Step 1 of 3: hub base URL".to_string()),
                Body::Text("Example: https://jupyter.example.edu".to_string()),
                Body::Text(String::new()),
                Body::Row(input_line("URL", &state.url, true)),
            ],
            " Enter: continue  Esc: quit ",
        ),
        Step::Token => (
            vec![
                Body::Text("Step 2 of 3: API token".to_string()),
                Body::Text(format!("Create one in the browser at {url}/hub/token")),
                Body::Text(String::new()),
                Body::Row(input_line("Token", &state.token, true)),
            ],
            " Enter: test the connection  Esc: quit ",
        ),
        Step::Testing => {
            let glyph = crate::tui::app::SPINNER_FRAMES
                [spinner_frame % crate::tui::app::SPINNER_FRAMES.len()];
            (
                vec![Body::Row(Line::from(Span::styled(
                    format!("Step 3 of 3: {glyph} testing the connection..."),
                    Style::default().fg(crate::tui::theme::SPINNER),
                )))],
                "",
            )
        }
        Step::Failed => (
            vec![
                Body::Text("The connection test failed:".to_string()),
                Body::Text(String::new()),
                Body::Text(state.error.clone().unwrap_or_default()),
            ],
            " any key: re-enter the token  Esc: quit ",
        ),
        Step::PresetOffer => {
            let username = state.username.clone().unwrap_or_default();
            match &state.found_options {
                Some(options) => {
                    let rendered: Vec<String> =
                        options.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    (
                        vec![
                            Body::Text(format!("Connected as {username}. Configuration saved.")),
                            Body::Text(String::new()),
                            Body::Text(
                                "A running server was found with these options:".to_string(),
                            ),
                            // Wrapped un-indented, then indented per line: wrap_text
                            // drops leading spaces, so a pre-indented string would
                            // render flush-left.
                            Body::Indented(rendered.join(" ")),
                            Body::Text(
                                "Save them as preset 'imported' for one-key starts?".to_string(),
                            ),
                        ],
                        " y: save  n: skip  Esc: quit ",
                    )
                }
                None => (
                    vec![
                        Body::Text(format!("Connected as {username}. Configuration saved.")),
                        Body::Text(String::new()),
                        Body::Text(
                            "JupyterCLI cannot list your hub's environment and resource"
                                .to_string(),
                        ),
                        Body::Text(
                            "options because JupyterHub does not expose them over its API."
                                .to_string(),
                        ),
                        Body::Text(format!(
                            "Start a server once in the browser at {url}/hub/spawn,"
                        )),
                        Body::Text("then run: jhc preset import".to_string()),
                    ],
                    " Enter: open the dashboard ",
                ),
            }
        }
    };

    // Wrap plain text to the content width before deriving the dialog height so
    // multi-row output grows the dialog instead of clipping at the border.
    let lines: Vec<Line> = body
        .into_iter()
        .flat_map(|item| match item {
            Body::Text(text) => wrap_text(&text, CONTENT_WIDTH)
                .into_iter()
                .map(Line::from)
                .collect::<Vec<Line>>(),
            Body::Indented(text) => wrap_text(&text, CONTENT_WIDTH.saturating_sub(2))
                .into_iter()
                .map(|line| Line::from(format!("  {line}")))
                .collect::<Vec<Line>>(),
            Body::Row(line) => vec![line],
        })
        .collect();

    let area = frame.area();
    let height = lines.len() as u16 + 4;
    let rect = super::render::centered_rect(64, height, area);
    frame.render_widget(Clear, rect);
    let block = super::render::dialog_block(" JupyterCLI setup ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut content = vec![Line::from("")];
    content.extend(lines);
    let padded = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    frame.render_widget(Paragraph::new(content), padded);
    if !hints.is_empty() {
        super::render::render_hints_below_dialog(frame, rect, area, hints);
    }
}

pub async fn run(terminal: &mut ratatui::DefaultTerminal) -> Result<Option<Config>, CliError> {
    use futures_util::StreamExt as _;

    // The unconfigured dashboard rendered behind the setup dialog. Drain the
    // constructor's refresh effect and op so no request fires and no spinner
    // shows: there is no hub to talk to yet.
    let size = crossterm::terminal::size().unwrap_or((80, 24));
    let mut backdrop = crate::tui::app::App::new(
        "not configured".to_string(),
        Default::default(),
        crate::shellops::TERMINAL_LIMIT,
        size,
    );
    let _ = backdrop.take_effects();
    backdrop.ops.clear();

    let mut events = crossterm::event::EventStream::new();
    let mut state = WizardState::new();
    let mut saved: Option<Config> = None;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    let mut spinner_frame = 0usize;
    let mut pending: Option<IdentityHandle> = None;

    loop {
        terminal
            .draw(|frame| render(frame, &state, &backdrop, spinner_frame))
            .map_err(CliError::Io)?;

        tokio::select! {
            event = events.next() => {
                let event = match event {
                    None => return Ok(None),
                    Some(event) => event.map_err(CliError::Io)?,
                };
                let key = match event {
                    crossterm::event::Event::Key(key) => key,
                    crossterm::event::Event::Resize(cols, rows) => {
                        backdrop.set_size(cols, rows);
                        continue;
                    }
                    _ => continue,
                };
                match state.on_key(&key) {
                    WizardAction::None => {}
                    WizardAction::Abort => return Ok(None),
                    WizardAction::TestConnection => {
                        let url = state.url.value().to_string();
                        let token = state.token.value().to_string();
                        pending = Some(tokio::spawn(async move {
                            fetch_identity(&url, &token).await
                        }));
                    }
                    WizardAction::SavePreset => {
                        let mut config = saved.take().expect("preset offer only follows a save");
                        if let Some(options) = state.found_options.clone() {
                            let hub = config
                                .hubs
                                .get_mut("default")
                                .expect("wizard saved the hub under the name 'default'");
                            hub.presets.insert("imported".to_string(), options);
                            config::save(&config)?;
                        }
                        return Ok(Some(config));
                    }
                    WizardAction::SkipPreset => {
                        return Ok(Some(
                            saved.take().expect("preset offer only follows a save"),
                        ));
                    }
                }
            }
            _ = tick.tick() => {
                spinner_frame = spinner_frame.wrapping_add(1);
            }
            result = async { pending.as_mut().expect("guarded by pending.is_some()").await }, if pending.is_some() => {
                pending = None;
                let outcome = result.map_err(|e| CliError::Io(std::io::Error::other(e)))?;
                match outcome {
                    Ok((username, options)) => {
                        let config = build_config(&state);
                        config::save(&config)?;
                        saved = Some(config);
                        state.offer_preset(username, options);
                    }
                    Err(e) => state.fail(e.to_string()),
                }
            }
        }
    }
}

fn build_config(state: &WizardState) -> Config {
    Config {
        default_hub: "default".to_string(),
        hubs: [(
            "default".to_string(),
            HubConfig {
                url: state.url.value().to_string(),
                token: state.token.value().to_string(),
                terminal_limit: None,
                presets: Default::default(),
            },
        )]
        .into(),
    }
}

async fn fetch_identity(url: &str, token: &str) -> Result<(String, Option<JsonMap>), CliError> {
    let client = HubClient::new(url, token)?.with_retry_warnings(false);
    let user = client.whoami().await?;
    let options = user
        .servers
        .values()
        .find(|s| s.ready && !s.user_options.is_empty())
        .map(|s| s.user_options.clone());
    Ok((user.name.clone(), options))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(state: &mut WizardState, text: &str) {
        for ch in text.chars() {
            state.on_key(&press(KeyCode::Char(ch)));
        }
    }

    #[test]
    fn happy_path_reaches_testing_with_collected_values() {
        let mut state = WizardState::new();
        assert!(matches!(state.step, Step::Welcome));
        state.on_key(&press(KeyCode::Enter));
        assert!(matches!(state.step, Step::Url));
        type_text(&mut state, "https://hub.example.edu");
        state.on_key(&press(KeyCode::Enter));
        assert!(matches!(state.step, Step::Token));
        type_text(&mut state, "sekrit");
        let action = state.on_key(&press(KeyCode::Enter));
        assert!(matches!(action, WizardAction::TestConnection));
        assert!(matches!(state.step, Step::Testing));
        assert_eq!(state.url.value(), "https://hub.example.edu");
        assert_eq!(state.token.value(), "sekrit");
    }

    #[test]
    fn empty_inputs_do_not_advance() {
        let mut state = WizardState::new();
        state.on_key(&press(KeyCode::Enter)); // -> Url
        assert!(matches!(
            state.on_key(&press(KeyCode::Enter)),
            WizardAction::None
        ));
        assert!(matches!(state.step, Step::Url));
    }

    #[test]
    fn esc_aborts_and_failure_returns_to_token() {
        let mut state = WizardState::new();
        state.on_key(&press(KeyCode::Enter));
        assert!(matches!(
            state.on_key(&press(KeyCode::Esc)),
            WizardAction::Abort
        ));

        let mut state = WizardState::new();
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, "https://x");
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, "bad");
        state.on_key(&press(KeyCode::Enter));
        state.fail("token invalid or expired".to_string());
        assert!(matches!(state.step, Step::Failed));
        state.on_key(&press(KeyCode::Enter));
        assert!(matches!(state.step, Step::Token));
        assert_eq!(state.url.value(), "https://x", "url survives a failed test");
    }

    #[test]
    fn preset_offer_actions() {
        let mut state = WizardState::new();
        state.offer_preset(
            "ww41".to_string(),
            Some(serde_json::from_str(r#"{"resource": "2_a100"}"#).unwrap()),
        );
        assert!(matches!(state.step, Step::PresetOffer));
        assert!(matches!(
            state.on_key(&press(KeyCode::Char('y'))),
            WizardAction::SavePreset
        ));
        state.offer_preset("ww41".to_string(), None);
        assert!(matches!(
            state.on_key(&press(KeyCode::Enter)),
            WizardAction::SkipPreset
        ));
    }

    #[test]
    fn render_overlays_the_setup_dialog_on_the_dashboard() {
        let mut state = WizardState::new();
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, "https://x");
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, "abc");
        let mut backdrop = crate::tui::app::App::new(
            "not configured".to_string(),
            Default::default(),
            999,
            (90, 24),
        );
        let _ = backdrop.take_effects();
        backdrop.ops.clear();
        let backend = ratatui::backend::TestBackend::new(90, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &backdrop, 0))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        assert!(text.contains("JupyterCLI setup"), "buffer:\n{text}");
        assert!(text.contains(" Servers "), "backdrop must show:\n{text}");
        assert!(text.contains("hub/token"));
        assert!(text.contains("***"));
        assert!(
            !text.contains("abc"),
            "token must never render in clear text"
        );
    }

    #[test]
    fn failed_step_separates_the_error_from_the_heading() {
        let mut state = WizardState::new();
        state.fail("token invalid or expired".to_string());
        let mut backdrop = crate::tui::app::App::new(
            "not configured".to_string(),
            Default::default(),
            999,
            (90, 24),
        );
        let _ = backdrop.take_effects();
        backdrop.ops.clear();
        let backend = ratatui::backend::TestBackend::new(90, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &backdrop, 0))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        let lines: Vec<&str> = text.lines().collect();
        let heading_row = lines
            .iter()
            .position(|l| l.contains("The connection test failed:"))
            .expect("failed heading row");
        let error_row = lines
            .iter()
            .position(|l| l.contains("token invalid or expired"))
            .expect("generated error row");
        assert_eq!(
            error_row,
            heading_row + 2,
            "a blank row must separate the heading from the error:\n{text}"
        );
    }

    #[test]
    fn welcome_dialog_pads_content_away_from_its_borders() {
        let state = WizardState::new();
        let mut backdrop = crate::tui::app::App::new(
            "not configured".to_string(),
            Default::default(),
            999,
            (90, 24),
        );
        let _ = backdrop.take_effects();
        backdrop.ops.clear();
        let backend = ratatui::backend::TestBackend::new(90, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &backdrop, 0))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        assert!(
            text.contains("│ Welcome to JupyterCLI."),
            "content needs one column of padding after the left border:\n{text}"
        );
        let lines: Vec<&str> = text.lines().collect();
        let content_row = lines
            .iter()
            .position(|l| l.contains("This wizard"))
            .expect("welcome body line");
        let bottom_row = lines
            .iter()
            .position(|l| l.contains('└'))
            .expect("dialog bottom border");
        assert!(
            bottom_row > content_row + 1,
            "a blank row must sit above the bottom border:\n{text}"
        );
    }

    #[test]
    fn wrap_text_greedy_wraps_and_breaks_long_words() {
        assert_eq!(wrap_text("hello world foo", 11), vec!["hello world", "foo"]);
        assert_eq!(wrap_text("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap_text("abcdef", 3), vec!["abc", "def"]);
        assert_eq!(wrap_text("anything", 0), vec!["anything"]);
        assert_eq!(wrap_text("", 60), vec![String::new()]);
    }

    #[test]
    fn input_window_keeps_the_cursor_cell_visible() {
        let long = "0123456789";
        let (before, at, after) = input_window(long, long.chars().count(), 5);
        assert_eq!(before, "6789", "cursor at end shows the tail");
        assert_eq!(at, " ", "cursor past the end is a space cell");
        assert_eq!(after, "");

        let (before, at, after) = input_window(long, 0, 5);
        assert_eq!(before, "", "cursor at 0 shows the head");
        assert_eq!(at, "0");
        assert_eq!(after, "1234");

        let (before, at, after) = input_window("ab", 1, 10);
        assert_eq!(
            (before.as_str(), at.as_str(), after.as_str()),
            ("a", "b", "")
        );

        let (before, at, after) = input_window("áéíóú", 5, 3);
        assert_eq!(before, "óú", "multibyte window stays on char boundaries");
        assert_eq!(at, " ");
        assert_eq!(after, "");
    }

    fn token_step_with_url(url: &str) -> WizardState {
        let mut state = WizardState::new();
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, url);
        state.on_key(&press(KeyCode::Enter));
        state
    }

    #[test]
    fn token_step_wraps_a_long_browser_url_without_clipping() {
        let state = token_step_with_url("https://jupyterhub.university.example.edu");
        let mut backdrop = crate::tui::app::App::new(
            "not configured".to_string(),
            Default::default(),
            999,
            (90, 24),
        );
        let _ = backdrop.take_effects();
        backdrop.ops.clear();
        let backend = ratatui::backend::TestBackend::new(90, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &backdrop, 0))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        let lines: Vec<&str> = text.lines().collect();
        let intro_row = lines
            .iter()
            .position(|l| l.contains("Create one in the browser at"))
            .expect("browser instruction row");
        let url_row = lines
            .iter()
            .position(|l| l.contains("https://jupyterhub.university.example.edu/hub/token"))
            .expect("wrapped url tail row must not be clipped");
        assert_ne!(
            intro_row, url_row,
            "the browser line must wrap across two rows"
        );
    }

    #[test]
    fn token_step_windows_a_long_masked_token_inside_the_dialog() {
        let mut state = token_step_with_url("https://x");
        type_text(&mut state, &"a".repeat(120));
        let mut backdrop = crate::tui::app::App::new(
            "not configured".to_string(),
            Default::default(),
            999,
            (90, 24),
        );
        let _ = backdrop.take_effects();
        backdrop.ops.clear();
        let backend = ratatui::backend::TestBackend::new(90, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &backdrop, 0))
            .unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        let token_row = text
            .lines()
            .find(|l| l.contains('*'))
            .expect("masked token row");
        assert!(
            token_row.matches('*').count() >= 40,
            "windowed masked value must render:\n{token_row}"
        );
        let last_star = token_row.rfind('*').unwrap();
        assert!(
            token_row[last_star..].contains('│'),
            "the token row must end inside the dialog:\n{token_row}"
        );
    }

    #[tokio::test]
    async fn fetch_identity_returns_username_and_running_options() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "ww41",
                "servers": {"": {"name": "", "ready": true, "url": "/user/ww41/",
                                  "user_options": {"resource": "2_a100"}}}
            })))
            .mount(&server)
            .await;

        let mut state = WizardState::new();
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, &server.uri());
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, "tok");

        let (username, options) = fetch_identity(&server.uri(), "tok").await.unwrap();
        assert_eq!(username, "ww41");
        assert_eq!(options.unwrap()["resource"], serde_json::json!("2_a100"));

        let config = build_config(&state);
        assert_eq!(config.default_hub, "default");
        let hub = &config.hubs["default"];
        assert_eq!(hub.url, server.uri());
        assert_eq!(hub.token, "tok");
        assert!(hub.presets.is_empty());
    }

    #[tokio::test]
    async fn fetch_identity_surfaces_auth_failure() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hub/api/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = fetch_identity(&server.uri(), "bad").await.unwrap_err();
        assert!(err.to_string().contains("token invalid or expired"));
    }
}
