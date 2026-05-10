use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use color_eyre::{
    Section, SectionExt,
    eyre::{Context, Result, eyre},
};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use kdl::{KdlDocument, KdlValue};
use rand::{RngExt, rngs::ThreadRng};
use ratatui::{
    DefaultTerminal, Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

const TANK_WIDTH: u16 = 120;
const TANK_HEIGHT: u16 = 40;
const TICK_RATE: Duration = Duration::from_millis(140);

fn main() -> Result<()> {
    color_eyre::install()?;

    let definitions = load_creatures(Path::new("art/creatures"))?;
    let mut terminal = ratatui::init();
    let result = App::new(definitions).run(&mut terminal);
    ratatui::restore();
    result
}

#[derive(Debug, Clone)]
struct CreatureDef {
    name: String,
    variants: Vec<Variant>,
    h_velocity: Option<i16>,
    v_velocity: Option<i16>,
    brownian: bool,
}

impl CreatureDef {
    fn best_variant(&self, dx: i16, tick: u64, phase: usize) -> &Variant {
        let wanted = if dx < 0 {
            "left"
        } else if dx > 0 {
            "right"
        } else {
            "face"
        };

        let matching = self
            .variants
            .iter()
            .filter(|variant| variant.pose.starts_with(wanted))
            .collect::<Vec<_>>();

        if matching.is_empty() {
            let face = self
                .variants
                .iter()
                .filter(|variant| variant.pose.starts_with("face"))
                .collect::<Vec<_>>();
            if !face.is_empty() {
                return face[(tick as usize / 3 + phase) % face.len()];
            }
            return &self.variants[(tick as usize / 3 + phase) % self.variants.len()];
        }

        matching[(tick as usize / 3 + phase) % matching.len()]
    }

    fn starting_velocity(&self, rng: &mut ThreadRng) -> (i16, i16) {
        let dx = self.h_velocity.unwrap_or_else(|| {
            let has_left = self
                .variants
                .iter()
                .any(|variant| variant.pose.starts_with("left"));
            let has_right = self
                .variants
                .iter()
                .any(|variant| variant.pose.starts_with("right"));

            match (has_left, has_right) {
                (true, true) => {
                    if rng.random_bool(0.5) {
                        -1
                    } else {
                        1
                    }
                }
                (true, false) => -1,
                (false, true) => 1,
                (false, false) => {
                    if rng.random_bool(0.5) {
                        -1
                    } else {
                        1
                    }
                }
            }
        });

        let dy = self.v_velocity.unwrap_or_else(|| {
            if self.brownian || rng.random_bool(0.35) {
                rng.random_range(-1..=1)
            } else {
                0
            }
        });

        (dx, dy)
    }
}

#[derive(Debug, Clone)]
struct Variant {
    pose: String,
    art: Vec<String>,
    width: u16,
    height: u16,
}

impl Variant {
    fn from_kdl_node(pose: String, art: &str) -> Self {
        let art = art
            .trim_matches('\n')
            .lines()
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect::<Vec<_>>();
        let width = art
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or_default()
            .min(u16::MAX as usize) as u16;
        let height = art.len().min(u16::MAX as usize) as u16;

        Self {
            pose,
            art,
            width,
            height,
        }
    }
}

#[derive(Debug)]
struct Entity {
    def: usize,
    x: i16,
    y: i16,
    dx: i16,
    dy: i16,
    phase: usize,
    color: Color,
}

impl Entity {
    fn tick(&mut self, def: &CreatureDef, bounds: Rect, variant: &Variant, rng: &mut ThreadRng) {
        if def.brownian && rng.random_bool(0.25) {
            self.dx = rng.random_range(-1..=1);
            self.dy = rng.random_range(-1..=1);
        }

        let max_x = (bounds.width.saturating_sub(variant.width).max(1) - 1) as i16;
        let max_y = (bounds.height.saturating_sub(variant.height).max(1) - 1) as i16;

        self.x += self.dx;
        self.y += self.dy;

        if self.x <= 0 {
            self.x = 0;
            self.dx = self.dx.abs().max(1);
        } else if self.x >= max_x {
            self.x = max_x;
            self.dx = -self.dx.abs().max(1);
        }

        if self.y <= 0 {
            self.y = 0;
            self.dy = self.dy.abs();
        } else if self.y >= max_y {
            self.y = max_y;
            self.dy = -self.dy.abs();
        }
    }
}

struct App {
    definitions: Vec<CreatureDef>,
    entities: Vec<Entity>,
    tick: u64,
    show_background: bool,
}

impl App {
    fn new(definitions: Vec<CreatureDef>) -> Self {
        let mut rng = rand::rng();
        let colors = [
            Color::LightCyan,
            Color::LightBlue,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightMagenta,
            Color::Cyan,
            Color::Green,
            Color::Yellow,
            Color::White,
        ];

        let entities = definitions
            .iter()
            .enumerate()
            .map(|(def_index, def)| {
                let (dx, dy) = def.starting_velocity(&mut rng);
                let variant = def.best_variant(dx, 0, def_index);
                let max_x = TANK_WIDTH
                    .saturating_sub(2)
                    .saturating_sub(variant.width)
                    .max(1) as i16;
                let max_y = TANK_HEIGHT
                    .saturating_sub(2)
                    .saturating_sub(variant.height)
                    .max(1) as i16;

                let name_hash = def
                    .name
                    .bytes()
                    .fold(0usize, |hash, byte| hash.wrapping_add(byte as usize));

                Entity {
                    def: def_index,
                    x: rng.random_range(0..=max_x),
                    y: rng.random_range(0..=max_y),
                    dx,
                    dy,
                    phase: rng.random_range(0..8),
                    color: colors[(def_index + name_hash) % colors.len()],
                }
            })
            .collect();

        Self {
            definitions,
            entities,
            tick: 0,
            show_background: false,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let mut last_tick = Instant::now();

        loop {
            terminal.draw(|frame| self.render(frame))?;

            let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
            if event::poll(timeout)?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('b') => self.show_background = !self.show_background,
                    _ => {}
                }
            }

            if last_tick.elapsed() >= TICK_RATE {
                self.tick();
                last_tick = Instant::now();
            }
        }
    }

    fn tick(&mut self) {
        self.tick += 1;
        let bounds = Rect::new(0, 0, TANK_WIDTH - 2, TANK_HEIGHT - 2);
        let mut rng = rand::rng();

        for entity in &mut self.entities {
            let def = &self.definitions[entity.def];
            let variant = def.best_variant(entity.dx, self.tick, entity.phase);
            entity.tick(def, bounds, variant, &mut rng);
        }
    }

    fn render(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        if area.width < TANK_WIDTH || area.height < TANK_HEIGHT {
            let message = Paragraph::new(vec![
                Line::from("Aquariuma needs a 120x40 terminal."),
                Line::from(format!("Current size: {}x{}", area.width, area.height)),
                Line::from("Resize the terminal, or press q / Esc to quit."),
            ])
            .style(Style::new().fg(Color::LightCyan));
            frame.render_widget(message, area);
            return;
        }

        let tank = centered_rect(area, TANK_WIDTH, TANK_HEIGHT);
        let water = Rect::new(tank.x + 1, tank.y + 1, tank.width - 2, tank.height - 2);
        let background_state = if self.show_background {
            "bg on"
        } else {
            "bg off"
        };
        let block = Block::new()
            .title(" Aquariuma ")
            .title_bottom(format!(
                " {} creatures | b {} | q quit ",
                self.definitions.len(),
                background_state
            ))
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::Blue))
            .style(Style::new().bg(Color::Black));
        frame.render_widget(block, tank);

        if self.show_background {
            self.render_water(frame, water);
        }
        self.render_creatures(frame, water);
    }

    fn render_water(&self, frame: &mut Frame<'_>, area: Rect) {
        let buffer = frame.buffer_mut();
        let water_style = Style::new().fg(Color::DarkGray);
        for y in 0..area.height {
            for x in 0..area.width {
                let ripple = match (x as u64 + y as u64 * 3 + self.tick / 2) % 23 {
                    0 => "~",
                    7 => ".",
                    _ => " ",
                };
                if ripple != " "
                    && let Some(cell) = buffer.cell_mut((area.x + x, area.y + y))
                {
                    cell.set_symbol(ripple).set_style(water_style);
                }
            }
        }
    }

    fn render_creatures(&self, frame: &mut Frame<'_>, area: Rect) {
        let buffer = frame.buffer_mut();

        for entity in &self.entities {
            let def = &self.definitions[entity.def];
            let variant = def.best_variant(entity.dx, self.tick, entity.phase);
            let style = Style::new().fg(entity.color).add_modifier(if def.brownian {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });

            for (line_index, line) in variant.art.iter().enumerate() {
                let y = area.y + entity.y.max(0) as u16 + line_index as u16;
                if y >= area.bottom() {
                    continue;
                }

                let x = area.x + entity.x.max(0) as u16;
                if x >= area.right() {
                    continue;
                }

                let width = area.right().saturating_sub(x) as usize;
                buffer.set_stringn(x, y, line, width, style);
            }
        }
    }
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
}

