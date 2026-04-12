#![allow(dead_code)]

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
}

include!(concat!(env!("OUT_DIR"), "/art_assets.rs"));
