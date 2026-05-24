use std::time::{Duration, Instant};

use color_eyre::eyre::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use rand::{RngExt, rngs::ThreadRng};
use ratatui::{DefaultTerminal, Frame, layout::Rect, style::Color};

use crate::{
    config::AppConfig,
    creature::{
        ActivityState, CreatureDef, CreaturePreferences, Entity, PoseIntent, Territory, Variant,
        tallest_variant_height,
    },
    render,
    world::{ReefWorld, WorldBounds, load_world_layer},
};

const DEFAULT_TICK_RATE: Duration = Duration::from_millis(220);
const MIN_TICK_RATE: Duration = Duration::from_millis(10);
const MAX_TICK_RATE: Duration = Duration::from_millis(3000);
const TICK_RATE_STEP: Duration = Duration::from_millis(20);
const SCROLL_STEP: i32 = 4;
const MIN_SPAWN_DEPTH_JITTER: f64 = 0.12;
const MAX_SPAWN_DEPTH_JITTER: f64 = 0.35;

pub struct App {
    pub(crate) definitions: Vec<CreatureDef>,
    pub(crate) entities: Vec<Entity>,
    pub(crate) tick: u64,
    pub(crate) show_background: bool,
    pub(crate) show_creature_names: bool,
    pub(crate) reef: ReefState,
    pub(crate) spawn_modal: Option<SpawnModal>,
    pub(crate) show_help: bool,
    creature_color_mode: CreatureColorMode,
    tick_rate: Duration,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CreatureColorMode {
    Ansi16,
    #[default]
    Indexed256,
    TrueColor,
}

#[derive(Debug, Clone)]
pub struct SpawnModal {
    pub selected: usize,
    pub order: Vec<usize>,
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

    pub fn y_bounds_for(&self, variant: &Variant) -> Option<(i32, i32)> {
        let max_y = self.bottom - variant.height as i32;
        if max_y < self.top {
            None
        } else {
            Some((self.top, max_y))
        }
    }

    pub fn floor_y_for(&self, variant: &Variant) -> Option<i32> {
        let y = self.bottom - variant.height as i32;
        if y < self.top { None } else { Some(y) }
    }
}

impl App {
    #[cfg(test)]
    pub fn new(
        config: AppConfig,
        definitions: Vec<CreatureDef>,
        launch_area: Rect,
    ) -> Result<Self> {
        Self::new_with_color_mode(
            config,
            definitions,
            launch_area,
            CreatureColorMode::default(),
        )
    }

