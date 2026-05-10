mod app;
mod config;
mod creature;
mod render;
mod world;

use std::io;

use color_eyre::eyre::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};

use crate::{app::App, config::Mode, creature::load_creatures};

fn main() -> Result<()> {
    color_eyre::install()?;

    let config = config::load_config("config.kdl".as_ref())?;
    let definitions = load_creatures("art/creatures".as_ref())?;
    let enable_mouse = matches!(config.mode, Mode::Reef);

    let mut terminal = ratatui::init();
    let mut mouse_enabled = false;

    let result = (|| -> Result<()> {
        if enable_mouse {
            let mut stdout = io::stdout();
            execute!(stdout, EnableMouseCapture)?;
            mouse_enabled = true;
        }

        let launch_size = terminal.size()?;
        let launch_area = ratatui::layout::Rect::new(0, 0, launch_size.width, launch_size.height);
        let mut app = App::new(config, definitions, launch_area)?;
        app.run(&mut terminal)
    })();

    if mouse_enabled {
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, DisableMouseCapture)
            && result.is_ok()
        {
            ratatui::restore();
            return Err(error.into());
        }
    }

    ratatui::restore();
    result
}
