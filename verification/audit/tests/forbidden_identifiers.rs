use std::fs;
use std::path::Path;

use fireline_audit::{
    strict_audit_enabled, workspace_root, ALLOW_LEGACY_HEADER, FORBIDDEN_IDENTIFIERS,
};
use regex::Regex;
use walkdir::WalkDir;

fn main() {
    if !strict_audit_enabled() {
        eprintln!("fireline-audit: strict-audit off, skipping forbidden-identifier grep");
        return;
    }

    let root = workspace_root();
    let targets = [
        root.join("crates/fireline-session/src"),
        root.join("crates/fireline-harness/src/state_projector.rs"),
        root.join("packages/state/src/schema.ts"),
    ];

    let mut violations = Vec::new();
    for target in targets {
        collect_violations(&target, &mut violations);
    }

    if !violations.is_empty() {
        panic!(
            "forbidden identifiers remain in agent-layer sources:\n\n{}",
            violations.join("\n")
        );
    }
}

fn collect_violations(path: &Path, violations: &mut Vec<String>) {
    if path.is_dir() {
        for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
            if entry.file_type().is_file() {
                scan_file(entry.path(), violations);
            }
        }
        return;
    }

    if path.is_file() {
        scan_file(path, violations);
    }
}

fn scan_file(path: &Path, violations: &mut Vec<String>) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            violations.push(format!("{}: could not read file: {error}", path.display()));
            return;
        }
    };

    if text.contains(ALLOW_LEGACY_HEADER) {
        return;
    }

    for token in FORBIDDEN_IDENTIFIERS {
        let pattern = Regex::new(&format!(r"\b{}\b", regex::escape(token))).expect("valid regex");
        for (line_number, line) in text.lines().enumerate() {
            if pattern.is_match(line) {
                violations.push(format!(
                    "{}:{}: forbidden identifier `{}`",
                    display_relative(path),
                    line_number + 1,
                    token
                ));
            }
        }
    }
}

fn display_relative(path: &Path) -> String {
    let root = workspace_root();
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
