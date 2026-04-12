use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
struct SourceAsset {
    id: String,
    kind: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_transparent")]
    transparent: String,
    #[serde(default)]
    anchor_x: i32,
    #[serde(default)]
    anchor_y: i32,
    art: String,
}

#[derive(Debug)]
struct CompiledAsset {
    id: String,
    kind: String,
    tags: Vec<String>,
    transparent: char,
    anchor_x: i32,
    anchor_y: i32,
    width: usize,
    height: usize,
    rows: Vec<String>,
}

pub fn generate_art_module(art_dir: &Path, output: &Path) -> Result<()> {
    let mut source_files = Vec::new();
    collect_art_files(art_dir, &mut source_files)?;
    source_files.sort();

    let mut assets = Vec::new();
    for path in source_files {
        assets.push(load_asset(&path)?);
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create generated art dir {}", parent.display()))?;
    }
    fs::write(output, emit_module(&assets))
        .with_context(|| format!("write generated art module {}", output.display()))?;
    Ok(())
}

fn collect_art_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).with_context(|| format!("read art dir {}", path.display()))? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_art_files(&entry_path, out)?;
        } else if entry_path.extension().is_some_and(|ext| ext == "toml") {
            out.push(entry_path);
        }
    }
    Ok(())
}

fn load_asset(path: &Path) -> Result<CompiledAsset> {
    let content =
        fs::read_to_string(path).with_context(|| format!("read art asset {}", path.display()))?;
    let source: SourceAsset =
        toml::from_str(&content).with_context(|| format!("parse TOML {}", path.display()))?;
    let transparent = parse_transparent(&source.transparent)
        .with_context(|| format!("parse transparent char in {}", path.display()))?;
    let rows = crop_ascii(&source.art, transparent)
        .with_context(|| format!("crop art in {}", path.display()))?;
    if rows.is_empty() {
        bail!("art in {} is empty after crop", path.display());
    }
    let width = rows.iter().map(|row| row.len()).max().unwrap_or(0);
    let height = rows.len();

    Ok(CompiledAsset {
        id: source.id,
        kind: source.kind,
        tags: source.tags,
        transparent,
        anchor_x: source.anchor_x,
        anchor_y: source.anchor_y,
        width,
        height,
        rows,
    })
}

fn parse_transparent(value: &str) -> Result<char> {
    let mut chars = value.chars();
    let ch = chars
        .next()
        .ok_or_else(|| anyhow!("transparent must contain exactly one character"))?;
    if chars.next().is_some() {
        bail!("transparent must contain exactly one character");
    }
    Ok(ch)
}

fn crop_ascii(source: &str, transparent: char) -> Result<Vec<String>> {
    let mut lines: Vec<&str> = source.lines().collect();
    while lines.first().is_some_and(|line| line.chars().all(|ch| ch == transparent)) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.chars().all(|ch| ch == transparent)) {
        lines.pop();
    }
    if lines.is_empty() {
        return Ok(Vec::new());
    }

    let max_width = lines.iter().map(|line| line.len()).max().unwrap_or(0);
    let normalized: Vec<Vec<char>> = lines
        .iter()
        .map(|line| {
            let mut chars: Vec<char> = line.chars().collect();
            while chars.len() < max_width {
                chars.push(transparent);
            }
            chars
        })
        .collect();

    let mut left = max_width;
    let mut right = 0usize;
    for row in &normalized {
        for (index, ch) in row.iter().enumerate() {
            if *ch != transparent {
                left = left.min(index);
                right = right.max(index);
            }
        }
    }

    if left > right {
        return Ok(Vec::new());
    }

    let mut cropped = Vec::with_capacity(normalized.len());
    for row in normalized {
        let segment: String = row[left..=right].iter().collect();
        cropped.push(segment);
    }
    Ok(cropped)
}

fn emit_module(assets: &[CompiledAsset]) -> String {
    let mut out = String::new();
    out.push_str("pub(crate) static ASCII_ART_ASSETS: &[AsciiArtAsset] = &[\n");
    for asset in assets {
        out.push_str("    AsciiArtAsset {\n");
        writeln!(&mut out, "        id: {:?},", asset.id).unwrap();
        writeln!(&mut out, "        kind: {:?},", asset.kind).unwrap();
        out.push_str("        tags: &[\n");
        for tag in &asset.tags {
            writeln!(&mut out, "            {:?},", tag).unwrap();
        }
        out.push_str("        ],\n");
        writeln!(&mut out, "        transparent: {:?},", asset.transparent).unwrap();
        writeln!(&mut out, "        anchor_x: {},", asset.anchor_x).unwrap();
        writeln!(&mut out, "        anchor_y: {},", asset.anchor_y).unwrap();
        writeln!(&mut out, "        width: {},", asset.width).unwrap();
        writeln!(&mut out, "        height: {},", asset.height).unwrap();
        out.push_str("        rows: &[\n");
        for row in &asset.rows {
            writeln!(&mut out, "            {:?},", row).unwrap();
        }
        out.push_str("        ],\n");
        out.push_str("    },\n");
    }
    out.push_str("];\n");
    out
}

fn default_transparent() -> String {
    " ".to_string()
}
