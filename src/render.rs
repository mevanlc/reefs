use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::{
    app::{App, RuntimeMode, TankState, WaterBand},
    creature::{CreatureDef, Entity, School, Variant},
    world::ReefWorld,
};

const MODAL_BORDER: Color = Color::LightCyan;
const KEY_HIGHLIGHT: Color = Color::Green;

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    match &app.mode {
        RuntimeMode::Tank(tank) => render_tank(frame, area, app, tank),
        RuntimeMode::Reef(reef) => {
            if area.height < reef.min_height {
                render_size_warning(frame, area, reef.min_height);
            } else {
                render_reef(frame, area, app, &reef.world);
            }
        }
    }

    if app.spawn_modal.is_some() {
        render_spawn_modal(frame, area, app);
    }

    if app.show_help {
        render_help_modal(frame, area);
    }
}

fn render_tank(frame: &mut Frame<'_>, area: Rect, app: &App, tank_state: &TankState) {
    if area.width < tank_state.width || area.height < tank_state.height {
        let message = Paragraph::new(vec![
            Line::from(format!(
                "Aquariuma needs a {}x{} terminal.",
                tank_state.width, tank_state.height
            )),
            Line::from(format!("Current size: {}x{}", area.width, area.height)),
            Line::from("Resize the terminal, or press q / Esc to quit."),
        ])
        .style(Style::new().fg(Color::LightCyan));
        frame.render_widget(message, area);
        return;
    }

    let tank = centered_rect(area, tank_state.width, tank_state.height);
    let water = Rect::new(tank.x + 1, tank.y + 1, tank.width - 2, tank.height - 2);
    let background_state = if app.show_background {
        "bg on"
    } else {
        "bg off"
    };
    let block = Block::new()
        .title(" Aquariuma ")
        .title_bottom(format!(
            " {} creatures | b {} | t names {} | q quit ",
            app.entities.len(),
            background_state,
            if app.show_creature_names { "on" } else { "off" }
        ))
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Blue))
        .style(Style::new().bg(Color::Black));
    frame.render_widget(block, tank);

    if app.show_background {
        render_water(frame, water, app.tick);
    }
    render_creatures(
        frame,
        water,
        &app.definitions,
        &app.entities,
        app.tick,
        0,
        app.show_creature_names,
    );
}

fn render_reef(frame: &mut Frame<'_>, area: Rect, app: &App, world: &ReefWorld) {
    if app.show_background {
        let band = WaterBand::for_reef(world, area.height);
        let water = Rect::new(
            area.x,
            area.y + band.top.max(0) as u16,
            area.width,
            (band.bottom - band.top).max(0) as u16,
        );
        render_water(frame, water, app.tick);
    }

    render_layer(frame, area, world, LayerPosition::Surface);
    render_surface_overlay(frame, area);
    render_layer(frame, area, world, LayerPosition::Floor);
    render_creatures(
        frame,
        area,
        &app.definitions,
        &app.entities,
        app.tick,
        world.viewport_x,
        app.show_creature_names,
    );
}

fn render_surface_overlay(frame: &mut Frame<'_>, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let buffer = frame.buffer_mut();
    let label_style = Style::new().fg(Color::LightCyan);
    render_surface_text(buffer, area, 2, " reefs ", label_style);
    render_surface_text(buffer, area, 11, " ", label_style);
    render_surface_text(buffer, area, 12, "?", key_style());
    render_surface_text(buffer, area, 13, " help ", label_style);
}

fn render_surface_text(buffer: &mut Buffer, area: Rect, offset: u16, text: &str, style: Style) {
    if offset >= area.width {
        return;
    }

    let x = area.x + offset;
    let width = area.right().saturating_sub(x) as usize;
    buffer.set_stringn(x, area.y, text, width, style);
}

fn render_size_warning(frame: &mut Frame<'_>, area: Rect, min_height: u16) {
    let message = Paragraph::new(vec![
        Line::from("Aquariuma reef mode needs more rows."),
        Line::from(format!("Minimum rows: {min_height}")),
        Line::from(format!("Current rows: {}", area.height)),
        Line::from("Resize the terminal, or press q / Esc to quit."),
    ])
    .style(Style::new().fg(Color::LightCyan));
    frame.render_widget(message, area);
}