    pub fn new_with_color_mode(
        config: AppConfig,
        definitions: Vec<CreatureDef>,
        launch_area: Rect,
        creature_color_mode: CreatureColorMode,
    ) -> Result<Self> {
        let initial_count_scale = config.reef.creatures.count_scale;
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
        let reef = ReefState {
            world,
            respawn_delay: Duration::from_millis(config.reef.creatures.respawn_delay_ms),
            last_area: launch_area,
            min_height,
            scroll_enabled: config.reef.horizontal.scroll_enabled
                && !config.reef.vertical.scroll_enabled,
        };

        let mut app = Self {
            definitions,
            entities: Vec::new(),
            tick: 0,
            show_background: false,
            show_creature_names: false,
            reef,
            spawn_modal: None,
            show_help: false,
            creature_color_mode,
            tick_rate: DEFAULT_TICK_RATE,
        };
        app.spawn_initial_entities(initial_count_scale);
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
    fn min_height(&self) -> u16 {
        self.reef.min_height
    }

    fn spawn_initial_entities(&mut self, count_scale: f64) {
        let mut rng = rand::rng();

        for def_index in 0..self.definitions.len() {
            let count = scaled_initial_count(self.definitions[def_index].count, count_scale);
            for copy_index in 0..count {
                let entity = spawn_reef_entity(
                    ColorSelection {
                        definitions: &self.definitions,
                        mode: self.creature_color_mode,
                    },
                    def_index,
                    copy_index,
                    &self.reef,
                    SpawnMode::Anywhere,
                    self.tick,
                    &mut rng,
                );
                self.entities.push(entity);
            }
        }
    }

    fn tick(&mut self) {
        self.tick += 1;
        let mut rng = rand::rng();
        tick_reef(
            &self.definitions,
            &mut self.entities,
            self.tick,
            &mut self.reef,
            self.creature_color_mode,
            &mut rng,
        );
    }

    fn scroll_reef(&mut self, delta: i32) {
        if self.reef.scroll_enabled {
            self.reef.world.scroll_by(delta);
        }
    }

    fn handle_resize(&mut self, width: u16, height: u16) {
        let mut rng = rand::rng();
        self.reef.last_area = Rect::new(0, 0, width, height);
        rebind_creatures_to_reef(
            &self.definitions,
            &mut self.entities,
            &self.reef,
            self.tick,
            &mut rng,
        );
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.show_help {
            return self.handle_help_modal_key(key);
        }

        if self.spawn_modal.is_some() {
            self.handle_spawn_modal_key(key);
            return false;
        }

        self.handle_runtime_key(key)
    }

    fn handle_runtime_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('?') => self.show_help = true,
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

    fn handle_help_modal_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc | KeyCode::Char('?') => self.show_help = false,
            KeyCode::Char('b')
            | KeyCode::Char('t')
            | KeyCode::Char('+')
            | KeyCode::Char('-')
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down => {
                self.handle_runtime_key(key);
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.show_help = false;
                self.open_spawn_modal();
            }
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
                if let Some(def_index) = self
                    .spawn_modal
                    .as_ref()
                    .and_then(|modal| modal.order.get(modal.selected))
                    .copied()
                {
                    self.spawn_creature(def_index);
                }
            }
            _ => {}
        }
    }

    fn open_spawn_modal(&mut self) {
        if self.definitions.is_empty() {
            return;
        }

        let mut order = (0..self.definitions.len()).collect::<Vec<_>>();
        order.sort_by_key(|def_index| self.spawned_count(*def_index));
        self.spawn_modal = Some(SpawnModal { selected: 0, order });
    }

    fn move_spawn_selection(&mut self, delta: isize) {
        let Some(modal) = &mut self.spawn_modal else {
            return;
        };
        let count = modal.order.len();
        if count == 0 {
            modal.selected = 0;
            return;
        }

        modal.selected = modal
            .selected
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1));
    }

    fn spawned_count(&self, def_index: usize) -> usize {
        self.entities
            .iter()
            .filter(|entity| entity.def == def_index)
            .count()
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
        let entity = spawn_reef_entity(
            ColorSelection {
                definitions: &self.definitions,
                mode: self.creature_color_mode,
            },
            def_index,
            copy_index,
            &self.reef,
            SpawnMode::Anywhere,
            self.tick,
            &mut rng,
        );

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
    creature_color_mode: CreatureColorMode,
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
                ColorSelection {
                    definitions,
                    mode: creature_color_mode,
                },
                entity.def,
                copy_index,
                reef,
                SpawnMode::Edge,
                tick,
                rng,
            );
            entity.x = replacement.x;
            entity.y = replacement.y;
            entity.dx = replacement.dx;
            entity.dy = replacement.dy;
            entity.animation_frame_tick = replacement.animation_frame_tick;
            entity.phase = replacement.phase;
            entity.pose_intent = replacement.pose_intent;
            entity.lateral_dx = replacement.lateral_dx;
            entity.depth_swim_ticks = replacement.depth_swim_ticks;
            entity.school_rearrangements = replacement.school_rearrangements;
            entity.activity = replacement.activity;
            entity.activity_ticks = replacement.activity_ticks;
            entity.idle_move_chance = replacement.idle_move_chance;
            entity.idle_turn_chance = replacement.idle_turn_chance;
            entity.territory = replacement.territory;
            entity.respawn_at = None;
            continue;
        }

        let def = &definitions[entity.def];
        entity.advance_animation(def, rng);
        entity.maybe_rearrange_school(def, rng);
        if def.is_floor_bound() {
            let variant = def.best_variant_for(
                0,
                PoseIntent::Lateral,
                entity.animation_tick_for(def, entity.animation_frame_tick),
                entity.phase,
            );
            entity.dx = 0;
            entity.dy = 0;
            entity.activity = ActivityState::Idle;
            entity.pose_intent = PoseIntent::Lateral;
            entity.y = band.floor_y_for(variant).unwrap_or(band.top);
            continue;
        }

        let motion_variant = def.best_variant_for(
            entity.pose_dx_for(def),
            entity.pose_intent,
            entity.animation_tick_for(def, entity.animation_frame_tick),
            entity.phase,
        );
        update_reef_motion(def, entity, &band, motion_variant, tick, rng);

        entity.x += entity.dx as i32;
        entity.y += entity.dy as i32;

        let variant = def.best_variant_for(
            entity.pose_dx_for(def),
            entity.pose_intent,
            entity.animation_tick_for(def, entity.animation_frame_tick),
            entity.phase,
        );
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

