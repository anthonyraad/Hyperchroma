use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Canonical recording names, in the order they appear in the instrument.
const NAMES: [&str; 24] = [
    "bug", "dark", "dragon", "electric", "fairy", "mulberry", "fighting", "fire", "flying",
    "ghost", "grass", "olive", "ground", "ice", "normal", "poison", "psychic", "aloe", "rock",
    "steel", "water", "sage", "clay", "cinnamon",
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
            if let Some(index) = files.iter().position(|(existing, _)| existing == &name) {
                let older = &files[index].1;
                let keep_new = match (fs::metadata(&path), fs::metadata(older)) {
                    (Ok(fresh), Ok(prior)) => fresh.modified().ok() >= prior.modified().ok(),
                    _ => true,
                };
                if keep_new {
                    println!(
                        "cargo:warning=replacing older sample {} with {} ({})",
                        older.display(),
                        path.display(),
                        name
                    );
                    files[index] = (name, path.clone());
                    println!("cargo:rerun-if-changed={}", path.display());
                } else {
                    println!(
                        "cargo:warning=skipping older duplicate sample {} ({})",
                        path.display(),
                        name
                    );
                }
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
    // B-sides first so `bug_b` is not collapsed into `bug`.
    for name in NAMES {
        if lower == format!("{name}_b")
            || lower.starts_with(&format!("{name}_b_"))
            || lower.starts_with(&format!("{name}_b-"))
            || lower == format!("{name}-b")
            || lower.starts_with(&format!("{name}-b_"))
            || lower.starts_with(&format!("{name}-b-"))
        {
            return format!("{name}_b");
        }
    }
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
    if let Some(base) = name.strip_suffix("_b") {
        if let Some(index) = NAMES.iter().position(|n| *n == base) {
            return (index, "b".to_string());
        }
    }
    match NAMES.iter().position(|n| *n == name) {
        Some(index) => (index, String::new()),
        None => (NAMES.len(), name.to_string()),
    }
}

fn path_for_include(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").replace('"', "")
}
