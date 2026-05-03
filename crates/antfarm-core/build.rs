use anyhow::Result;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() -> Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let art_dir = workspace_root.join("art");
    println!("cargo:rerun-if-changed={}", art_dir.display());
    emit_rerun_paths(&art_dir)?;

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let output = out_dir.join("art_assets.rs");
    antfarm_tools::generate_art_module(&art_dir, &output)?;
    Ok(())
}

fn emit_rerun_paths(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        println!("cargo:rerun-if-changed={}", entry_path.display());
        if entry.file_type()?.is_dir() {
            emit_rerun_paths(&entry_path)?;
        }
    }
    Ok(())
}
