use std::time::{Duration, Instant};

use color_eyre::eyre::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use rand::{RngExt, rngs::ThreadRng};
use ratatui::{DefaultTerminal, Frame, layout::Rect, style::Color};

use crate::{
    config::{AppConfig, Mode},
    creature::{CreatureDef, Entity, PoseIntent, SpawnLocation, Variant, tallest_variant_height},
    render,
    world::{ReefWorld, WorldBounds, load_world_layer},
};

const DEFAULT_TICK_RATE: Duration = Duration::from_millis(140);
const MIN_TICK_RATE: Duration = Duration::from_millis(10);
const MAX_TICK_RATE: Duration = Duration::from_millis(3000);
const TICK_RATE_STEP: Duration = Duration::from_millis(20);
const SCROLL_STEP: i32 = 4;

pub struct App {
    pub(crate) definitions: Vec<CreatureDef>,
    pub(crate) entities: Vec<Entity>,
    pub(crate) tick: u64,
    pub(crate) show_background: bool,
    pub(crate) show_creature_names: bool,
    pub(crate) mode: RuntimeMode,
    pub(crate) spawn_modal: Option<SpawnModal>,
    tick_rate: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct SpawnModal {
    pub selected: usize,
}

pub enum RuntimeMode {
    Tank(TankState),
    Reef(ReefState),
}

pub struct TankState {
    pub width: u16,
    pub height: u16,
}

pub struct ReefState {
    pub world: ReefWorld,
    pub respawn_delay: Duration,
    pub last_area: Rect,
    pub min_height: u16,
    pub scroll_enabled: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct WaterBand {
    pub top: i32,
    pub bottom: i32,
}

impl WaterBand {
    pub fn for_reef(world: &ReefWorld, terminal_height: u16) -> Self {
        Self {
            top: world.surface.height as i32,
            bottom: terminal_height.saturating_sub(world.floor.height) as i32,
        }
    }

    pub fn random_y_for(&self, variant: &Variant, rng: &mut ThreadRng) -> Option<i32> {
        let max_y = self.bottom - variant.height as i32;
        if max_y < self.top {
            None
        } else {
            Some(rng.random_range(self.top..=max_y))
        }
    }

    pub fn clamp_y_for(&self, y: i32, variant: &Variant) -> Option<i32> {
        let max_y = self.bottom - variant.height as i32;
        if max_y < self.top {
            None
        } else {
            Some(y.clamp(self.top, max_y))
        }
    }

    pub fn floor_y_for(&self, variant: &Variant) -> Option<i32> {
        let y = self.bottom - variant.height as i32;
        if y < self.top { None } else { Some(y) }
    }
}

impl App {
    pub fn new(
        config: AppConfig,
        definitions: Vec<CreatureDef>,
        launch_area: Rect,
    ) -> Result<Self> {
        let initial_count_scale = match config.mode {
            Mode::Reef => config.reef.creatures.count_scale,
            Mode::Tank => 1.0,
        };
        let mode = match config.mode {
            Mode::Tank => RuntimeMode::Tank(TankState {
                width: config.tank.width,
                height: config.tank.height,
            }),
            Mode::Reef => {
                let surface = load_world_layer(&config.reef.horizontal.surface)?;
                let floor = load_world_layer(&config.reef.horizontal.floor)?;
                let min_height = surface
                    .height
                    .saturating_add(floor.height)
                    .saturating_add(tallest_variant_height(&definitions));
                let world = ReefWorld::new(
                    surface,
                    floor,
                    launch_area.width,
                    config.reef.horizontal.offscreen_pages,
                );

                RuntimeMode::Reef(ReefState {
                    world,
                    respawn_delay: Duration::from_millis(config.reef.creatures.respawn_delay_ms),
                    last_area: launch_area,
                    min_height,
                    scroll_enabled: config.reef.horizontal.scroll_enabled
                        && !config.reef.vertical.scroll_enabled,
                })
            }
        };

        let mut app = Self {
            definitions,
            entities: Vec::new(),
            tick: 0,
            show_background: false,
            show_creature_names: false,
            mode,
            spawn_modal: None,
            tick_rate: DEFAULT_TICK_RATE,
        };
        app.spawn_initial_entities(launch_area, initial_count_scale);
        Ok(app)
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let mut last_tick = Instant::now();

        loop {
            terminal.draw(|frame| self.render(frame))?;

            let timeout = self.tick_rate.saturating_sub(last_tick.elapsed());
            if event::poll(timeout)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if self.handle_key(key) {
                            return Ok(());
                        }
                    }
                    Event::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollLeft => self.scroll_reef(-SCROLL_STEP),
                        MouseEventKind::ScrollRight => self.scroll_reef(SCROLL_STEP),
                        _ => {}
                    },
                    Event::Resize(width, height) => self.handle_resize(width, height),
                    _ => {}
                }
            }

