pub mod anim;
pub mod app;
pub mod dialogs;
pub mod grid;
pub mod input;
pub mod net;
pub mod render;
pub mod suspend;
pub mod theme;
pub mod wizard;

use std::time::{Duration, Instant};

use futures_util::StreamExt as _;

use crate::api::HubClient;
use crate::cli::CliError;
use crate::config::{self, Config, ConfigError};

const TICK_EVERY: Duration = Duration::from_millis(100);

pub async fn run(hub_flag: Option<&str>) -> Result<(), CliError> {
    let log_path = crate::logging::init_tui().ok();
    let print_log_path = || {
        if let Some(path) = log_path.as_deref() {
            eprintln!("log: {}", path.display());
        }
    };
    let cfg = match config::load() {
        Ok(cfg) => cfg,
        Err(ConfigError::NotFound(_)) => {
            let mut terminal = ratatui::init();
            let outcome = wizard::run(&mut terminal).await;
            match outcome {
                Ok(Some(cfg)) => {
                    let result = dashboard_loop(&mut terminal, cfg, hub_flag).await;
                    ratatui::restore();
                    print_log_path();
                    return result;
                }
                Ok(None) => {
                    ratatui::restore();
                    print_log_path();
                    return Ok(());
                }
                Err(e) => {
                    ratatui::restore();
                    print_log_path();
                    return Err(e);
                }
            }
        }
        Err(e) => return Err(e.into()),
    };
    let mut terminal = ratatui::init();
    let result = dashboard_loop(&mut terminal, cfg, hub_flag).await;
    ratatui::restore();
    print_log_path();
    result
}

async fn dashboard_loop(
    terminal: &mut ratatui::DefaultTerminal,
    cfg: Config,
    hub_flag: Option<&str>,
) -> Result<(), CliError> {
    let (hub_name, hub) = cfg.resolve_hub(hub_flag)?;
    // Verbose logging and retry warnings stay off in the TUI: stderr writes would
    // corrupt the alternate screen. JHC_DEBUG_LOG still works here (file-backed).
    let token = hub.effective_token();
    crate::api::log_client_init(hub_name, &hub.url, &token);
    let client = HubClient::new(&hub.url, &token)?.with_retry_warnings(false);
    let size = crossterm::terminal::size().unwrap_or((80, 24));
    let mut app = app::App::new(
        hub_name.to_string(),
        hub.presets.clone(),
        hub.effective_terminal_limit(),
        size,
    );

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut events = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(TICK_EVERY);
    let mut peek: Option<tokio::task::AbortHandle> = None;

    loop {
        terminal
            .draw(|frame| render::draw(frame, &app))
            .map_err(CliError::Io)?;

        tokio::select! {
            event = events.next() => match event {
                Some(Ok(crossterm::event::Event::Key(key))) => app.on_key(&key, Instant::now()),
                Some(Ok(crossterm::event::Event::Resize(cols, rows))) => app.set_size(cols, rows),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(CliError::Io(e)),
                None => return Ok(()),
            },
            message = rx.recv() => {
                if let Some(message) = message {
                    app.apply(message, Instant::now());
                }
            }
            _ = tick.tick() => app.tick(Instant::now()),
        }

        for effect in app.take_effects() {
            match effect {
                app::Effect::Quit => {
                    if let Some(handle) = peek.take() {
                        handle.abort();
                    }
                    return Ok(());
                }
                app::Effect::PeekStop => {
                    if let Some(handle) = peek.take() {
                        handle.abort();
                    }
                }
                app::Effect::PeekStart {
                    op,
                    url,
                    terminal,
                    rows,
                    cols,
                } => {
                    if let Some(handle) = peek.take() {
                        handle.abort();
                    }
                    peek = Some(net::spawn_peek(
                        op,
                        url,
                        terminal,
                        rows,
                        cols,
                        client.clone(),
                        tx.clone(),
                    ));
                }
                app::Effect::Attach { target } => {
                    if let Some(handle) = peek.take() {
                        handle.abort();
                    }
                    let message = suspend::attach_in_subprocess(&app.hub_name, &target).await?;
                    terminal.clear().map_err(CliError::Io)?;
                    app.after_attach(message, Instant::now());
                }
                app::Effect::SavePreset { name, options } => {
                    match config::add_preset(&app.hub_name, &name, options.clone()) {
                        Ok(()) => app.on_preset_saved(name, options, Instant::now()),
                        Err(e) => app.set_status(e.to_string(), true, Instant::now()),
                    }
                }
                other => net::dispatch(other, client.clone(), tx.clone()),
            }
        }
    }
}
