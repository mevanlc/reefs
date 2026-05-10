use std::{
    fs,
    path::Path,
    str::FromStr,
    time::{Duration, Instant},
};

use color_eyre::{
    Section, SectionExt,
    eyre::{Result, WrapErr, eyre},
};
use kdl::{KdlDocument, KdlValue};
use rand::{RngExt, rngs::ThreadRng};
use ratatui::{layout::Rect, style::Color};

#[derive(Debug, Clone)]
pub struct CreatureDef {
    pub name: String,
    pub variants: Vec<Variant>,
    pub count: usize,
    pub four_way_swimmer: bool,
    pub spawn_location: SpawnLocation,
    h_velocity: Option<i16>,
    v_velocity: Option<i16>,
    pub brownian: bool,
    pub colors: Vec<Color>,
    default_movement: bool,
    school_rearrange_chance: Option<f64>,
}

impl CreatureDef {
    pub fn best_variant(&self, dx: i16, tick: u64, phase: usize) -> &Variant {
        self.best_variant_for(dx, PoseIntent::Lateral, tick, phase)
    }

    pub fn best_variant_for(
        &self,
        dx: i16,
        pose_intent: PoseIntent,
        tick: u64,
        phase: usize,
    ) -> &Variant {
        let wanted = match pose_intent {
            PoseIntent::Face => "face",
            PoseIntent::FaceAway => "face-away",
            PoseIntent::Lateral => {
                if dx < 0 {
                    "left"
                } else if dx > 0 {
                    "right"
                } else {
                    "face"
                }
            }
        };

        let matching = self
            .variants
            .iter()
            .filter(|variant| pose_matches(&variant.pose, wanted))
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

    pub fn starting_velocity(&self, rng: &mut ThreadRng) -> (i16, i16) {
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

    pub fn uses_default_movement(&self) -> bool {
        self.default_movement
    }

    pub fn school_rearrange_chance(&self) -> Option<f64> {
        self.school_rearrange_chance
    }
}

pub fn default_movement_transition_chance() -> f64 {
    1.0 - 0.5_f64.powf(1.0 / 200.0)
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub pose: String,
    pub art: Vec<String>,
    pub width: u16,
    pub height: u16,
    pub school: Option<School>,
}

#[derive(Debug, Clone)]
pub struct School {
    pub unit: String,
    pub units: Vec<SchoolUnit>,
}

#[derive(Debug, Clone, Copy)]
pub struct SchoolUnit {
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoseIntent {
    Lateral,
    Face,
    FaceAway,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnLocation {
    Water,
    Floor,
}

impl Variant {
    fn from_kdl_node(pose: String, art: &str, unit: Option<&str>, unit_brownian: bool) -> Self {
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
            school: unit
                .filter(|unit| unit_brownian && !unit.is_empty())
                .map(|unit| School::from_art(unit, &art)),
            art,
            width,
            height,
        }
    }
}

impl School {
    fn from_art(unit: &str, art: &[String]) -> Self {
        let units = art
            .iter()
            .enumerate()
            .flat_map(|(row, line)| unit_positions(line, unit).map(move |column| (column, row)))
            .filter_map(|(column, row)| {
                let x = u16::try_from(column).ok()?;
                let y = u16::try_from(row).ok()?;
                Some(SchoolUnit { x, y })
            })
            .collect();

        Self {
            unit: unit.to_string(),
            units,
        }
    }
}

fn unit_positions<'a>(line: &'a str, unit: &'a str) -> impl Iterator<Item = usize> + 'a {
    line.match_indices(unit)
        .map(|(byte_index, _)| line[..byte_index].chars().count())
}

#[derive(Debug)]
pub struct Entity {
    pub def: usize,
    pub x: i32,
    pub y: i32,
    pub dx: i16,
    pub dy: i16,
    pub phase: usize,
    pub color: Color,
    pub respawn_at: Option<Instant>,
    pub pose_intent: PoseIntent,
    pub lateral_dx: i16,
    pub depth_swim_ticks: u8,
    pub school_rearrangements: u64,
}

impl Entity {
    pub fn tick_bounded(
        &mut self,
        def: &CreatureDef,
        bounds: Rect,
        variant: &Variant,
        rng: &mut ThreadRng,
    ) {
        if def.brownian && rng.random_bool(0.25) {
            self.dx = rng.random_range(-1..=1);
            self.dy = rng.random_range(-1..=1);
        } else if def.uses_default_movement()
            && rng.random_bool(default_movement_transition_chance())
        {
            self.toggle_vertical_motion(rng);
        }

        let max_x = (bounds.width.saturating_sub(variant.width).max(1) - 1) as i32;
        let max_y = (bounds.height.saturating_sub(variant.height).max(1) - 1) as i32;

        self.x += self.dx as i32;
        self.y += self.dy as i32;

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

    pub fn is_active(&self) -> bool {
        self.respawn_at.is_none()
    }

    pub fn mark_exited(&mut self, delay: Duration, now: Instant) {
        self.respawn_at = Some(now + delay);
    }

    pub fn resume_lateral_motion(&mut self) {
        if self.lateral_dx == 0 {
            self.lateral_dx = if self.dx < 0 { -1 } else { 1 };
        }

        self.dx = self.lateral_dx;
        self.dy = 0;
        self.pose_intent = PoseIntent::Lateral;
        self.depth_swim_ticks = 0;
    }

    pub fn toggle_vertical_motion(&mut self, rng: &mut ThreadRng) {
        self.dy = if self.dy == 0 {
            if rng.random_bool(0.5) { -1 } else { 1 }
        } else {
            0
        };
    }

    pub fn maybe_rearrange_school(&mut self, def: &CreatureDef, rng: &mut ThreadRng) {
        if let Some(chance) = def.school_rearrange_chance()
            && rng.random_bool(chance)
        {
            self.school_rearrangements = self.school_rearrangements.wrapping_add(1);
        }
    }
}

pub fn load_creatures(dir: &Path) -> Result<Vec<CreatureDef>> {
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

pub fn load_creature(path: &Path) -> Result<CreatureDef> {
    let source =
        fs::read_to_string(path).wrap_err_with(|| format!("reading {}", path.display()))?;
    let parse_source = normalize_creature_kdl(&source);
    let doc = parse_source
        .parse::<KdlDocument>()
        .wrap_err_with(|| format!("parsing {}", path.display()))
        .with_section(|| parse_source.clone().header("KDL source"))?;

    let fallback_name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("creature")
        .to_string();

    let name = string_arg(&doc, "name").unwrap_or(fallback_name);
    let motion = string_arg(&doc, "motion");
    let brownian = motion.as_deref().is_some_and(|motion| motion == "brownian");
    let unit_motion = doc.get("unit-motion");
    let unit_brownian = unit_motion
        .and_then(|node| node.get(0))
        .and_then(KdlValue::as_string)
        .is_some_and(|motion| motion == "brownian");
    let school_rearrange_chance = if unit_brownian {
        Some(
            optional_probability_prop(
                unit_motion.expect("unit-motion exists"),
                "rearrange-chance",
            )?
            .unwrap_or(0.33),
        )
    } else {
        None
    };
    let h_velocity = int_arg(&doc, "h-velocity").map(clamp_velocity);
    let v_velocity = int_arg(&doc, "v-velocity").map(clamp_velocity);
    let spawn_location = match string_arg(&doc, "spawn-location").as_deref() {
        Some("floor") => SpawnLocation::Floor,
        Some("water") | None => SpawnLocation::Water,
        Some(other) => {
            return Err(eyre!(
                "{} has unsupported spawn-location {other:?}",
                path.display()
            ));
        }
    };
    let count = int_arg(&doc, "count")
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(1);
    let colors = parse_colors(&doc, path)?;
    let default_movement = motion.is_none()
        && h_velocity.is_none()
        && v_velocity.is_none()
        && spawn_location == SpawnLocation::Water;
    let variants = doc
        .nodes()
        .iter()
        .filter_map(|node| {
            let pose = node.name().value();
            if !is_pose_node(pose) {
                return None;
            }
            let art = node.get(0)?.as_string()?;
            let unit = node.get("unit").and_then(KdlValue::as_string);
            Some(Variant::from_kdl_node(
                pose.to_string(),
                art,
                unit,
                unit_brownian,
            ))
        })
        .collect::<Vec<_>>();

    if variants.is_empty() {
        return Err(eyre!("{} has no drawable pose nodes", path.display()));
    }
    let four_way_swimmer = has_pose(&variants, "left")
        && has_pose(&variants, "right")
        && has_pose(&variants, "face")
        && has_pose(&variants, "face-away");

    Ok(CreatureDef {
        name,
        variants,
        count,
        four_way_swimmer,
        spawn_location,
        h_velocity,
        v_velocity,
        brownian,
        colors,
        default_movement,
        school_rearrange_chance,
    })
}

pub fn tallest_variant_height(definitions: &[CreatureDef]) -> u16 {
    definitions
        .iter()
        .flat_map(|definition| &definition.variants)
        .map(|variant| variant.height)
        .max()
        .unwrap_or_default()
}

fn is_pose_node(name: &str) -> bool {
    ["left", "right", "face", "away"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn has_pose(variants: &[Variant], pose: &str) -> bool {
    variants
        .iter()
        .any(|variant| pose_matches(&variant.pose, pose))
}

fn pose_matches(pose: &str, wanted: &str) -> bool {
    if pose == wanted {
        return true;
    }

    pose.strip_prefix(wanted)
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
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

fn parse_colors(doc: &KdlDocument, path: &Path) -> Result<Vec<Color>> {
    let Some(node) = doc.get("colors") else {
        return Ok(Vec::new());
    };

    node.entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .map(|entry| {
            let value = entry
                .value()
                .as_string()
                .ok_or_else(|| eyre!("{} `colors` entries must be strings", path.display()))?;
            Color::from_str(value)
                .map_err(|_| eyre!("{} has unsupported color {value:?}", path.display()))
        })
        .collect()
}

fn optional_probability_prop(node: &kdl::KdlNode, name: &str) -> Result<Option<f64>> {
    let Some(value) = node.get(name) else {
        return Ok(None);
    };
    let Some(value) = value
        .as_float()
        .or_else(|| value.as_integer().map(|int| int as f64))
    else {
        return Err(eyre!(
            "`{}` property `{name}` must be a number from 0.0 to 1.0",
            node.name().value()
        ));
    };

    if !(0.0..=1.0).contains(&value) {
        return Err(eyre!(
            "`{}` property `{name}` must be from 0.0 to 1.0, got {value}",
            node.name().value()
        ));
    }

    Ok(Some(value))
}

fn clamp_velocity(value: i128) -> i16 {
    value.clamp(-1, 1) as i16
}

fn normalize_creature_kdl(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let indent_len = line.len() - trimmed.len();
            let indent = &line[..indent_len];
            if let Some(value) = trimmed.strip_prefix("count=")
                && !value.is_empty()
                && value.chars().all(|ch| ch.is_ascii_digit())
            {
                format!("{indent}count {value}")
            } else if trimmed.starts_with("colors ") {
                format!("{indent}{}", trimmed.replace(',', ""))
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    fn bertrand_has_left_and_right_animation_variants() {
        let bertrand =
            load_creature(Path::new("art/creatures/bertrand.kdl")).expect("bertrand loads");
        let poses = bertrand
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

    #[test]
    fn parses_creature_colors() {
        let bertrand =
            load_creature(Path::new("art/creatures/bertrand.kdl")).expect("bertrand loads");

        assert_eq!(
            bertrand.colors,
            vec![
                Color::Rgb(0xeb, 0xbe, 0x0f),
                Color::Rgb(0x8b, 0xc7, 0x14),
                Color::Rgb(0x80, 0x9b, 0x41),
                Color::Rgb(0x41, 0x4f, 0xbf),
                Color::Rgb(0x83, 0x67, 0xc3),
            ]
        );
    }

    #[test]
    fn absent_creature_count_defaults_to_one() {
        let bumble = load_creature(Path::new("art/creatures/bumble.kdl")).expect("bumble loads");

        assert_eq!(bumble.count, 1);
    }

    #[test]
    fn explicit_zero_creature_count_means_spawn_only() {
        let path = write_test_creature(
            "zero-count",
            r####"
name "zero-count"
count=0

face ###"""
<>
"""###
"####,
        );
        let creature = load_creature(&path).expect("zero-count loads");

        assert_eq!(creature.count, 0);
    }

    #[test]
    fn creature_count_comes_from_count_param() {
        let boxfish = load_creature(Path::new("art/creatures/boxfish.kdl")).expect("boxfish loads");

        assert_eq!(boxfish.count, 2);
    }

    #[test]
    fn default_movement_is_only_for_unspecified_water_creatures() {
        let bumble = load_creature(Path::new("art/creatures/bumble.kdl")).expect("bumble loads");
        let squigs = load_creature(Path::new("art/creatures/squigs.kdl")).expect("squigs loads");
        let wort =
            load_creature(Path::new("art/creatures/wigglewort.kdl")).expect("wigglewort loads");

        assert!(bumble.uses_default_movement());
        assert!(!squigs.uses_default_movement());
        assert!(!wort.uses_default_movement());
    }

    #[test]
    fn default_movement_transition_is_even_odds_across_200_columns() {
        let chance = default_movement_transition_chance();
        let chance_over_200_columns = 1.0 - (1.0 - chance).powi(200);

        assert!((chance_over_200_columns - 0.5).abs() < 0.000_000_000_001);
    }

    #[test]
    fn detects_four_way_swimmers_from_pose_set() {
        let creatures = load_creatures(Path::new("art/creatures")).expect("creatures load");
        let four_way = creatures
            .iter()
            .filter(|creature| creature.four_way_swimmer)
            .map(|creature| creature.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(four_way, vec!["boxfish"]);
    }

    #[test]
    fn face_pose_does_not_match_face_away_variants() {
        let boxfish = load_creature(Path::new("art/creatures/boxfish.kdl")).expect("boxfish loads");

        assert!(
            boxfish
                .best_variant_for(0, PoseIntent::Face, 0, 0)
                .pose
                .starts_with("face")
        );
        assert!(
            !boxfish
                .best_variant_for(0, PoseIntent::Face, 0, 0)
                .pose
                .starts_with("face-away")
        );
        assert!(
            boxfish
                .best_variant_for(0, PoseIntent::FaceAway, 0, 0)
                .pose
                .starts_with("face-away")
        );
    }

    #[test]
    fn parses_floor_spawn_location() {
        let wort =
            load_creature(Path::new("art/creatures/wigglewort.kdl")).expect("wigglewort loads");

        assert_eq!(wort.spawn_location, SpawnLocation::Floor);
    }

    #[test]
    fn parses_school_local_brownian_units() {
        let squigs = load_creature(Path::new("art/creatures/squigs.kdl")).expect("squigs loads");
        let variant = squigs.best_variant_for(0, PoseIntent::Face, 0, 0);
        let school = variant.school.as_ref().expect("squigs has school units");

        assert!(squigs.brownian);
        assert!(squigs.school_rearrange_chance().is_some());
        assert_eq!(school.unit, "~");
        assert_eq!(school.units.len(), 9);
        assert_school_units_fit_bbox(variant);
    }

    #[test]
    fn parses_multichar_school_units_for_each_pose() {
        let oldskool =
            load_creature(Path::new("art/creatures/oldskool.kdl")).expect("oldskool loads");

        assert!(oldskool.brownian);
        assert!(oldskool.school_rearrange_chance().is_some());
        for variant in &oldskool.variants {
            let school = variant.school.as_ref().expect("oldskool has school units");

            assert_eq!(school.units.len(), 9);
            assert_eq!(school.unit.chars().count(), 3);
            assert_school_units_fit_bbox(variant);
        }
    }

    #[test]
    fn school_rearrangement_chance_is_configurable() {
        let path = write_test_creature(
            "school-chance",
            r####"
name "school-chance"
unit-motion "brownian" rearrange-chance=0.75

face ###"""
o o
"""### unit="o"
"####,
        );
        let creature = load_creature(&path).expect("school chance loads");

        assert_eq!(creature.school_rearrange_chance(), Some(0.75));
    }

    #[test]
    fn normalizes_compact_count_param_for_kdl_parser() {
        assert_eq!(
            normalize_creature_kdl("name \"bee\"\n\ncount=2\n"),
            "name \"bee\"\n\ncount 2"
        );
    }

    #[test]
    fn normalizes_comma_separated_colors_for_kdl_parser() {
        assert_eq!(
            normalize_creature_kdl("colors \"#fff\", \"#000\"\n"),
            "colors \"#fff\" \"#000\""
        );
    }

    fn assert_school_units_fit_bbox(variant: &Variant) {
        let expected_width = variant
            .art
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or_default();
        let expected_height = variant.art.len();
        let school = variant.school.as_ref().expect("variant has school units");
        let unit_width = school.unit.chars().count() as u16;

        assert_eq!(variant.width as usize, expected_width);
        assert_eq!(variant.height as usize, expected_height);
        assert!(
            school
                .units
                .iter()
                .all(|unit| unit.x.saturating_add(unit_width) <= variant.width
                    && unit.y < variant.height)
        );
    }

    fn write_test_creature(name: &str, source: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "reefs-creature-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("test creature dir created");
        let path = dir.join(format!("{name}.kdl"));
        fs::write(&path, source.trim_start()).expect("test creature written");
        path
    }
}