            if last_tick.elapsed() >= self.tick_rate {
                self.tick();
                last_tick = Instant::now();
            }
        }
    }

    pub fn render(&self, frame: &mut Frame<'_>) {
        render::render(frame, self);
    }

    #[cfg(test)]
    fn min_height(&self) -> Option<u16> {
        match &self.mode {
            RuntimeMode::Tank(_) => None,
            RuntimeMode::Reef(reef) => Some(reef.min_height),
        }
    }

    fn spawn_initial_entities(&mut self, launch_area: Rect, count_scale: f64) {
        let mut rng = rand::rng();

        for def_index in 0..self.definitions.len() {
            let count = scaled_initial_count(self.definitions[def_index].count, count_scale);
            for copy_index in 0..count {
                let entity = match &self.mode {
                    RuntimeMode::Tank(tank) => {
                        spawn_tank_entity(&self.definitions, def_index, copy_index, tank, &mut rng)
                    }
                    RuntimeMode::Reef(reef) => spawn_reef_entity(
                        &self.definitions,
                        def_index,
                        copy_index,
                        &reef.world,
                        launch_area,
                        SpawnMode::Anywhere,
                        &mut rng,
                    ),
                };
                self.entities.push(entity);
            }
        }
    }

    fn tick(&mut self) {
        self.tick += 1;
        let mut rng = rand::rng();
        match &mut self.mode {
            RuntimeMode::Tank(tank) => {
                let bounds = Rect::new(0, 0, tank.width - 2, tank.height - 2);
                for entity in &mut self.entities {
                    let def = &self.definitions[entity.def];
                    entity.maybe_rearrange_school(def, &mut rng);
                    let variant = def.best_variant(entity.dx, self.tick, entity.phase);
                    entity.tick_bounded(def, bounds, variant, &mut rng);
                }
            }
            RuntimeMode::Reef(reef) => tick_reef(
                &self.definitions,
                &mut self.entities,
                self.tick,
                reef,
                &mut rng,
            ),
        }
    }

    fn scroll_reef(&mut self, delta: i32) {
        if let RuntimeMode::Reef(reef) = &mut self.mode
            && reef.scroll_enabled
        {
            reef.world.scroll_by(delta);
        }
    }

    fn handle_resize(&mut self, width: u16, height: u16) {
        let mut rng = rand::rng();
        if let RuntimeMode::Reef(reef) = &mut self.mode {
            reef.last_area = Rect::new(0, 0, width, height);
            rebind_creatures_to_reef(
                &self.definitions,
                &mut self.entities,
                &reef.world,
                reef.last_area,
                self.tick,
                &mut rng,
            );
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.spawn_modal.is_some() {
            self.handle_spawn_modal_key(key);
            return false;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('b') => self.show_background = !self.show_background,
            KeyCode::Char('t') => self.show_creature_names = !self.show_creature_names,
            KeyCode::Char('+') => self.spawn_random_creature(),
            KeyCode::Char('-') => self.despawn_random_creature(),
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_spawn_modal()
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => self.speed_up_animation(),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.slow_down_animation()
            }
            KeyCode::Left => self.scroll_reef(-SCROLL_STEP),
            KeyCode::Right => self.scroll_reef(SCROLL_STEP),
            _ => {}
        }

        false
    }

    fn handle_spawn_modal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.spawn_modal = None,
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.spawn_modal = None
            }
            KeyCode::Up => self.move_spawn_selection(-1),
            KeyCode::Down => self.move_spawn_selection(1),
            KeyCode::Enter => {
                if let Some(modal) = self.spawn_modal {
                    self.spawn_creature(modal.selected);
                }
            }
            _ => {}
        }
    }

    fn open_spawn_modal(&mut self) {
        if self.definitions.is_empty() {
            return;
        }

        self.spawn_modal = Some(SpawnModal { selected: 0 });
    }

    fn move_spawn_selection(&mut self, delta: isize) {
        let Some(modal) = &mut self.spawn_modal else {
            return;
        };
        let count = self.definitions.len();
        if count == 0 {
            modal.selected = 0;
            return;
        }

        modal.selected = modal
            .selected
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1));
    }

    fn spawn_random_creature(&mut self) {
        let spawnable = self
            .definitions
            .iter()
            .enumerate()
            .filter_map(|(index, definition)| (definition.count > 0).then_some(index))
            .collect::<Vec<_>>();
        if spawnable.is_empty() {
            return;
        }

        let mut rng = rand::rng();
        let def_index = spawnable[rng.random_range(0..spawnable.len())];
        self.spawn_creature(def_index);
    }

    fn spawn_creature(&mut self, def_index: usize) {
        if def_index >= self.definitions.len() {
            return;
        }

        let mut rng = rand::rng();
        let copy_index = self
            .entities
            .iter()
            .filter(|entity| entity.def == def_index)
            .count();
        let entity = match &self.mode {
            RuntimeMode::Tank(tank) => {
                spawn_tank_entity(&self.definitions, def_index, copy_index, tank, &mut rng)
            }
            RuntimeMode::Reef(reef) => spawn_reef_entity(
                &self.definitions,
                def_index,
                copy_index,
                &reef.world,
                reef.last_area,
                SpawnMode::Anywhere,
                &mut rng,
            ),
        };

        self.entities.push(entity);
    }

    fn despawn_random_creature(&mut self) {
        if self.entities.is_empty() {
            return;
        }

        let mut rng = rand::rng();
        let index = rng.random_range(0..self.entities.len());
        self.entities.swap_remove(index);
    }

    fn speed_up_animation(&mut self) {
        self.tick_rate = self
            .tick_rate
            .saturating_sub(TICK_RATE_STEP)
            .max(MIN_TICK_RATE);
    }

    fn slow_down_animation(&mut self) {
        self.tick_rate = self
            .tick_rate
            .saturating_add(TICK_RATE_STEP)
            .min(MAX_TICK_RATE);
    }
}

