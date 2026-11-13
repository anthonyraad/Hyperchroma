use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Canonical recording names, in the order they appear in the instrument.
const NAMES: [&str; 18] = [
    "bug", "dark", "dragon", "electric", "fairy", "fighting", "fire", "flying", "ghost", "grass",
    "ground", "ice", "normal", "poison", "psychic", "rock", "steel", "water",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let samples_dir = manifest.join("samples");
    println!("cargo:rerun-if-changed={}", samples_dir.display());

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if samples_dir.is_dir() {
        let read = fs::read_dir(&samples_dir).expect("read samples directory");
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "mp3" && ext != "wav" {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if stem.is_empty() {
                continue;
            }
            let name = canonical(&stem);
            if files.iter().any(|(existing, _)| existing == &name) {
                println!(
                    "cargo:warning=skipping duplicate sample {} ({})",
                    path.display(),
                    name
                );
                continue;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            files.push((name, path));
        }
    }

    files.sort_by_key(|(name, _)| name_rank(name));

    let mut rust = String::from(
        "pub struct Asset {\n    pub name: &'static str,\n    pub bytes: &'static [u8],\n}\n\
         pub static ASSETS: &[Asset] = &[\n",
    );
    for (name, path) in &files {
        let rendered = path_for_include(path);
        rust.push_str(&format!(
            "    Asset {{ name: \"{name}\", bytes: include_bytes!(\"{rendered}\") }},\n"
        ));
    }
    rust.push_str("];\n");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("assets.rs");
    fs::write(out, rust).expect("write assets.rs");
}

fn canonical(stem: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    for name in NAMES {
        if lower == name
            || lower.starts_with(&format!("{name}_"))
            || lower.starts_with(&format!("{name}-"))
        {
            return name.to_string();
        }
    }
    lower
}

fn name_rank(name: &str) -> (usize, String) {
    match NAMES.iter().position(|n| *n == name) {
        Some(index) => (index, String::new()),
        None => (NAMES.len(), name.to_string()),
    }
}

fn path_for_include(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").replace('"', "")
}
