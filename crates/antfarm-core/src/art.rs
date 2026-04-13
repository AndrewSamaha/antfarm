#[derive(Debug)]
pub struct AsciiArtAsset {
    pub id: &'static str,
    pub kind: &'static str,
    pub tags: &'static [&'static str],
    pub transparent: char,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub width: usize,
    pub height: usize,
    pub rows: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/art_assets.rs"));

pub fn find_ascii_art_asset(id: &str) -> Option<&'static AsciiArtAsset> {
    ASCII_ART_ASSETS.iter().find(|asset| asset.id == id)
}

impl AsciiArtAsset {
    pub fn world_width(&self) -> i32 {
        (self.width as i32 + 1) / 2
    }

    pub fn world_anchor_x(&self) -> i32 {
        self.anchor_x.div_euclid(2)
    }

    pub fn glyph_pair_at_world(&self, local_x: i32, local_y: i32) -> Option<(char, char)> {
        if local_x < 0 || local_y < 0 {
            return None;
        }

        let row = self.rows.get(local_y as usize)?;
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