fn scaled_initial_count(count: usize, scale: f64) -> usize {
    if scale <= 0.0 {
        0
    } else {
        ((count as f64) * scale).round().min(usize::MAX as f64) as usize
    }
}

fn tick_reef(
    definitions: &[CreatureDef],
    entities: &mut [Entity],
    tick: u64,
    reef: &mut ReefState,
    rng: &mut ThreadRng,
) {
    if reef.last_area.height < reef.min_height {
        return;
    }

    let now = Instant::now();
    let bounds = reef.world.simulated_bounds(reef.last_area.width);
    let band = WaterBand::for_reef(&reef.world, reef.last_area.height);

    for (copy_index, entity) in entities.iter_mut().enumerate() {
        if let Some(respawn_at) = entity.respawn_at {
            if now < respawn_at {
                continue;
            }

            let replacement = spawn_reef_entity(
                definitions,
                entity.def,
                copy_index,
                &reef.world,
                reef.last_area,
                SpawnMode::Edge,
                rng,
            );
            entity.x = replacement.x;
            entity.y = replacement.y;
            entity.dx = replacement.dx;
            entity.dy = replacement.dy;
            entity.phase = replacement.phase;
            entity.pose_intent = replacement.pose_intent;
            entity.lateral_dx = replacement.lateral_dx;
            entity.depth_swim_ticks = replacement.depth_swim_ticks;
            entity.school_rearrangements = replacement.school_rearrangements;
            entity.respawn_at = None;
            continue;
        }

        let def = &definitions[entity.def];
        entity.maybe_rearrange_school(def, rng);
        if def.spawn_location == SpawnLocation::Floor {
            let variant = def.best_variant_for(0, PoseIntent::Lateral, tick, entity.phase);
            entity.dx = 0;
            entity.dy = 0;
            entity.pose_intent = PoseIntent::Lateral;
            entity.y = band.floor_y_for(variant).unwrap_or(band.top);
            continue;
        }

        update_reef_motion(def, entity, rng);

        entity.x += entity.dx as i32;
        entity.y += entity.dy as i32;

        let variant = def.best_variant_for(entity.dx, entity.pose_intent, tick, entity.phase);
        if let Some(clamped_y) = band.clamp_y_for(entity.y, variant)
            && clamped_y != entity.y
        {
            if def.four_way_swimmer && entity.depth_swim_ticks > 0 {
                entity.resume_lateral_motion();
            } else {
                entity.dy = if clamped_y <= band.top {
                    entity.dy.abs()
                } else {
                    -entity.dy.abs()
                };
            }
            entity.y = clamped_y;
        }

        if entity_exited(entity, variant, bounds) {
            entity.mark_exited(reef.respawn_delay, now);
        }
    }
}

