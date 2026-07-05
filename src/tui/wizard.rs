use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::api::HubClient;
use crate::cli::CliError;
use crate::config::{self, Config, HubConfig, JsonMap};

use super::input::LineInput;

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

pub fn render(frame: &mut Frame, state: &WizardState) {
    let block = Block::new().borders(Borders::ALL).title("JupyterCLI setup");
    let url = state.url.value().trim_end_matches('/').to_string();
    let lines: Vec<Line> = match &state.step {
        Step::Welcome => vec![
            Line::from("Welcome to JupyterCLI."),
            Line::from("This wizard connects you to a JupyterHub in three steps."),
            Line::from(""),
            Line::from("Enter continue  Esc quit"),
        ],
        Step::Url => vec![
            Line::from("Step 1 of 3: hub base URL"),
            Line::from("Example: https://jupyter.example.edu"),
            Line::from(""),
            Line::from(format!("> {}", state.url.display())),
            Line::from(""),
            Line::from("Enter continue  Esc quit"),
        ],
        Step::Token => vec![
            Line::from("Step 2 of 3: API token"),
            Line::from(format!("Create one in the browser at {url}/hub/token")),
            Line::from(""),
            Line::from(format!("> {}", state.token.display())),
            Line::from(""),
            Line::from("Enter test the connection  Esc quit"),
        ],
        Step::Testing => vec![Line::from("Step 3 of 3: testing the connection...")],
        Step::Failed => vec![
            Line::from("The connection test failed:"),
            Line::from(state.error.clone().unwrap_or_default()),
            Line::from(""),
            Line::from("Press any key to re-enter the token  Esc quit"),
        ],
        Step::PresetOffer => {
            let username = state.username.clone().unwrap_or_default();
            match &state.found_options {
                Some(options) => {
                    let rendered: Vec<String> =
                        options.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    vec![
                        Line::from(format!("Connected as {username}. Configuration saved.")),
                        Line::from(""),
                        Line::from("A running server was found with these options:"),
                        Line::from(format!("  {}", rendered.join(" "))),
                        Line::from("Save them as preset 'imported' for one-key starts?"),
                        Line::from(""),
                        Line::from("y save  n skip  Esc quit"),
                    ]
                }
                None => vec![
                    Line::from(format!("Connected as {username}. Configuration saved.")),
                    Line::from(""),
                    Line::from("JupyterCLI cannot list your hub's environment and resource"),
                    Line::from("options because JupyterHub does not expose them over its API."),
                    Line::from(format!(
                        "Start a server once in the browser at {url}/hub/spawn,"
                    )),
                    Line::from("then run: jhc preset import"),
                    Line::from(""),
                    Line::from("Enter open the dashboard"),
                ],
            }
        }
    };
    frame.render_widget(Paragraph::new(lines).block(block), frame.area());
}

pub async fn run(terminal: &mut ratatui::DefaultTerminal) -> Result<Option<Config>, CliError> {
    use futures_util::StreamExt as _;
    let mut events = crossterm::event::EventStream::new();
    let mut state = WizardState::new();
    let mut saved: Option<Config> = None;

    loop {
        terminal
            .draw(|frame| render(frame, &state))
            .map_err(CliError::Io)?;
        let Some(event) = events.next().await else {
            return Ok(None);
        };
        let key = match event.map_err(CliError::Io)? {
            crossterm::event::Event::Key(key) => key,
            _ => continue,
        };
        match state.on_key(&key) {
            WizardAction::None => {}
            WizardAction::Abort => return Ok(None),
            WizardAction::TestConnection => {
                // Draw the Testing frame before the await so the user sees it.
                terminal
                    .draw(|frame| render(frame, &state))
                    .map_err(CliError::Io)?;
                match fetch_identity(&state).await {
                    Ok((username, options)) => {
                        let config = build_config(&state);
                        config::save(&config)?;
                        saved = Some(config);
                        state.offer_preset(username, options);
                    }
                    Err(e) => state.fail(e.to_string()),
                }
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
}

fn build_config(state: &WizardState) -> Config {
    Config {
        default_hub: "default".to_string(),
        hubs: [(
            "default".to_string(),
            HubConfig {
                url: state.url.value().to_string(),
                token: state.token.value().to_string(),
                presets: Default::default(),
            },
        )]
        .into(),
    }
}

async fn fetch_identity(state: &WizardState) -> Result<(String, Option<JsonMap>), CliError> {
    let client = HubClient::new(state.url.value(), state.token.value())?.with_retry_warnings(false);
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
    fn render_names_the_product_and_masks_the_token() {
        let mut state = WizardState::new();
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, "https://x");
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, "abc");
        let backend = ratatui::backend::TestBackend::new(70, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &state)).unwrap();
        let text = crate::tui::render::buffer_text(&terminal);
        assert!(text.contains("JupyterCLI"));
        assert!(
            text.contains("hub/token"),
            "token guidance must name <url>/hub/token"
        );
        assert!(text.contains("***"));
        assert!(
            !text.contains("abc"),
            "token must never render in clear text"
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

        let (username, options) = fetch_identity(&state).await.unwrap();
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

        let mut state = WizardState::new();
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, &server.uri());
        state.on_key(&press(KeyCode::Enter));
        type_text(&mut state, "bad");

        let err = fetch_identity(&state).await.unwrap_err();
        assert!(err.to_string().contains("token invalid or expired"));
    }
}