fn render_spawn_modal(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(modal) = app.spawn_modal.as_ref() else {
        return;
    };

    let width = area.width.clamp(36, 54);
    let footer_rows = spawn_help_line_count() + 1;
    let fixed_rows = 2 + footer_rows + 2;
    let max_list_rows = area.height.saturating_sub(fixed_rows).max(1);
    let list_rows = (modal.order.len() as u16).min(max_list_rows);
    let height = list_rows
        .saturating_add(fixed_rows)
        .min(area.height)
        .max(fixed_rows);
    let modal_area = centered_rect(area, width, height);
    let visible_rows = modal_area.height.saturating_sub(fixed_rows) as usize;
    let selected = modal.selected.min(modal.order.len().saturating_sub(1));
    let start = selected
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(modal.order.len().saturating_sub(visible_rows));
    let end = modal.order.len().min(start.saturating_add(visible_rows));
    let inner_width = modal_area.width.saturating_sub(2) as usize;
    let count_width = "# spawned".len();
    let marker_width = 2;
    let gap_width = 2;
    let name_width = inner_width
        .saturating_sub(marker_width + gap_width + count_width)
        .max("name".len());
    let mut lines = vec![
        Line::from(format!(
            "{:marker_width$}{:<name_width$}{:gap_width$}{:>count_width$}",
            "", "name", "", "# spawned"
        )),
        Line::from(format!(
            "{:marker_width$}{:-<name_width$}{:gap_width$}{:->count_width$}",
            "", "", "", ""
        )),
    ];
    lines.extend(
        modal.order[start..end]
            .iter()
            .enumerate()
            .map(|(offset, def_index)| {
                let index = start + offset;
                let definition = &app.definitions[*def_index];
                let count = app
                    .entities
                    .iter()
                    .filter(|entity| entity.def == *def_index)
                    .count();
                let name = fit_column(&definition.name, name_width);
                let label = format!(
                    "{:<marker_width$}{name}{:gap_width$}{count:>count_width$}",
                    if index == selected { ">" } else { " " },
                    ""
                );
                if index == selected {
                    Line::from(label).black().on_light_cyan()
                } else {
                    Line::from(label)
                }
            }),
    );
    lines.push(Line::from(format!("{:-<inner_width$}", "")));
    lines.extend(spawn_help_lines(inner_width));

    let block = Block::new()
        .title(" Spawn ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(MODAL_BORDER))
        .style(Style::new().bg(Color::Black));
    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(Style::new().fg(Color::White).bg(Color::Black));

    frame.render_widget(Clear, modal_area);
    frame.render_widget(paragraph, modal_area);
}

fn fit_column(text: &str, width: usize) -> String {
    let mut fitted = text.chars().take(width).collect::<String>();
    let len = fitted.chars().count();
    if len < width {
        fitted.push_str(&" ".repeat(width - len));
    }
    fitted
}

fn spawn_help_line_count() -> u16 {
    2
}

fn spawn_help_lines(width: usize) -> Vec<Line<'static>> {
    vec![
        shortcut_line(
            &[
                ("↑", true),
                ("/", false),
                ("↓", true),
                (" select  ", false),
                ("Enter", true),
                (" spawn", false),
            ],
            width,
            Color::DarkGray,
        ),
        shortcut_line(
            &[
                ("Esc", true),
                ("/", false),
                ("Ctrl", true),
                ("+", false),
                ("S", true),
                (" close", false),
            ],
            width,
            Color::DarkGray,
        ),
    ]
}

fn render_help_modal(frame: &mut Frame<'_>, area: Rect) {
    let shortcuts = [
        help_line(&[("?", true)], "show or hide help"),
        help_line(&[("Esc", true)], "close modal / quit"),
        help_line(&[("q", true)], "quit"),
        help_line(&[("b", true)], "toggle water background"),
        help_line(&[("t", true)], "toggle creature names"),
        help_line(&[("+", true)], "spawn a random creature"),
        help_line(&[("-", true)], "despawn a random creature"),
        help_line(
            &[("Ctrl", true), ("+", false), ("S", true)],
            "open spawn menu",
        ),
        help_line(&[("←", true), ("/", false), ("→", true)], "scroll reef"),
        help_line(
            &[
                ("Shift", true),
                ("+", false),
                ("↑", true),
                ("/", false),
                ("↓", true),
            ],
            "adjust speed",
        ),
    ];
    let width = area.width.clamp(37, 42);
    let height = ((shortcuts.len() + 2) as u16).min(area.height).max(3);
    let modal_area = centered_rect(area, width, height);
    let visible_rows = modal_area.height.saturating_sub(2) as usize;
    let lines = shortcuts.into_iter().take(visible_rows).collect::<Vec<_>>();
    let block = Block::new()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(MODAL_BORDER))
        .style(Style::new().bg(Color::Black));
    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(Style::new().bg(Color::Black));

    frame.render_widget(Clear, modal_area);
    frame.render_widget(paragraph, modal_area);
}