fn update_reef_motion(
    def: &CreatureDef,
    entity: &mut Entity,
    band: &WaterBand,
    variant: &Variant,
    tick: u64,
    rng: &mut ThreadRng,
) {
    let was_idle = entity.activity == ActivityState::Idle;
    entity.advance_activity(def, rng);
    if entity.activity == ActivityState::Idle {
        entity.update_idle_motion(tick, rng);
        return;
    }
    if was_idle && entity.dx == 0 {
        entity.resume_lateral_motion();
    }

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

    apply_depth_bias(def, entity, band, variant, rng);
    apply_territory_bias(def, entity, rng);
}

fn apply_depth_bias(
    def: &CreatureDef,
    entity: &mut Entity,
    band: &WaterBand,
    variant: &Variant,
    rng: &mut ThreadRng,
) {
    if def.four_way_swimmer || def.is_floor_bound() {
        return;
    }
    let Some((min_y, max_y)) = band.y_bounds_for(variant) else {
        return;
    };
    if min_y >= max_y {
        return;
    }

    let preferences = &def.preferences;
    let target = preferred_depth_target(preferences);

    let target_y = min_y + ((max_y - min_y) as f64 * target).round() as i32;
    let distance = target_y - entity.y;
    if distance.abs() <= 1 {
        if rng.random_bool((preferences.sedentary * 0.15).clamp(0.0, 0.5)) {
            entity.dy = 0;
        }
        return;
    }

    let preference_strength =
        (depth_preference_strength(preferences) * 0.35 + 0.08).clamp(0.0, 0.6);
    if rng.random_bool(preference_strength) {
        entity.dy = distance.signum() as i16;
    }
}

fn preferred_depth_target(preferences: &CreaturePreferences) -> f64 {
    let mut target = preferences.depth;
    target += preferences.demersal * (1.0 - target) * 0.4;
    target -= preferences.reefer * target * 0.25;
    target.clamp(0.0, 1.0)
}

fn depth_preference_strength(preferences: &CreaturePreferences) -> f64 {
    preferences
        .demersal
        .max(preferences.reefer)
        .max((preferences.depth - 0.5).abs() * 2.0)
        .clamp(0.0, 1.0)
}

fn preferred_spawn_y_for(
    preferences: &CreaturePreferences,
    band: &WaterBand,
    variant: &Variant,
    rng: &mut ThreadRng,
) -> Option<i32> {
    let (min_y, max_y) = band.y_bounds_for(variant)?;
    let (min_y, max_y) = preferred_spawn_y_range(preferences, min_y, max_y);
    Some(rng.random_range(min_y..=max_y))
}

fn preferred_spawn_y_range(
    preferences: &CreaturePreferences,
    min_y: i32,
    max_y: i32,
) -> (i32, i32) {
    if min_y >= max_y {
        return (min_y, max_y);
    }

    let target = preferred_depth_target(preferences);
    let span = max_y - min_y;
    let target_y = min_y + (span as f64 * target).round() as i32;
    let jitter_fraction = MAX_SPAWN_DEPTH_JITTER
        - (MAX_SPAWN_DEPTH_JITTER - MIN_SPAWN_DEPTH_JITTER)
            * depth_preference_strength(preferences);
    let jitter = (span as f64 * jitter_fraction).round() as i32;

    (
        target_y.saturating_sub(jitter).clamp(min_y, max_y),
        target_y.saturating_add(jitter).clamp(min_y, max_y),
    )
}

