//! Keeps `diagrams/*.mmd` as the single source of the Mermaid diagrams.
//!
//! Every ```mermaid block in `docs/**/*.md` that is preceded by a line
//! `{/* diagram: NAME */}` is replaced with the content of `diagrams/NAME.mmd`.
//! Run from the repo root: `cargo run -p sync-diagrams` (or `npm run sync-diagrams`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MARK_START: &str = "{/* diagram: ";
const MARK_END: &str = " */}";
const FENCE: &str = "```mermaid";

fn main() -> ExitCode {
    let root = repo_root();
    let docs = root.join("docs");
    let diagrams = root.join("diagrams");
    let mut files = Vec::new();
    collect_md(&docs, &mut files);
    files.sort();

    let mut changed = 0;
    for path in &files {
        let original = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cannot read {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };
        let updated = match sync(&original, &diagrams) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        };
        if updated != original {
            if let Err(e) = fs::write(path, updated) {
                eprintln!("cannot write {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
            changed += 1;
        }
    }
    println!("updated {changed} file(s)");
    ExitCode::SUCCESS
}

/// The repo root is the directory holding `diagrams/`, searched upwards from the cwd.
fn repo_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("cwd");
    loop {
        if dir.join("diagrams").is_dir() && dir.join("docs").is_dir() {
            return dir;
        }
        if !dir.pop() {
            eprintln!("run from inside the llm_brain repository");
            std::process::exit(1);
        }
    }
}

fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_md(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// Rewrites every marked mermaid block; unmarked blocks are left untouched.
fn sync(text: &str, diagrams: &Path) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let marked = line
            .strip_prefix(MARK_START)
            .and_then(|rest| rest.strip_suffix(MARK_END))
            .filter(|_| lines.get(i + 1).is_some_and(|l| l.trim_end() == FENCE));
        match marked {
            Some(name) => {
                let src = diagrams.join(format!("{name}.mmd"));
                let body = fs::read_to_string(&src)
                    .map_err(|e| format!("missing diagram {}: {e}", src.display()))?;
                // skip the old block: fence, body, closing fence
                let mut j = i + 2;
                while j < lines.len() && lines[j].trim_end() != "```" {
                    j += 1;
                }
                if j >= lines.len() {
                    return Err(format!("unterminated mermaid block after {line}"));
                }
                out.push(line.to_string());
                out.push(FENCE.to_string());
                out.push(body.trim_end().to_string());
                out.push("```".to_string());
                i = j + 1;
            }
            None => {
                out.push(line.to_string());
                i += 1;
            }
        }
    }
    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_marked_block_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("sync-diagrams-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("x.mmd"), "flowchart LR\n  A --> B\n").unwrap();
        let input = "# T\n\n{/* diagram: x */}\n```mermaid\n```\n\n```mermaid\nkeep\n```\n";
        let got = sync(input, &dir).unwrap();
        assert_eq!(
            got,
            "# T\n\n{/* diagram: x */}\n```mermaid\nflowchart LR\n  A --> B\n```\n\n```mermaid\nkeep\n```\n"
        );
        assert_eq!(sync(&got, &dir).unwrap(), got, "idempotent");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_diagram_is_an_error() {
        let input = "{/* diagram: nope */}\n```mermaid\n```\n";
        assert!(sync(input, Path::new("/nonexistent")).is_err());
    }
}
