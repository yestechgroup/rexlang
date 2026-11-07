//! Mechanical `#[trusted]` register-coverage gate (issue #89).
//!
//! `docs/VERIFICATION.md`'s policy says every `#[trusted]` in the tree must
//! have a row in the proof-obligation register ("an assumption, not a
//! proof"). Until now that was enforced by review only; this test scans all
//! workspace `*.rs` sources for `#[trusted]`-attributed functions and
//! asserts each item name appears in the register.
//!
//! The check is deliberately textual (grep-level): a line that trims to
//! exactly `#[trusted]` opens an item, and the attributed `fn`'s name is
//! taken from the following item declaration. Adding a stray `#[trusted]`
//! anywhere fails with the item name in the message; shrinking the trusted
//! set to zero would trivially pass — that is the policy's goal.

use std::path::{Path, PathBuf};

/// The register every `#[trusted]` must appear in.
const REGISTER: &str = "docs/VERIFICATION.md";

/// Directories never scanned: build output, generated Coma, VCS internals.
const SKIPPED_DIRS: [&str; 3] = ["target", "verif", ".git"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("rex-verify lives two levels below the workspace root")
        .to_path_buf()
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if SKIPPED_DIRS.contains(&name) {
                continue;
            }
            collect_rs(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

/// The attributed item's name: the identifier after the `fn` token on the
/// first non-attribute, non-comment line following `#[trusted]`.
fn trusted_item_name(lines: &[&str], attr_line: usize) -> Option<String> {
    let end = (attr_line + 41).min(lines.len());
    for line in &lines[attr_line + 1..end] {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("#[") {
            continue;
        }
        let mut tokens = trimmed.split_whitespace();
        while let Some(token) = tokens.next() {
            if token == "fn" {
                if let Some(next) = tokens.next() {
                    let name: String = next
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }
    }
    None
}

#[test]
fn every_trusted_item_has_a_register_row() {
    let root = workspace_root();
    let register_path = root.join(REGISTER);
    let register = std::fs::read_to_string(&register_path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}) — the `#[trusted]` gate needs the proof-obligation register",
            register_path.display()
        )
    });

    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    let kernel = root.join("crates/rex-verify/src/lib.rs");
    assert!(
        files.iter().any(|p| p == &kernel),
        "the scanner must visit the kernel sources; walked {} files from {}",
        files.len(),
        root.display()
    );

    let mut offenders = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("unreadable source {} ({e})", path.display()));
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim() != "#[trusted]" {
                continue;
            }
            let rel = path.strip_prefix(&root).unwrap_or(path).display();
            match trusted_item_name(&lines, i) {
                Some(name) if register.contains(&name) => {}
                Some(name) => offenders.push(format!(
                    "unregistered `#[trusted]` item `{name}` at {rel}:{} — add a register row for `{name}` in docs/VERIFICATION.md or un-trust it",
                    i + 1
                )),
                None => offenders.push(format!(
                    "`#[trusted]` at {rel}:{} has no `fn` item within 40 lines — keep the attributed item adjacent so the gate can name it",
                    i + 1
                )),
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the `#[trusted]` policy is violated:\n{}",
        offenders.join("\n")
    );
}
