pub mod input;
pub mod render;

use crate::cli::CliError;
use crate::config::{self, ConfigError};

pub async fn run(hub_flag: Option<&str>) -> Result<(), CliError> {
    match config::load() {
        Ok(cfg) => {
            let _ = (cfg, hub_flag);
            placeholder_loop().await
        }
        Err(ConfigError::NotFound(_)) => placeholder_loop().await,
        Err(e) => Err(e.into()),
    }
}

async fn placeholder_loop() -> Result<(), CliError> {
    use futures_util::StreamExt as _;
    let mut terminal = ratatui::init();
    let mut events = crossterm::event::EventStream::new();
    let result = loop {
        if let Err(e) = terminal.draw(render::draw_placeholder) {
            break Err(CliError::Io(e));
        }
        match events.next().await {
            Some(Ok(crossterm::event::Event::Key(key)))
                if key.kind == crossterm::event::KeyEventKind::Press
                    && key.code == crossterm::event::KeyCode::Char('q') =>
            {
                break Ok(());
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => break Err(CliError::Io(e)),
            None => break Ok(()),
        }
    };
    ratatui::restore();
    result
}