fn update_reef_motion(def: &CreatureDef, entity: &mut Entity, rng: &mut ThreadRng) {
    if def.four_way_swimmer {
        update_four_way_swim(entity, rng);
    } else if def.brownian && rng.random_bool(0.25) {
        entity.dx = rng.random_range(-1..=1);
        entity.dy = rng.random_range(-1..=1);
    } else if def.uses_default_movement()
        && rng.random_bool(crate::creature::default_movement_transition_chance())
    {
        entity.toggle_vertical_motion(rng);
    }
}

fn update_four_way_swim(entity: &mut Entity, rng: &mut ThreadRng) {
    if entity.depth_swim_ticks > 0 {
        entity.depth_swim_ticks -= 1;
        entity.dx = 0;
        entity.dy = match entity.pose_intent {
            PoseIntent::FaceAway => -1,
            PoseIntent::Face => 1,
            PoseIntent::Lateral => 0,
        };

        if entity.depth_swim_ticks == 0 {
            entity.resume_lateral_motion();
        }
        return;
    }

    if entity.dx != 0 {
        entity.lateral_dx = entity.dx.signum();
    }
    if entity.lateral_dx == 0 {
        entity.lateral_dx = if rng.random_bool(0.5) { -1 } else { 1 };
    }

    entity.dx = entity.lateral_dx;
    entity.dy = 0;
    entity.pose_intent = PoseIntent::Lateral;

    if rng.random_bool(0.035) {
        let swim_towards = rng.random_bool(0.25);
        entity.pose_intent = if swim_towards {
            PoseIntent::Face
        } else {
            PoseIntent::FaceAway
        };
        entity.depth_swim_ticks = rng.random_range(6..=18);
        entity.dx = 0;
        entity.dy = if swim_towards { 1 } else { -1 };
    } else if rng.random_bool(0.02) {
        entity.lateral_dx = -entity.lateral_dx;
        entity.dx = entity.lateral_dx;
    }
}

fn entity_exited(entity: &Entity, variant: &Variant, bounds: WorldBounds) -> bool {
    entity.x + variant.width as i32 <= bounds.start || entity.x >= bounds.end
}

