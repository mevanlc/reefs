use std::fs;

use color_eyre::{
    Section, SectionExt,
    eyre::{Result, WrapErr, eyre},
};
use kdl::{KdlDocument, KdlNode};
use ratatui::style::Color;

use crate::config::LayerConfig;

#[derive(Debug, Clone)]
pub struct ReefWorld {
    pub surface: WorldLayer,
    pub floor: WorldLayer,
    pub launch_page_width: u16,
    pub offscreen_pages: f64,
    pub viewport_x: i32,
}

impl ReefWorld {
    pub fn new(
        surface: WorldLayer,
        floor: WorldLayer,
        launch_page_width: u16,
        offscreen_pages: f64,
    ) -> Self {
        Self {
            surface,
            floor,
            launch_page_width: launch_page_width.max(1),
            offscreen_pages,
            viewport_x: 0,
        }
    }

    pub fn scroll_by(&mut self, delta: i32) {
        self.viewport_x = self.viewport_x.saturating_add(delta);
    }

    pub fn simulated_bounds(&self, visible_width: u16) -> WorldBounds {
        let offscreen_cols = (self.launch_page_width as f64 * self.offscreen_pages).round() as i32;
        WorldBounds {
            start: self.viewport_x.saturating_sub(offscreen_cols),
            end: self
                .viewport_x
                .saturating_add(visible_width as i32)
                .saturating_add(offscreen_cols),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldBounds {
    pub start: i32,
    pub end: i32,
}

#[derive(Debug, Clone)]
pub struct WorldLayer {
    chunks: Vec<LayerChunk>,
    pattern: Vec<usize>,
    pattern_width: i32,
    pub color: Color,
    pub height: u16,
}

impl WorldLayer {
    pub fn cell_at(&self, world_x: i32, row: u16) -> Option<char> {
        let mut offset = world_x.rem_euclid(self.pattern_width);
        for chunk_index in &self.pattern {
            let chunk = &self.chunks[*chunk_index];
            if offset < chunk.width as i32 {
                let line = chunk.lines.get(row as usize)?;
                let symbol = line.chars().nth(offset as usize)?;

                return if symbol == ' ' { None } else { Some(symbol) };
            }
            offset -= chunk.width as i32;
        }

        None
    }

    #[cfg(test)]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

#[derive(Debug, Clone)]
struct LayerChunk {
    lines: Vec<String>,
    width: u16,
    height: u16,
}

pub fn load_world_layer(config: &LayerConfig) -> Result<WorldLayer> {
    let source = fs::read_to_string(&config.file)
        .wrap_err_with(|| format!("reading {}", config.file.display()))?;
    let doc = source
        .parse::<KdlDocument>()
        .wrap_err_with(|| format!("parsing {}", config.file.display()))
        .with_section(|| source.clone().header("KDL source"))?;

    let chunks_node = doc
        .get("chunks")
        .ok_or_else(|| eyre!("{} is missing `chunks`", config.file.display()))?;
    let chunks = chunks_node
        .children()
        .ok_or_else(|| eyre!("{} `chunks` node has no children", config.file.display()))?
        .nodes()
        .iter()
        .map(parse_chunk_node)
        .collect::<Vec<_>>();

    if chunks.is_empty() {
        return Err(eyre!("{} has no world chunks", config.file.display()));
    }

    let height = chunks
        .iter()
        .map(|chunk| chunk.height)
        .max()
        .unwrap_or_default()
        .max(1);
    Ok(WorldLayer::new(chunks, config.color, height))
}

fn build_pattern(chunks: &[LayerChunk]) -> (Vec<usize>, i32) {
    const PATTERN_CHUNKS: i32 = 64;

    let pattern = (0..PATTERN_CHUNKS)
        .map(|block| stable_index(block, chunks.len()))
        .collect::<Vec<_>>();
    let pattern_width = pattern
        .iter()
        .map(|chunk_index| chunks[*chunk_index].width.max(1) as i32)
        .sum::<i32>()
        .max(1);

    (pattern, pattern_width)
}

impl WorldLayer {
    fn new(chunks: Vec<LayerChunk>, color: Color, height: u16) -> Self {
        let (pattern, pattern_width) = build_pattern(&chunks);
        Self {
            chunks,
            pattern,
            pattern_width,
            color,
            height,
        }
    }
}

fn parse_chunk_node(node: &KdlNode) -> LayerChunk {
    let source = node
        .get(0)
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| node.name().value());
    let lines = source
        .trim_matches('\n')
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect::<Vec<_>>();
    let width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or_default()
        .min(u16::MAX as usize) as u16;
    let height = lines.len().min(u16::MAX as usize) as u16;

    LayerChunk {
        lines,
        width: width.max(1),
        height,
    }
}

fn stable_index(block: i32, len: usize) -> usize {
    let mut value = block as i64;
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51afd7ed558ccd_u64 as i64);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ceb9fe1a85ec53_u64 as i64);
    value ^= value >> 33;
    value.rem_euclid(len as i64) as usize
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn loads_world_chunks_from_quoted_child_nodes() {
        let layer = load_world_layer(&LayerConfig {
            file: PathBuf::from("art/world/floor1.kdl"),
            color: Color::Green,
        })
        .expect("floor loads");

        assert_eq!(layer.chunk_count(), 4);
        assert_eq!(layer.height, 1);
    }

    #[test]
    fn simulated_bounds_use_launch_page_for_offscreen_extent() {
        let world = ReefWorld::new(
            WorldLayer {
                chunks: vec![LayerChunk {
                    lines: vec!["~".to_string()],
                    width: 1,
                    height: 1,
                }],
                pattern: vec![0],
                pattern_width: 1,
                color: Color::Blue,
                height: 1,
            },
            WorldLayer {
                chunks: vec![LayerChunk {
                    lines: vec!["-".to_string()],
                    width: 1,
                    height: 1,
                }],
                pattern: vec![0],
                pattern_width: 1,
                color: Color::Green,
                height: 1,
            },
            100,
            0.5,
        );

        assert_eq!(
            world.simulated_bounds(80),
            WorldBounds {
                start: -50,
                end: 130
            }
        );
    }

    #[test]
    fn variable_width_chunks_render_contiguously() {
        let layer = WorldLayer::new(
            vec![
                LayerChunk {
                    lines: vec!["--".to_string()],
                    width: 2,
                    height: 1,
                },
                LayerChunk {
                    lines: vec!["----".to_string()],
                    width: 4,
                    height: 1,
                },
            ],
            Color::Green,
            1,
        );

        for x in 0..64 {
            assert_eq!(layer.cell_at(x, 0), Some('-'));
        }
    }
}