fn load_creatures(dir: &Path) -> Result<Vec<CreatureDef>> {
    let mut paths = fs::read_dir(dir)
        .wrap_err_with(|| format!("reading creature directory {}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();

    let creatures = paths
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "kdl"))
        .map(|path| load_creature(&path))
        .collect::<Result<Vec<_>>>()?;

    if creatures.is_empty() {
        Err(eyre!("no .kdl creatures found in {}", dir.display()))
    } else {
        Ok(creatures)
    }
}

fn load_creature(path: &Path) -> Result<CreatureDef> {
    let source =
        fs::read_to_string(path).wrap_err_with(|| format!("reading {}", path.display()))?;
    let doc = source
        .parse::<KdlDocument>()
        .wrap_err_with(|| format!("parsing {}", path.display()))
        .with_section(|| source.clone().header("KDL source"))?;

    let fallback_name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("creature")
        .to_string();

    let name = string_arg(&doc, "name").unwrap_or(fallback_name);
    let brownian = string_arg(&doc, "unit-motion").is_some_and(|motion| motion == "brownian");
    let h_velocity = int_arg(&doc, "h-velocity").map(clamp_velocity);
    let v_velocity = int_arg(&doc, "v-velocity").map(clamp_velocity);
    let variants = doc
        .nodes()
        .iter()
        .filter_map(|node| {
            let pose = node.name().value();
            if !is_pose_node(pose) {
                return None;
            }
            let art = node.get(0)?.as_string()?;
            Some(Variant::from_kdl_node(pose.to_string(), art))
        })
        .collect::<Vec<_>>();

    if variants.is_empty() {
        return Err(eyre!("{} has no drawable pose nodes", path.display()));
    }

    Ok(CreatureDef {
        name,
        variants,
        h_velocity,
        v_velocity,
        brownian,
    })
}

