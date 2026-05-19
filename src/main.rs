mod app;
mod config;
mod creature;
mod kdl_parse;
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

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, process::Command};

    use kdl::KdlDocument;

    use crate::kdl_parse::format_parse_error;

    #[test]
    fn all_repo_kdl_files_parse() {
        let mut failures = Vec::new();

        for path in repo_kdl_files() {
            let source = match fs::read_to_string(&path) {
                Ok(source) => source,
                Err(error) => {
                    failures.push(format!("{}: read failed: {error}", path.display()));
                    continue;
                }
            };

            if let Err(error) = source.parse::<KdlDocument>() {
                failures.push(format_parse_error(&path, &source, &error));
            }
        }

        assert!(
            failures.is_empty(),
            "KDL parse failures:\n{}",
            failures.join("\n")
        );
    }

    fn repo_kdl_files() -> Vec<PathBuf> {
        let output = Command::new("rg")
            .args(["--files", "-g", "*.kdl"])
            .output()
            .expect("rg is available for repo file discovery");
        assert!(
            output.status.success(),
            "rg failed while listing KDL files: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(PathBuf::from)
            .collect()
    }
}