fn apply_territory_bias(def: &CreatureDef, entity: &mut Entity, rng: &mut ThreadRng) {
    let Some(territory) = entity.territory else {
        return;
    };
    let territorial = def.preferences.territorial;
    if territorial <= 0.0 {
        return;
    }

    if entity.x < territory.min_x {
        entity.dx = 1;
    } else if entity.x > territory.max_x {
        entity.dx = -1;
    } else if rng.random_bool((territorial * 0.12).clamp(0.0, 0.75)) {
        if entity.dx > 0 && entity.x >= territory.max_x {
            entity.dx = -1;
        } else if entity.dx < 0 && entity.x <= territory.min_x {
            entity.dx = 1;
        }
    }

    if entity.y < territory.min_y {
        entity.dy = 1;
    } else if entity.y > territory.max_y {
        entity.dy = -1;
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
    reef: &ReefState,
    _tick: u64,
    rng: &mut ThreadRng,
) {
    let band = WaterBand::for_reef(&reef.world, reef.last_area.height);
    for entity in entities {
        if !entity.is_active() {
            continue;
        }

        let def = &definitions[entity.def];
        let variant = def.best_variant_for(
            entity.pose_dx_for(def),
            entity.pose_intent,
            entity.animation_tick_for(def, entity.animation_frame_tick),
            entity.phase,
        );
        if def.is_floor_bound() {
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

#[derive(Debug, Clone, Copy)]
struct ColorSelection<'a> {
    definitions: &'a [CreatureDef],
    mode: CreatureColorMode,
}

fn spawn_reef_entity(
    colors: ColorSelection<'_>,
    def_index: usize,
    copy_index: usize,
    reef: &ReefState,
    mode: SpawnMode,
    animation_frame_tick: u64,
    rng: &mut ThreadRng,
) -> Entity {
    let def = &colors.definitions[def_index];
    let (mut dx, dy) = def.starting_velocity(rng);
    if dx == 0 && !def.is_floor_bound() {
        dx = if rng.random_bool(0.5) { -1 } else { 1 };
    }

    let variant = def.best_variant(dx, 0, def_index + copy_index);
    let bounds = reef.world.simulated_bounds(reef.last_area.width);
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

    let band = WaterBand::for_reef(&reef.world, reef.last_area.height);
    let y = if def.is_floor_bound() {
        band.floor_y_for(variant).unwrap_or(band.top)
    } else {
        preferred_spawn_y_for(&def.preferences, &band, variant, rng).unwrap_or(band.top)
    };
    let (dx, dy) = if def.is_floor_bound() {
        (0, 0)
    } else {
        (dx, dy)
    };
    let (activity, activity_ticks) = def.initial_activity(rng);
    let (min_y, max_y) = band.y_bounds_for(variant).unwrap_or((band.top, band.top));
    let territory = assign_territory(
        def,
        x,
        y,
        TerritoryBounds {
            min_x: bounds.start,
            max_x,
            min_y,
            max_y,
        },
        rng,
    );

    Entity {
        def: def_index,
        x,
        y,
        dx,
        dy,
        animation_frame_tick,
        phase: rng.random_range(0..8),
        color: entity_color(colors, def_index, rng),
        respawn_at: None,
        pose_intent: PoseIntent::Lateral,
        lateral_dx: dx,
        depth_swim_ticks: 0,
        school_rearrangements: 0,
        activity,
        activity_ticks,
        idle_move_chance: crate::creature::DEFAULT_IDLE_MOVE_CHANCE,
        idle_turn_chance: crate::creature::DEFAULT_IDLE_TURN_CHANCE,
        territory,
    }
}

fn assign_territory(
    def: &CreatureDef,
    x: i32,
    y: i32,
    bounds: TerritoryBounds,
    rng: &mut ThreadRng,
) -> Option<Territory> {
    let geometry = def.preferences.territory_geometry.as_ref()?;
    let (width, height) = geometry.sample_size(rng);
    Some(Territory {
        min_x: anchored_min(x, width.max(1) as i32, bounds.min_x, bounds.max_x),
        max_x: anchored_max(x, width.max(1) as i32, bounds.min_x, bounds.max_x),
        min_y: anchored_min(y, height.max(1) as i32, bounds.min_y, bounds.max_y),
        max_y: anchored_max(y, height.max(1) as i32, bounds.min_y, bounds.max_y),
    })
}

#[derive(Debug, Clone, Copy)]
struct TerritoryBounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

fn anchored_min(center: i32, size: i32, min: i32, max: i32) -> i32 {
    if min >= max {
        return min;
    }
    let size = size.min(max - min + 1).max(1);
    let start = center - size / 2;
    start.clamp(min, max - size + 1)
}

fn anchored_max(center: i32, size: i32, min: i32, max: i32) -> i32 {
    let start = anchored_min(center, size, min, max);
    start + size.min(max.saturating_sub(min) + 1).max(1) - 1
}

fn entity_color(colors: ColorSelection<'_>, def_index: usize, rng: &mut ThreadRng) -> Color {
    let def = &colors.definitions[def_index];
    if !def.colors.is_empty() {
        let encoded_colors = def
            .colors
            .iter()
            .filter_map(|color| colors.mode.encode_kdl_color(*color))
            .collect::<Vec<_>>();
        if !encoded_colors.is_empty() {
            return encoded_colors[rng.random_range(0..encoded_colors.len())];
        }
    }

    colors.mode.random_global_color(rng)
}

impl CreatureColorMode {
    fn random_global_color(self, rng: &mut ThreadRng) -> Color {
        match self {
            Self::Ansi16 => ANSI_16_COLORS[rng.random_range(0..ANSI_16_COLORS.len())],
            Self::Indexed256 => Color::Indexed(rng.random_range(0..=u8::MAX)),
            Self::TrueColor => Color::Rgb(rng.random(), rng.random(), rng.random()),
        }
    }

    fn encode_kdl_color(self, color: Color) -> Option<Color> {
        match self {
            Self::Ansi16 => ansi16_index(color).map(|index| ANSI_16_COLORS[index as usize]),
            Self::Indexed256 => match color {
                Color::Rgb(red, green, blue) => {
                    Some(Color::Indexed(nearest_256_color_index(red, green, blue)))
                }
                Color::Indexed(index) => Some(Color::Indexed(index)),
                _ => ansi16_index(color).map(Color::Indexed),
            },
            Self::TrueColor => match color {
                Color::Rgb(red, green, blue) => Some(Color::Rgb(red, green, blue)),
                Color::Indexed(index) => {
                    let (red, green, blue) = indexed_color_rgb(index);
                    Some(Color::Rgb(red, green, blue))
                }
                _ => ansi16_index(color).map(|index| {
                    let (red, green, blue) = indexed_color_rgb(index);
                    Color::Rgb(red, green, blue)
                }),
            },
        }
    }
}

const ANSI_16_COLORS: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

fn ansi16_index(color: Color) -> Option<u8> {
    match color {
        Color::Black => Some(0),
        Color::Red => Some(1),
        Color::Green => Some(2),
        Color::Yellow => Some(3),
        Color::Blue => Some(4),
        Color::Magenta => Some(5),
        Color::Cyan => Some(6),
        Color::Gray => Some(7),
        Color::DarkGray => Some(8),
        Color::LightRed => Some(9),
        Color::LightGreen => Some(10),
        Color::LightYellow => Some(11),
        Color::LightBlue => Some(12),
        Color::LightMagenta => Some(13),
        Color::LightCyan => Some(14),
        Color::White => Some(15),
        Color::Indexed(index) if index < 16 => Some(index),
        _ => None,
    }
}

fn nearest_256_color_index(red: u8, green: u8, blue: u8) -> u8 {
    (0..=u8::MAX)
        .min_by_key(|index| {
            let (candidate_red, candidate_green, candidate_blue) = indexed_color_rgb(*index);
            color_distance_squared(
                (red, green, blue),
                (candidate_red, candidate_green, candidate_blue),
            )
        })
        .expect("the 256-color palette is non-empty")
}

fn color_distance_squared(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let red = i32::from(a.0) - i32::from(b.0);
    let green = i32::from(a.1) - i32::from(b.1);
    let blue = i32::from(a.2) - i32::from(b.2);
    (red * red + green * green + blue * blue) as u32
}

fn indexed_color_rgb(index: u8) -> (u8, u8, u8) {
    const ANSI_RGB: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

    match index {
        0..=15 => ANSI_RGB[index as usize],
        16..=231 => {
            let offset = index - 16;
            let red = CUBE_LEVELS[(offset / 36) as usize];
            let green = CUBE_LEVELS[((offset % 36) / 6) as usize];
            let blue = CUBE_LEVELS[(offset % 6) as usize];
            (red, green, blue)
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            (value, value, value)
        }
    }
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
            app.definitions
                .iter()
                .flat_map(|definition| &definition.variants)
                .map(|variant| variant.height)
                .max()
                .unwrap()
                + 2
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
    fn preferred_spawn_depth_range_centers_near_target_with_jitter() {
        let preferences = CreaturePreferences {
            demersal: 0.8,
            depth: 0.9,
            ..CreaturePreferences::default()
        };

        let target = preferred_depth_target(&preferences);
        let (min_y, max_y) = preferred_spawn_y_range(&preferences, 0, 100);

        assert!((target - 0.932).abs() < f64::EPSILON);
        assert_eq!((min_y, max_y), (76, 100));
    }

    #[test]
    fn neutral_spawn_depth_range_keeps_broad_vertical_variation() {
        let preferences = CreaturePreferences::default();

        assert_eq!(preferred_spawn_y_range(&preferences, 0, 100), (15, 85));
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
            animation_frame_tick: 0,
            phase: 0,
            color: Color::White,
            respawn_at: None,
            pose_intent: PoseIntent::Lateral,
            lateral_dx: -1,
            depth_swim_ticks: 0,
            school_rearrangements: 0,
            activity: ActivityState::Active,
            activity_ticks: 1,
            idle_move_chance: crate::creature::DEFAULT_IDLE_MOVE_CHANCE,
            idle_turn_chance: crate::creature::DEFAULT_IDLE_TURN_CHANCE,
            territory: None,
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
            animation_frame_tick: 0,
            phase: 0,
            color: Color::White,
            respawn_at: None,
            pose_intent: PoseIntent::FaceAway,
            lateral_dx: -1,
            depth_swim_ticks: 2,
            school_rearrangements: 0,
            activity: ActivityState::Active,
            activity_ticks: 1,
            idle_move_chance: crate::creature::DEFAULT_IDLE_MOVE_CHANCE,
            idle_turn_chance: crate::creature::DEFAULT_IDLE_TURN_CHANCE,
            territory: None,
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
        let expected_y = WaterBand::for_reef(&app.reef.world, app.reef.last_area.height)
            .floor_y_for(variant)
            .expect("floor y fits");

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
    fn spawned_creatures_use_their_defined_colors_in_truecolor_mode() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new_with_color_mode(
            config,
            definitions,
            Rect::new(0, 0, 120, 40),
            CreatureColorMode::TrueColor,
        )
        .expect("app starts");
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
    fn custom_rgb_creature_colors_snap_to_256_color_mode() {
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let bertrand = definitions
            .iter()
            .position(|definition| definition.name == "bertrand")
            .expect("bertrand definition exists");
        let mut rng = rand::rng();

        let color = entity_color(
            ColorSelection {
                definitions: &definitions,
                mode: CreatureColorMode::Indexed256,
            },
            bertrand,
            &mut rng,
        );

        assert!(matches!(color, Color::Indexed(_)));
    }

    #[test]
    fn color_modes_generate_requested_global_color_classes() {
        let mut rng = rand::rng();

        assert!(ANSI_16_COLORS.contains(&CreatureColorMode::Ansi16.random_global_color(&mut rng)));
        assert!(matches!(
            CreatureColorMode::Indexed256.random_global_color(&mut rng),
            Color::Indexed(_)
        ));
        assert!(matches!(
            CreatureColorMode::TrueColor.random_global_color(&mut rng),
            Color::Rgb(_, _, _)
        ));
    }

    #[test]
    fn ansi16_mode_uses_global_pool_for_rgb_only_creature_colors() {
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let bertrand = definitions
            .iter()
            .position(|definition| definition.name == "bertrand")
            .expect("bertrand definition exists");
        let mut rng = rand::rng();

        let color = entity_color(
            ColorSelection {
                definitions: &definitions,
                mode: CreatureColorMode::Ansi16,
            },
            bertrand,
            &mut rng,
        );

        assert!(ANSI_16_COLORS.contains(&color));
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
        assert_eq!(
            app.spawn_modal.as_ref().map(|modal| modal.selected),
            Some(0)
        );

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.spawn_modal.as_ref().map(|modal| modal.selected),
            Some(1)
        );

        let selected_def = app
            .spawn_modal
            .as_ref()
            .and_then(|modal| modal.order.get(modal.selected))
            .copied()
            .expect("selected creature");
        let before_total = app.entities.len();
        let before_selected = app.spawned_count(selected_def);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(
            app.spawn_modal.as_ref().map(|modal| modal.selected),
            Some(1)
        );
        assert_eq!(app.entities.len(), before_total + 1);
        assert_eq!(app.spawned_count(selected_def), before_selected + 1);
    }

    #[test]
    fn spawn_modal_order_is_sorted_by_spawned_count_when_opened() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        app.open_spawn_modal();

        let order = app
            .spawn_modal
            .as_ref()
            .map(|modal| modal.order.clone())
            .expect("spawn modal opens");
        let counts = order
            .iter()
            .map(|def_index| app.spawned_count(*def_index))
            .collect::<Vec<_>>();
        assert!(counts.windows(2).all(|window| window[0] <= window[1]));
    }

    #[test]
    fn spawn_modal_order_does_not_resort_after_spawning() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        app.open_spawn_modal();
        let order_before = app
            .spawn_modal
            .as_ref()
            .map(|modal| modal.order.clone())
            .expect("spawn modal opens");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(
            app.spawn_modal.as_ref().map(|modal| &modal.order),
            Some(&order_before)
        );
    }

    #[test]
    fn question_mark_opens_and_closes_help_modal() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        assert!(!app.show_help);

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.show_help);

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(!app.show_help);
    }

    #[test]
    fn esc_closes_help_modal() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));

        assert!(!app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(!app.show_help);
    }

    #[test]
    fn q_still_quits_from_help_modal() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));

        assert!(app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    }

    #[test]
    fn help_modal_passes_through_runtime_shortcuts() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");
        let initial_entities = app.entities.len();
        let initial_viewport_x = reef_viewport_x(&app);

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));

        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert!(app.show_background);

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(app.show_creature_names);

        app.handle_key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE));
        assert_eq!(app.entities.len(), initial_entities + 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE));
        assert_eq!(app.entities.len(), initial_entities);

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(reef_viewport_x(&app), initial_viewport_x + SCROLL_STEP);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(reef_viewport_x(&app), initial_viewport_x);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        assert_eq!(app.tick_rate, DEFAULT_TICK_RATE - TICK_RATE_STEP);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        assert_eq!(app.tick_rate, DEFAULT_TICK_RATE);

        assert!(app.show_help);
    }

    #[test]
    fn ctrl_s_opens_spawn_modal_from_help_modal() {
        let config = load_config("config.kdl".as_ref()).expect("config loads");
        let definitions = load_creatures("art/creatures".as_ref()).expect("creatures load");
        let mut app = App::new(config, definitions, Rect::new(0, 0, 120, 40)).expect("app starts");

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert!(!app.show_help);
        assert!(app.spawn_modal.is_some());
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

    fn reef_viewport_x(app: &App) -> i32 {
        app.reef.world.viewport_x
    }
}
