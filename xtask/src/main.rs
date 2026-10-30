//! MakerFoundry's developer tasks. `cargo xtask ci` runs the checks every commit must pass, the
//! same ones CI runs: formatting, clippy with warnings as errors, the tests, and the assets table.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let result = match task.as_deref() {
        Some("ci") => ci(),
        Some("assets") => assets(),
        _ => {
            eprintln!(
                "usage: cargo xtask <task>\n\ntasks:\n  ci      fmt, clippy -D warnings, tests, the service's own build, assets\n  assets  every asset file has a row in ASSETS.md"
            );
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn ci() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace", "--locked"])?;
    // the service on its own, as the Docker image builds it: without the tests' dev-dependencies,
    // whose features would otherwise hide a missing one (MF-7's axum `json`)
    cargo(&["build", "-p", "makerfoundry", "--bins", "--locked"])?;
    assets()?;
    test_files()?;
    eprintln!("xtask: ci passed");
    Ok(())
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    eprintln!("xtask: cargo {}", args.join(" "));
    let status = Command::new(&cargo)
        .args(args)
        .current_dir(root())
        .status()
        .map_err(|err| format!("can't run {cargo}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed ({status})", args.join(" ")))
    }
}

/// The workspace root: the folder above this crate's.
/// Every file in `apps/*/tests` has tests in it. (A script once emptied `tests/pages.rs` by
/// opening it to write before reading it; an empty test file runs nothing and passes, so it went
/// unnoticed for a day.)
fn test_files() -> Result<(), String> {
    let apps = root().join("apps");
    let mut problems = Vec::new();
    let mut count = 0;
    for app in std::fs::read_dir(&apps).map_err(|e| format!("{}: {e}", apps.display()))? {
        let dir = app.map_err(|e| e.to_string())?.path().join("tests");
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().is_some_and(|e| e == "rs") {
                count += 1;
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                if !text.contains("#[test]") && !text.contains("#[tokio::test]") && !text.contains("proptest!") {
                    problems.push(format!("{} has no tests in it", path.display()));
                }
            }
        }
    }
    if problems.is_empty() {
        eprintln!("xtask: test files ok ({count} files)");
        Ok(())
    } else {
        Err(format!("test files:\n  {}", problems.join("\n  ")))
    }
}

fn root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    here.parent().unwrap_or(here).to_path_buf()
}

/// Files that are assets (not code or docs) must each have a row in ASSETS.md.
const ASSET_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "ico", "svg", "bmp", "tif", "tiff", "ttf", "otf", "woff", "woff2",
];

/// Fonts are never committed (CLAUDE.md › Assets).
const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "woff", "woff2"];

/// Folders that hold build output or someone else's files, not ours.
const SKIPPED_DIRS: &[&str] = &[".git", "target", "plan"];

fn assets() -> Result<(), String> {
    let root = root();
    let table =
        std::fs::read_to_string(root.join("ASSETS.md")).map_err(|err| format!("can't read ASSETS.md: {err}"))?;
    let mut files = Vec::new();
    collect_assets(&root, &root, &mut files)?;
    let mut problems = Vec::new();
    for file in &files {
        let ext = Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if FONT_EXTENSIONS.contains(&ext.as_str()) {
            problems.push(format!("{file}: font files are not allowed in this repository"));
        } else if !table.contains(&format!("`{file}`")) {
            problems.push(format!("{file}: no row in ASSETS.md"));
        }
    }
    if problems.is_empty() {
        eprintln!("xtask: assets ok ({} files)", files.len());
        Ok(())
    } else {
        Err(format!("assets:\n  {}", problems.join("\n  ")))
    }
}

fn collect_assets(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|err| format!("can't list {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("can't list {}: {err}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let kind = entry
            .file_type()
            .map_err(|err| format!("can't stat {}: {err}", path.display()))?;
        if kind.is_dir() {
            if !SKIPPED_DIRS.iter().any(|skip| name == *skip) {
                collect_assets(root, &path, out)?;
            }
        } else if kind.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ASSET_EXTENSIONS.contains(&ext.as_str()) {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(())
}
