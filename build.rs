use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn collect_toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_toml_files(&path));
            } else if path.extension().is_some_and(|e| e == "toml") {
                let name = path.file_name().unwrap_or_default();
                if name != "SAMPLE.toml" {
                    files.push(path);
                }
            }
        }
    }
    files.sort();
    files
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let frameworks_dir = manifest_dir.join("frameworks");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_file = out_dir.join("framework_includes.rs");

    let toml_files = collect_toml_files(&frameworks_dir);

    let mut output = fs::File::create(&out_file).expect("create output file");

    writeln!(output, "fn framework_toml_strings() -> &'static [&'static str] {{").expect("write");
    writeln!(output, "    &[").expect("write");

    for path in &toml_files {
        let rel = path.strip_prefix(&manifest_dir).unwrap_or(path);
        let rel_str = rel.display().to_string().replace('\\', "/");
        println!("cargo:rerun-if-changed={rel_str}");
        writeln!(output, "        include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{rel_str}\")),").expect("write");
    }

    writeln!(output, "    ]").expect("write");
    writeln!(output, "}}").expect("write");

    println!("cargo:rerun-if-changed=frameworks");
}