fn is_pose_node(name: &str) -> bool {
    ["left", "right", "face", "away"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn string_arg(doc: &KdlDocument, node_name: &str) -> Option<String> {
    doc.get(node_name)
        .and_then(|node| node.get(0))
        .and_then(KdlValue::as_string)
        .map(ToOwned::to_owned)
}

fn int_arg(doc: &KdlDocument, node_name: &str) -> Option<i128> {
    doc.get(node_name)
        .and_then(|node| node.get(0))
        .and_then(KdlValue::as_integer)
}

fn clamp_velocity(value: i128) -> i16 {
    value.clamp(-1, 1) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_all_creature_files() {
        let creatures = load_creatures(Path::new("art/creatures")).expect("creatures load");

        assert!(creatures.len() >= 17);
        assert!(creatures.iter().any(|creature| creature.name == "boxfish"));
        assert!(creatures.iter().any(|creature| creature.name == "mj"));
        assert!(
            creatures
                .iter()
                .all(|creature| !creature.variants.is_empty())
        );
    }

    #[test]
    fn turtle_has_left_and_right_animation_variants() {
        let turtle = load_creature(Path::new("art/creatures/turtle.kdl")).expect("turtle loads");
        let poses = turtle
            .variants
            .iter()
            .map(|variant| variant.pose.as_str())
            .collect::<Vec<_>>();

        assert!(poses.contains(&"left"));
        assert!(poses.contains(&"left1"));
        assert!(poses.contains(&"left2"));
        assert!(poses.contains(&"right"));
        assert!(poses.contains(&"right1"));
        assert!(poses.contains(&"right2"));
    }
}
