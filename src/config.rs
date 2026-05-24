use std::{
    fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, WrapErr, eyre};
use kdl::{KdlDocument, KdlNode, KdlValue};
use ratatui::style::Color;

use crate::kdl_parse;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub reef: ReefConfig,
}

#[derive(Debug, Clone)]
pub struct ReefConfig {
    pub horizontal: HorizontalConfig,
    pub vertical: VerticalConfig,
    pub creatures: CreatureBehaviorConfig,
    pub colors: ReefColorConfig,
}

#[derive(Debug, Clone)]
pub struct HorizontalConfig {
    pub scroll_enabled: bool,
    pub offscreen_pages: f64,
    pub floor: LayerConfig,
    pub surface: LayerConfig,
}

#[derive(Debug, Clone)]
pub struct VerticalConfig {
    pub scroll_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct LayerConfig {
    pub file: PathBuf,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct CreatureBehaviorConfig {
    pub respawn_delay_ms: u64,
    pub count_scale: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReefColorConfig {
    pub indexed256: ColorPoolConfig,
    pub truecolor: ColorPoolConfig,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ColorPoolConfig {
    pub random_color_brightness_min: f64,
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    let source =
        fs::read_to_string(path).wrap_err_with(|| format!("reading {}", path.display()))?;
    let doc = kdl_parse::parse_document(path, &source)?;

    parse_config(&doc)
}

fn parse_config(doc: &KdlDocument) -> Result<AppConfig> {
    Ok(AppConfig {
        reef: parse_reef(required_node(doc, "reef")?)?,
    })
}

fn parse_reef(node: &KdlNode) -> Result<ReefConfig> {
    let horizontal = child(node, "horizontal")?;
    let vertical = child(node, "vertical")?;
    let creatures = child(node, "creatures")?;

    assert_arg(child(horizontal, "size")?, 0, "infinite")?;
    assert_arg(child(vertical, "size")?, 0, "fit-terminal")?;

    let horizontal_scroll = child(horizontal, "scroll")?;
    let vertical_scroll = child(vertical, "scroll")?;

    Ok(ReefConfig {
        horizontal: HorizontalConfig {
            scroll_enabled: prop_bool(horizontal_scroll, "enabled")?,
            offscreen_pages: prop_float(horizontal_scroll, "offscreen-pages")?,
            floor: parse_layer(child(horizontal, "floor")?)?,
            surface: parse_layer(child(horizontal, "surface")?)?,
        },
        vertical: VerticalConfig {
            scroll_enabled: prop_bool(vertical_scroll, "enabled")?,
        },
        creatures: parse_creatures(creatures)?,
        colors: parse_reef_colors(node)?,
    })
}

fn parse_layer(node: &KdlNode) -> Result<LayerConfig> {
    assert_prop(node, "chunkgen", "random")?;
    Ok(LayerConfig {
        file: PathBuf::from(prop_string(node, "file")?),
        color: parse_color(prop_string(node, "color")?)?,
    })
}

fn parse_creatures(node: &KdlNode) -> Result<CreatureBehaviorConfig> {
    assert_arg(child(node, "edge-behavior")?, 0, "exit-world")?;
    let respawn = child(node, "respawn")?;
    assert_prop(respawn, "condition", "after-exit-world")?;
    Ok(CreatureBehaviorConfig {
        respawn_delay_ms: prop_u64(respawn, "delay-ms")?,
        count_scale: optional_child(node, "count-scale")
            .map(|node| arg_non_negative_float(node, 0))
            .transpose()?
            .unwrap_or(1.0),
    })
}

fn parse_reef_colors(node: &KdlNode) -> Result<ReefColorConfig> {
    Ok(ReefColorConfig {
        indexed256: optional_child(node, "colors-256")
            .map(parse_color_pool)
            .transpose()?
            .unwrap_or_default(),
        truecolor: optional_child(node, "colors-true")
            .map(parse_color_pool)
            .transpose()?
            .unwrap_or_default(),
    })
}

fn parse_color_pool(node: &KdlNode) -> Result<ColorPoolConfig> {
    Ok(ColorPoolConfig {
        random_color_brightness_min: optional_child(node, "random-color-brightness-min")
            .map(|node| arg_unit_float(node, 0))
            .transpose()?
            .unwrap_or_default(),
    })
}

fn required_node<'a>(doc: &'a KdlDocument, name: &str) -> Result<&'a KdlNode> {
    doc.get(name)
        .ok_or_else(|| eyre!("missing required `{name}` node"))
}

fn child<'a>(node: &'a KdlNode, name: &str) -> Result<&'a KdlNode> {
    node.children()
        .and_then(|children| children.get(name))
        .ok_or_else(|| {
            eyre!(
                "missing required `{name}` child in `{}`",
                node.name().value()
            )
        })
}