fn help_line(key_parts: &[(&'static str, bool)], description: &'static str) -> Line<'static> {
    const KEY_WIDTH: usize = 9;

    let key_style = key_style();
    let plain_style = Style::new().fg(Color::White);
    let key_len = key_parts
        .iter()
        .map(|(part, _)| part.chars().count())
        .sum::<usize>();
    let padding = KEY_WIDTH.saturating_sub(key_len) + 1;
    let mut spans = key_parts
        .iter()
        .map(|(part, highlighted)| {
            if *highlighted {
                Span::styled(*part, key_style)
            } else {
                Span::styled(*part, plain_style)
            }
        })
        .collect::<Vec<_>>();
    spans.push(Span::styled(" ".repeat(padding), plain_style));
    spans.push(Span::styled(description, plain_style));

    Line::from(spans)
}

fn shortcut_line(
    parts: &[(&'static str, bool)],
    width: usize,
    plain_color: Color,
) -> Line<'static> {
    let plain_style = Style::new().fg(plain_color);
    let len = parts
        .iter()
        .map(|(part, _)| part.chars().count())
        .sum::<usize>();
    let mut spans = shortcut_spans(parts, plain_style);
    if len < width {
        spans.push(Span::styled(" ".repeat(width - len), plain_style));
    }

    Line::from(spans)
}

fn shortcut_spans(parts: &[(&'static str, bool)], plain_style: Style) -> Vec<Span<'static>> {
    parts
        .iter()
        .map(|(part, highlighted)| {
            if *highlighted {
                Span::styled(*part, key_style())
            } else {
                Span::styled(*part, plain_style)
            }
        })
        .collect()
}

fn key_style() -> Style {
    Style::new().fg(KEY_HIGHLIGHT).add_modifier(Modifier::BOLD)
}

#[derive(Debug, Clone, Copy)]
enum LayerPosition {
    Surface,
    Floor,
}

fn render_layer(frame: &mut Frame<'_>, area: Rect, world: &ReefWorld, position: LayerPosition) {
    let (layer, start_y) = match position {
        LayerPosition::Surface => (&world.surface, area.y),
        LayerPosition::Floor => (
            &world.floor,
            area.bottom().saturating_sub(world.floor.height),
        ),
    };
    let style = Style::new().fg(layer.color);
    let buffer = frame.buffer_mut();

    for row in 0..layer.height {
        let y = start_y + row;
        if y >= area.bottom() {
            continue;
        }

        for x in 0..area.width {
            if let Some(symbol) = layer.cell_at(world.viewport_x + x as i32, row)
                && let Some(cell) = buffer.cell_mut((area.x + x, y))
            {
                let mut encoded = [0; 4];
                cell.set_symbol(symbol.encode_utf8(&mut encoded))
                    .set_style(style);
            }
        }
    }
}

fn render_water(frame: &mut Frame<'_>, area: Rect, tick: u64) {
    let buffer = frame.buffer_mut();
    let water_style = Style::new().fg(Color::DarkGray);
    for y in 0..area.height {
        for x in 0..area.width {
            let ripple = match (x as u64 + y as u64 * 3 + tick / 2) % 23 {
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

fn render_creatures(
    frame: &mut Frame<'_>,
    area: Rect,
    definitions: &[CreatureDef],
    entities: &[Entity],
    tick: u64,
    viewport_x: i32,
    show_names: bool,
) {
    let buffer = frame.buffer_mut();

    for entity in entities {
        if !entity.is_active() {
            continue;
        }

        let def = &definitions[entity.def];
        let variant = def.best_variant_for(
            entity.pose_dx(),
            entity.pose_intent,
            entity.animation_tick(tick),
            entity.phase,
        );
        let style = Style::new().fg(entity.color).add_modifier(if def.brownian {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });

        if let Some(school) = &variant.school {
            render_school(buffer, area, entity, variant, school, viewport_x, style);
        } else {
            render_static_art(buffer, area, entity, variant, viewport_x, style);
        }

        if show_names {
            render_creature_name(
                buffer,
                area,
                entity,
                variant.width,
                variant.height,
                &def.name,
                viewport_x,
            );
        }
    }
}

fn render_static_art(
    buffer: &mut Buffer,
    area: Rect,
    entity: &Entity,
    variant: &Variant,
    viewport_x: i32,
    style: Style,
) {
    for (line_index, line) in variant.art.iter().enumerate() {
        let y = area.y as i32 + entity.y + line_index as i32;
        if y < area.y as i32 || y >= area.bottom() as i32 {
            continue;
        }

        let raw_x = area.x as i32 + entity.x - viewport_x;
        if raw_x >= area.right() as i32 {
            continue;
        }

        let (x, text) = if raw_x < area.x as i32 {
            let skip = (area.x as i32 - raw_x) as usize;
            let clipped = line.chars().skip(skip).collect::<String>();
            (area.x, clipped)
        } else {
            (raw_x as u16, line.clone())
        };

        if text.is_empty() || x >= area.right() {
            continue;
        }

        let width = area.right().saturating_sub(x) as usize;
        buffer.set_stringn(x, y as u16, text, width, style);
    }
}

fn render_school(
    buffer: &mut Buffer,
    area: Rect,
    entity: &Entity,
    variant: &Variant,
    school: &School,
    viewport_x: i32,
    style: Style,
) {
    let unit_width = school.unit.chars().count().max(1) as u16;
    let max_x = variant.width.saturating_sub(unit_width) as u64;
    let max_y = variant.height.saturating_sub(1) as u64;

    for (index, unit) in school.units.iter().enumerate() {
        let local_x = brownian_coordinate(
            unit.x as u64,
            max_x,
            entity.school_rearrangements,
            entity.phase,
            index,
            0,
        );
        let local_y = brownian_coordinate(
            unit.y as u64,
            max_y,
            entity.school_rearrangements,
            entity.phase,
            index,
            1,
        );
        let raw_x = area.x as i32 + entity.x - viewport_x + local_x as i32;
        let y = area.y as i32 + entity.y + local_y as i32;
        if y < area.y as i32 || y >= area.bottom() as i32 {
            continue;
        }

        render_clipped_text(buffer, area, raw_x, y as u16, &school.unit, style);
    }
}

fn brownian_coordinate(
    origin: u64,
    max: u64,
    rearrangements: u64,
    phase: usize,
    unit_index: usize,
    axis: u64,
) -> u64 {
    if max == 0 || rearrangements == 0 {
        return origin.min(max);
    }

    let seed = rearrangements
        .wrapping_add((phase as u64).wrapping_mul(0x9e37_79b9))
        .wrapping_add((unit_index as u64).wrapping_mul(0x85eb_ca6b))
        .wrapping_add(axis.wrapping_mul(0xc2b2_ae35));
    let drift = stable_hash(seed) % (max + 1);

    origin.wrapping_add(drift).wrapping_rem(max + 1)
}

fn stable_hash(mut value: u64) -> u64 {
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    value ^ (value >> 33)
}

fn render_clipped_text(
    buffer: &mut Buffer,
    area: Rect,
    raw_x: i32,
    y: u16,
    text: &str,
    style: Style,
) {
    let text_width = text.chars().count() as i32;
    if text_width == 0 || raw_x >= area.right() as i32 || raw_x + text_width <= area.x as i32 {
        return;
    }

    let (x, text) = if raw_x < area.x as i32 {
        let skip = (area.x as i32 - raw_x) as usize;
        (area.x, text.chars().skip(skip).collect::<String>())
    } else {
        (raw_x as u16, text.to_string())
    };

    if text.is_empty() || x >= area.right() {
        return;
    }

    let width = area.right().saturating_sub(x) as usize;
    buffer.set_stringn(x, y, text, width, style);
}

fn render_creature_name(
    buffer: &mut Buffer,
    area: Rect,
    entity: &Entity,
    creature_width: u16,
    creature_height: u16,
    name: &str,
    viewport_x: i32,
) {
    let name_width = name.chars().count() as i32;
    if name_width == 0 {
        return;
    }

    let y = area.y as i32 + entity.y + creature_height as i32;
    if y < area.y as i32 || y >= area.bottom() as i32 {
        return;
    }

    let creature_center = area.x as i32 + entity.x - viewport_x + creature_width as i32 / 2;
    let raw_x = creature_center - name_width / 2;
    if raw_x >= area.right() as i32 || raw_x + name_width <= area.x as i32 {
        return;
    }

    let (x, text) = if raw_x < area.x as i32 {
        let skip = (area.x as i32 - raw_x) as usize;
        (area.x, name.chars().skip(skip).collect::<String>())
    } else {
        (raw_x as u16, name.to_string())
    };

    if text.is_empty() || x >= area.right() {
        return;
    }

    let width = area.right().saturating_sub(x) as usize;
    let style = Style::new().fg(Color::LightCyan);
    buffer.set_stringn(x, y as u16, text, width, style);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    use super::*;
    use crate::{
        app::{App, SpawnModal},
        config::load_config,
        creature::load_creatures,
    };

    #[test]
    fn reef_surface_renders_help_hint_overlay() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let app = App::new(config, definitions, Rect::new(0, 0, 80, 30)).expect("app starts");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal starts");

        terminal.draw(|frame| render(frame, &app)).expect("draws");

        let buffer = terminal.backend().buffer();
        let rendered = (0..25)
            .filter_map(|x| buffer.cell((x, 0)))
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert_eq!(rendered, "~~ reefs ~~ ? help ~~~~~~");
        let help_key = buffer.cell((12, 0)).expect("surface help key");
        assert_eq!(help_key.fg, Color::Green);
        assert!(help_key.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn spawn_modal_renders_aligned_table_headers() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 80, 30)).expect("app starts");
        app.spawn_modal = Some(SpawnModal {
            selected: 0,
            order: (0..app.definitions.len()).collect(),
        });
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal starts");

        terminal.draw(|frame| render(frame, &app)).expect("draws");

        let buffer = terminal.backend().buffer();
        let rows = (0..30)
            .map(|y| {
                (0..80)
                    .filter_map(|x| buffer.cell((x, y)))
                    .map(|cell| cell.symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            rows.iter()
                .any(|row| row.contains("name                                     # spawned"))
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("---------------------------------------  ---------"))
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("↑/↓ select  Enter spawn"))
        );
        assert!(rows.iter().any(|row| row.contains("Esc/Ctrl+S close")));

        let (row_index, row) = rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.contains("Esc/Ctrl+S close"))
            .expect("spawn hint row is rendered");
        let key_start = row.find("Esc/Ctrl+S").expect("spawn hint key starts");
        let key_x = row[..key_start].chars().count() as u16;
        let y = row_index as u16;
        let esc_cell = buffer.cell((key_x, y)).expect("Esc cell");
        let slash_cell = buffer.cell((key_x + 3, y)).expect("slash cell");
        let ctrl_cell = buffer.cell((key_x + 4, y)).expect("Ctrl cell");
        let plus_cell = buffer.cell((key_x + 8, y)).expect("plus cell");
        let s_cell = buffer.cell((key_x + 9, y)).expect("S cell");

        assert_eq!(esc_cell.fg, Color::Green);
        assert!(esc_cell.modifier.contains(Modifier::BOLD));
        assert!(!slash_cell.modifier.contains(Modifier::BOLD));
        assert_eq!(ctrl_cell.fg, Color::Green);
        assert!(ctrl_cell.modifier.contains(Modifier::BOLD));
        assert!(!plus_cell.modifier.contains(Modifier::BOLD));
        assert_eq!(s_cell.fg, Color::Green);
        assert!(s_cell.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn help_modal_excludes_spawn_menu_hints() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 80, 30)).expect("app starts");
        app.show_help = true;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal starts");

        terminal.draw(|frame| render(frame, &app)).expect("draws");

        let buffer = terminal.backend().buffer();
        let rows = (0..30)
            .map(|y| {
                (0..80)
                    .filter_map(|x| buffer.cell((x, y)))
                    .map(|cell| cell.symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            !rows
                .iter()
                .any(|row| row.contains("Spawn menu: Up/Down select, Enter spawn"))
        );
    }

    #[test]
    fn help_modal_aligns_and_highlights_key_tokens() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 80, 30)).expect("app starts");
        app.show_help = true;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal starts");

        terminal.draw(|frame| render(frame, &app)).expect("draws");

        let buffer = terminal.backend().buffer();
        let rows = (0..30)
            .map(|y| {
                (0..80)
                    .filter_map(|x| buffer.cell((x, y)))
                    .map(|cell| cell.symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            rows.iter()
                .any(|row| row.contains("?         show or hide help"))
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("Ctrl+S    open spawn menu"))
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("Shift+↑/↓ adjust speed"))
        );

        let (row_index, row) = rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.contains("Ctrl+S    open spawn menu"))
            .expect("Ctrl+S row is rendered");
        let key_start = row.find("Ctrl+S").expect("Ctrl+S text starts");
        let key_x = row[..key_start].chars().count() as u16;
        let y = row_index as u16;
        let ctrl_cell = buffer.cell((key_x, y)).expect("Ctrl cell");
        let plus_cell = buffer.cell((key_x + 4, y)).expect("plus cell");
        let s_cell = buffer.cell((key_x + 5, y)).expect("S cell");

        assert_eq!(ctrl_cell.fg, Color::Green);
        assert!(ctrl_cell.modifier.contains(Modifier::BOLD));
        assert!(!plus_cell.modifier.contains(Modifier::BOLD));
        assert_eq!(s_cell.fg, Color::Green);
        assert!(s_cell.modifier.contains(Modifier::BOLD));
    }
}
