#![allow(dead_code)]

#[derive(Debug)]
pub(crate) struct AsciiArtIdleAnimationFrame {
    pub(crate) duration_ms: u64,
    pub(crate) rows: &'static [&'static str],
}

#[derive(Debug)]
pub(crate) struct AsciiArtIdleAnimation {
    pub(crate) name: &'static str,
    pub(crate) average_interval_ms: u64,
    pub(crate) frames: &'static [AsciiArtIdleAnimationFrame],
}

#[derive(Debug)]
pub(crate) struct AsciiArtAsset {
    pub(crate) id: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) tags: &'static [&'static str],
    pub(crate) transparent: char,
    pub(crate) anchor_x: i32,
    pub(crate) anchor_y: i32,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) rows: &'static [&'static str],
    pub(crate) idle_animations: &'static [AsciiArtIdleAnimation],
}

include!(concat!(env!("OUT_DIR"), "/art_assets.rs"));

impl AsciiArtAsset {
    pub(crate) fn world_anchor_x(&self) -> i32 {
        self.anchor_x.div_euclid(2)
    }

    pub(crate) fn glyph_pair_at_world(&self, local_x: i32, local_y: i32) -> Option<(char, char)> {
        self.glyph_pair_at_world_in_rows(self.rows, local_x, local_y)
    }

    pub(crate) fn glyph_pair_at_world_in_rows(
        &self,
        rows: &'static [&'static str],
        local_x: i32,
        local_y: i32,
    ) -> Option<(char, char)> {
        if local_x < 0 || local_y < 0 {
            return None;
        }

        let row = rows.get(local_y as usize)?;
        let chars: Vec<char> = row.chars().collect();
        let start = local_x as usize * 2;
        if start >= chars.len() {
            return None;
        }

        let left = chars[start];
        let right = chars.get(start + 1).copied().unwrap_or(self.transparent);
        if left == self.transparent && right == self.transparent {
            return None;
        }

        Some((left, right))
    }
}