fn rebind_creatures_to_reef(
    definitions: &[CreatureDef],
    entities: &mut [Entity],
    world: &ReefWorld,
    area: Rect,
    tick: u64,
    rng: &mut ThreadRng,
) {
    let band = WaterBand::for_reef(world, area.height);
    for entity in entities {
        if !entity.is_active() {
            continue;
        }

        let def = &definitions[entity.def];
        let variant = def.best_variant_for(entity.dx, entity.pose_intent, tick, entity.phase);
        if def.spawn_location == SpawnLocation::Floor {
            if let Some(y) = band.floor_y_for(variant) {
                entity.y = y;
            }
            continue;
        }

        if band.clamp_y_for(entity.y, variant) != Some(entity.y)
            && let Some(y) = band.random_y_for(variant, rng)
        {
            entity.y = y;
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SpawnMode {
    Anywhere,
    Edge,
}

fn spawn_tank_entity(
    definitions: &[CreatureDef],
    def_index: usize,
    copy_index: usize,
    tank: &TankState,
    rng: &mut ThreadRng,
) -> Entity {
    let def = &definitions[def_index];
    let (dx, dy) = def.starting_velocity(rng);
    let variant = def.best_variant(dx, 0, def_index + copy_index);
    let max_x = tank
        .width
        .saturating_sub(2)
        .saturating_sub(variant.width)
        .max(1) as i32;
    let max_y = tank
        .height
        .saturating_sub(2)
        .saturating_sub(variant.height)
        .max(1) as i32;

    Entity {
        def: def_index,
        x: rng.random_range(0..=max_x),
        y: rng.random_range(0..=max_y),
        dx,
        dy,
        phase: rng.random_range(0..8),
        color: entity_color(definitions, def_index, copy_index, rng),
        respawn_at: None,
        pose_intent: PoseIntent::Lateral,
        lateral_dx: dx,
        depth_swim_ticks: 0,
        school_rearrangements: 0,
    }
}

fn spawn_reef_entity(
    definitions: &[CreatureDef],
    def_index: usize,
    copy_index: usize,
    world: &ReefWorld,
    area: Rect,
    mode: SpawnMode,
    rng: &mut ThreadRng,
) -> Entity {
    let def = &definitions[def_index];
    let (mut dx, dy) = def.starting_velocity(rng);
    if dx == 0 && def.spawn_location != SpawnLocation::Floor {
        dx = if rng.random_bool(0.5) { -1 } else { 1 };
    }

    let variant = def.best_variant(dx, 0, def_index + copy_index);
    let bounds = world.simulated_bounds(area.width);
    let max_x = bounds
        .end
        .saturating_sub(variant.width as i32)
        .max(bounds.start);
    let (x, dx) = match mode {
        SpawnMode::Anywhere => (rng.random_range(bounds.start..=max_x), dx),
        SpawnMode::Edge => {
            if rng.random_bool(0.5) {
                (bounds.start, dx.abs().max(1))
            } else {
                (max_x, -dx.abs().max(1))
            }
        }
    };

    let band = WaterBand::for_reef(world, area.height);
    let y = match def.spawn_location {
        SpawnLocation::Water => band.random_y_for(variant, rng).unwrap_or(band.top),
        SpawnLocation::Floor => band.floor_y_for(variant).unwrap_or(band.top),
    };
    let (dx, dy) = match def.spawn_location {
        SpawnLocation::Water => (dx, dy),
        SpawnLocation::Floor => (0, 0),
    };

    Entity {
        def: def_index,
        x,
        y,
        dx,
        dy,
        phase: rng.random_range(0..8),
        color: entity_color(definitions, def_index, copy_index, rng),
        respawn_at: None,
        pose_intent: PoseIntent::Lateral,
        lateral_dx: dx,
        depth_swim_ticks: 0,
        school_rearrangements: 0,
    }
}

fn entity_color(
    definitions: &[CreatureDef],
    def_index: usize,
    copy_index: usize,
    rng: &mut ThreadRng,
) -> Color {
    let def = &definitions[def_index];
    if !def.colors.is_empty() {
        return def.colors[rng.random_range(0..def.colors.len())];
    }

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
    let name_hash = def
        .name
        .bytes()
        .fold(0usize, |hash, byte| hash.wrapping_add(byte as usize));

    colors[(def_index + copy_index + name_hash) % colors.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::load_config, creature::load_creatures};

    #[test]
    fn reef_min_height_uses_tallest_creature_plus_layers() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        assert_eq!(
            app.min_height(),
            Some(
                app.definitions
                    .iter()
                    .flat_map(|definition| &definition.variants)
                    .map(|variant| variant.height)
                    .max()
                    .unwrap()
                    + 2
            )
        );
    }

    #[test]
    fn water_band_rejects_creatures_that_overlap_floor() {
        let variant = Variant {
            pose: "face".to_string(),
            art: vec!["xx".to_string(), "xx".to_string()],
            width: 2,
            height: 2,
            school: None,
        };
        let band = WaterBand { top: 1, bottom: 5 };

        assert_eq!(band.clamp_y_for(4, &variant), Some(3));
    }

    #[test]
    fn entity_exits_only_at_offscreen_bounds() {
        let variant = Variant {
            pose: "face".to_string(),
            art: vec!["xx".to_string()],
            width: 2,
            height: 1,
            school: None,
        };
        let bounds = WorldBounds {
            start: -10,
            end: 20,
        };
        let mut entity = Entity {
            def: 0,
            x: -9,
            y: 1,
            dx: -1,
            dy: 0,
            phase: 0,
            color: Color::White,
            respawn_at: None,
            pose_intent: PoseIntent::Lateral,
            lateral_dx: -1,
            depth_swim_ticks: 0,
            school_rearrangements: 0,
        };

        assert!(!entity_exited(&entity, &variant, bounds));
        entity.x = -12;
        assert!(entity_exited(&entity, &variant, bounds));
        entity.x = 19;
        assert!(!entity_exited(&entity, &variant, bounds));
        entity.x = 20;
        assert!(entity_exited(&entity, &variant, bounds));
    }

    #[test]
    fn four_way_depth_swim_pauses_lateral_motion_then_resumes() {
        let mut rng = rand::rng();
        let mut entity = Entity {
            def: 0,
            x: 0,
            y: 4,
            dx: 0,
            dy: 0,
            phase: 0,
            color: Color::White,
            respawn_at: None,
            pose_intent: PoseIntent::FaceAway,
            lateral_dx: -1,
            depth_swim_ticks: 2,
            school_rearrangements: 0,
        };

        update_four_way_swim(&mut entity, &mut rng);
        assert_eq!(entity.dx, 0);
        assert_eq!(entity.dy, -1);
        assert_eq!(entity.pose_intent, PoseIntent::FaceAway);
        assert_eq!(entity.depth_swim_ticks, 1);

        update_four_way_swim(&mut entity, &mut rng);
        assert_eq!(entity.dx, -1);
        assert_eq!(entity.dy, 0);
        assert_eq!(entity.pose_intent, PoseIntent::Lateral);
        assert_eq!(entity.depth_swim_ticks, 0);
    }

    #[test]
    fn floor_spawn_creatures_are_stationary_and_floor_bound() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");
        let def_index = app
            .definitions
            .iter()
            .position(|definition| definition.name == "wigglewort")
            .expect("wigglewort definition exists");
        let variant = app.definitions[def_index].best_variant(0, 0, 0);
        let expected_y = match &app.mode {
            RuntimeMode::Reef(reef) => WaterBand::for_reef(&reef.world, reef.last_area.height)
                .floor_y_for(variant)
                .expect("floor y fits"),
            RuntimeMode::Tank(_) => unreachable!("test config uses reef mode"),
        };

        for entity in app.entities.iter().filter(|entity| entity.def == def_index) {
            assert_eq!(entity.dx, 0);
            assert_eq!(entity.dy, 0);
            assert_eq!(entity.y, expected_y);
        }

        app.tick();

        for entity in app.entities.iter().filter(|entity| entity.def == def_index) {
            assert_eq!(entity.dx, 0);
            assert_eq!(entity.dy, 0);
            assert_eq!(entity.y, expected_y);
            assert_eq!(entity.respawn_at, None);
        }
    }

    #[test]
    fn spawn_random_creature_adds_one_entity() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");
        let before = app.entities.len();

        app.spawn_random_creature();

        assert_eq!(app.entities.len(), before + 1);
    }

    #[test]
    fn spawned_creatures_use_their_defined_colors() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");
        let bertrand = app
            .definitions
            .iter()
            .position(|definition| definition.name == "bertrand")
            .expect("bertrand definition exists");

        app.spawn_creature(bertrand);
        let entity = app
            .entities
            .iter()
            .find(|entity| entity.def == bertrand)
            .expect("bertrand spawned");

        assert!(app.definitions[bertrand].colors.contains(&entity.color));
    }

    #[test]
    fn initial_spawn_counts_use_config_count_scale() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let expected = definitions
            .iter()
            .map(|definition| {
                scaled_initial_count(definition.count, config.reef.creatures.count_scale)
            })
            .sum::<usize>();
        let app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        assert_eq!(app.entities.len(), expected);
    }

    #[test]
    fn zero_count_creatures_do_not_spawn_initially_but_can_be_spawned() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");
        let zero_count = zero_count_definition(&app);

        assert_eq!(
            app.entities
                .iter()
                .filter(|entity| entity.def == zero_count)
                .count(),
            0
        );

        app.spawn_creature(zero_count);

        assert_eq!(
            app.entities
                .iter()
                .filter(|entity| entity.def == zero_count)
                .count(),
            1
        );
    }

    #[test]
    fn random_spawn_excludes_zero_count_creatures() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");
        let zero_count = zero_count_definition(&app);

        for _ in 0..100 {
            app.spawn_random_creature();
        }

        assert_eq!(
            app.entities
                .iter()
                .filter(|entity| entity.def == zero_count)
                .count(),
            0
        );
    }

    #[test]
    fn despawn_random_creature_removes_one_entity() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");
        let before = app.entities.len();

        app.despawn_random_creature();

        assert_eq!(app.entities.len(), before - 1);
    }

    #[test]
    fn ctrl_s_modal_spawns_selected_creature_on_enter() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(app.spawn_modal.map(|modal| modal.selected), Some(0));

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.spawn_modal.map(|modal| modal.selected), Some(1));

        let before_total = app.entities.len();
        let before_selected = app.entities.iter().filter(|entity| entity.def == 1).count();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.spawn_modal.map(|modal| modal.selected), Some(1));
        assert_eq!(app.entities.len(), before_total + 1);
        assert_eq!(
            app.entities.iter().filter(|entity| entity.def == 1).count(),
            before_selected + 1
        );
    }

    #[test]
    fn t_toggles_creature_names() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        assert!(!app.show_creature_names);

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(app.show_creature_names);

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(!app.show_creature_names);
    }

    #[test]
    fn shift_up_and_down_adjust_animation_speed() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        assert_eq!(app.tick_rate, DEFAULT_TICK_RATE);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        assert_eq!(app.tick_rate, DEFAULT_TICK_RATE - TICK_RATE_STEP);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        assert_eq!(app.tick_rate, DEFAULT_TICK_RATE);

        for _ in 0..20 {
            app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        }
        assert_eq!(app.tick_rate, MIN_TICK_RATE);

        for _ in 0..200 {
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        }
        assert_eq!(app.tick_rate, MAX_TICK_RATE);
    }

    fn zero_count_definition(app: &App) -> usize {
        app.definitions
            .iter()
            .position(|definition| definition.count == 0)
            .expect("at least one count=0 creature definition exists")
    }
}
