pub mod app;
pub mod dialogs;
pub mod input;
pub mod net;
pub mod render;
pub mod suspend;
pub mod wizard;

use std::time::{Duration, Instant};

use futures_util::StreamExt as _;

use crate::api::HubClient;
use crate::cli::CliError;
use crate::config::{self, Config, ConfigError};

const REFRESH_EVERY: Duration = Duration::from_secs(15);
const TICK_EVERY: Duration = Duration::from_millis(500);

pub async fn run(hub_flag: Option<&str>) -> Result<(), CliError> {
    let cfg = match config::load() {
        Ok(cfg) => cfg,
        Err(ConfigError::NotFound(_)) => {
            let mut terminal = ratatui::init();
            let outcome = wizard::run(&mut terminal).await;
            match outcome {
                Ok(Some(cfg)) => {
                    let result = dashboard_loop(&mut terminal, cfg, hub_flag).await;
                    ratatui::restore();
                    return result;
                }
                Ok(None) => {
                    ratatui::restore();
                    return Ok(());
                }
                Err(e) => {
                    ratatui::restore();
                    return Err(e);
                }
            }
        }
        Err(e) => return Err(e.into()),
    };
    let mut terminal = ratatui::init();
    let result = dashboard_loop(&mut terminal, cfg, hub_flag).await;
    ratatui::restore();
    result
}

async fn dashboard_loop(
    terminal: &mut ratatui::DefaultTerminal,
    cfg: Config,
    hub_flag: Option<&str>,
) -> Result<(), CliError> {
    let (hub_name, hub) = cfg.resolve_hub(hub_flag)?;
    // Verbose logging stays off in the TUI: stderr writes would corrupt the alternate screen.
    let client = HubClient::new(&hub.url, &hub.effective_token())?;
    let mut app = app::App::new(hub_name.to_string(), hub.presets.clone());

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut events = crossterm::event::EventStream::new();
    let mut refresh = tokio::time::interval(REFRESH_EVERY);
    refresh.tick().await; // consume the immediate first tick; App::new queued a Refresh already
    let mut tick = tokio::time::interval(TICK_EVERY);

    loop {
        terminal
            .draw(|frame| render::draw(frame, &app))
            .map_err(CliError::Io)?;

        tokio::select! {
            event = events.next() => match event {
                Some(Ok(crossterm::event::Event::Key(key))) => app.on_key(&key, Instant::now()),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(CliError::Io(e)),
                None => return Ok(()),
            },
            message = rx.recv() => {
                if let Some(message) = message {
                    app.apply(message, Instant::now());
                }
            }
            _ = refresh.tick() => {
                app.loading = true;
                net::dispatch(app::Effect::Refresh, client.clone(), tx.clone());
            }
            _ = tick.tick() => app.tick(Instant::now()),
        }

        for effect in app.take_effects() {
            match effect {
                app::Effect::Quit => return Ok(()),
                app::Effect::Attach { target } => {
                    let message = suspend::attach_in_subprocess(&app.hub_name, &target).await?;
                    terminal.clear().map_err(CliError::Io)?;
                    app.set_status(message, false, Instant::now());
                    if let Some(server) = app.selected_server()
                        && let Some(url) = server.url.clone()
                    {
                        let effect = app::Effect::FetchShells {
                            server: server.display.clone(),
                            url,
                        };
                        net::dispatch(effect, client.clone(), tx.clone());
                    }
                }
                other => net::dispatch(other, client.clone(), tx.clone()),
            }
        }
    }
}