fn optional_child<'a>(node: &'a KdlNode, name: &str) -> Option<&'a KdlNode> {
    node.children().and_then(|children| children.get(name))
}

fn arg_string(node: &KdlNode, index: usize) -> Result<&str> {
    node.get(index)
        .and_then(KdlValue::as_string)
        .ok_or_else(|| eyre!("`{}` requires string argument {index}", node.name().value()))
}

fn arg_non_negative_float(node: &KdlNode, index: usize) -> Result<f64> {
    let value = node
        .get(index)
        .and_then(|value| {
            value
                .as_float()
                .or_else(|| value.as_integer().map(|int| int as f64))
        })
        .ok_or_else(|| {
            eyre!(
                "`{}` requires numeric argument {index}",
                node.name().value()
            )
        })?;

    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(eyre!(
            "`{}` argument {index} must be a finite non-negative number",
            node.name().value()
        ))
    }
}

fn arg_unit_float(node: &KdlNode, index: usize) -> Result<f64> {
    let value = arg_non_negative_float(node, index)?;
    if value <= 1.0 {
        Ok(value)
    } else {
        Err(eyre!(
            "`{}` argument {index} must be between 0.0 and 1.0",
            node.name().value()
        ))
    }
}

fn prop_string<'a>(node: &'a KdlNode, name: &str) -> Result<&'a str> {
    node.get(name).and_then(KdlValue::as_string).ok_or_else(|| {
        eyre!(
            "`{}` requires string property `{name}`",
            node.name().value()
        )
    })
}

fn prop_bool(node: &KdlNode, name: &str) -> Result<bool> {
    node.get(name)
        .and_then(KdlValue::as_bool)
        .ok_or_else(|| eyre!("`{}` requires bool property `{name}`", node.name().value()))
}

fn prop_float(node: &KdlNode, name: &str) -> Result<f64> {
    node.get(name)
        .and_then(|value| {
            value
                .as_float()
                .or_else(|| value.as_integer().map(|int| int as f64))
        })
        .ok_or_else(|| {
            eyre!(
                "`{}` requires numeric property `{name}`",
                node.name().value()
            )
        })
}

fn prop_u64(node: &KdlNode, name: &str) -> Result<u64> {
    let value = node
        .get(name)
        .and_then(KdlValue::as_integer)
        .ok_or_else(|| {
            eyre!(
                "`{}` requires integer property `{name}`",
                node.name().value()
            )
        })?;

    value.try_into().map_err(|_| {
        eyre!(
            "`{}` property `{name}` must be non-negative",
            node.name().value()
        )
    })
}

fn assert_arg(node: &KdlNode, index: usize, expected: &str) -> Result<()> {
    let actual = arg_string(node, index)?;
    if actual == expected {
        Ok(())
    } else {
        Err(eyre!(
            "`{}` argument {index} must be {expected:?}, got {actual:?}",
            node.name().value()
        ))
    }
}

fn assert_prop(node: &KdlNode, name: &str, expected: &str) -> Result<()> {
    let actual = prop_string(node, name)?;
    if actual == expected {
        Ok(())
    } else {
        Err(eyre!(
            "`{}` property `{name}` must be {expected:?}, got {actual:?}",
            node.name().value()
        ))
    }
}

fn parse_color(name: &str) -> Result<Color> {
    match name {
        "black" => Ok(Color::Black),
        "blue" => Ok(Color::Blue),
        "cyan" => Ok(Color::Cyan),
        "green" => Ok(Color::Green),
        "magenta" => Ok(Color::Magenta),
        "red" => Ok(Color::Red),
        "white" => Ok(Color::White),
        "yellow" => Ok(Color::Yellow),
        other => Err(eyre!("unsupported color {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_current_config_shape() {
        let config = load_config(Path::new("config.kdl")).expect("config loads");

        assert!(!config.reef.vertical.scroll_enabled);
        assert_eq!(config.reef.creatures.respawn_delay_ms, 1000);
        assert_eq!(config.reef.creatures.count_scale, 1.9);
        assert_eq!(
            config.reef.colors.indexed256.random_color_brightness_min,
            0.3
        );
        assert_eq!(
            config.reef.colors.truecolor.random_color_brightness_min,
            0.3
        );
    }
}
