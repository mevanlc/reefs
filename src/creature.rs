use std::{
    fs,
    path::Path,
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
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub pose: String,
    pub art: Vec<String>,
    pub width: u16,
    pub height: u16,
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
    let brownian = string_arg(&doc, "unit-motion").is_some_and(|motion| motion == "brownian");
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
    let count = int_arg(&doc, "n")
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(1);
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
            if let Some(value) = trimmed.strip_prefix("n=")
                && !value.is_empty()
                && value.chars().all(|ch| ch.is_ascii_digit())
            {
                format!("{indent}n {value}")
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

    #[test]
    fn absent_creature_count_defaults_to_one() {
        let bumble = load_creature(Path::new("art/creatures/bumble.kdl")).expect("bumble loads");

        assert_eq!(bumble.count, 1);
    }

    #[test]
    fn explicit_zero_creature_count_means_spawn_only() {
        let turtle = load_creature(Path::new("art/creatures/turtle.kdl")).expect("turtle loads");

        assert_eq!(turtle.count, 0);
    }

    #[test]
    fn creature_count_comes_from_n_param() {
        let boxfish = load_creature(Path::new("art/creatures/boxfish.kdl")).expect("boxfish loads");

        assert_eq!(boxfish.count, 2);
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
    fn normalizes_compact_n_param_for_kdl_parser() {
        assert_eq!(
            normalize_creature_kdl("name \"bee\"\n\nn=2\n"),
            "name \"bee\"\n\nn 2"
        );
    }
}
