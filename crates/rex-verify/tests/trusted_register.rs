//! Mechanical register↔code drift gate (issue #203, extending #89).
//!
//! `docs/VERIFICATION.md`'s tables are generated from the
//! machine-readable register source `docs/verification/register.json`
//! (data model + validation + rendering in `rex-evidence`), and the
//! register must agree with the tree it describes. This test enforces
//! the agreement:
//!
//! 1. the register parses and passes `rex_evidence::validate` — closed
//!    status/evidence vocabularies (`"verified"` is never a status),
//!    pointer grammar, review-schedule discipline, status↔evidence
//!    joins, hold completeness, trusted-row references;
//! 2. every evidence pointer resolves: the file exists under the
//!    workspace root, a `path:symbol` symbol occurs in that file, and
//!    `proof`/`test` pointers' symbols are `fn` items;
//! 3. the `trusted[]` list is **bidirectionally** consistent with the
//!    tree: every `#[trusted]` item (textual scan, 40-line adjacency
//!    rule — the #89 scanner) appears in the register, and every
//!    registered item is an actual `#[trusted]` in the tree;
//! 4. the rendered register block byte-equals the span between the
//!    markers in `docs/VERIFICATION.md` — regenerate (rewriting only
//!    that span) with `REX_UPDATE_FIXTURES=1 cargo test -p rex-verify`.
//!
//! Adding a stray `#[trusted]` anywhere fails with the item name; a dead
//! pointer, a status without its evidence kind, or a hand-edited table
//! fails just as loudly. Shrinking the trusted set to zero would
//! trivially pass — that is the policy's goal.

use std::path::{Path, PathBuf};

use rex_evidence::{parse, render_markdown, validate};

/// The machine-readable register source (the tables' single home).
const REGISTER_JSON: &str = "docs/verification/register.json";

/// The document whose tables are generated from the register source.
const DOC: &str = "docs/VERIFICATION.md";

/// Exactly the delimiters the renderer's consumer splices between.
const BEGIN_MARKER: &str = "<!-- BEGIN GENERATED REGISTER (source: docs/verification/register.json; regenerate: REX_UPDATE_FIXTURES=1 cargo test -p rex-verify) -->";
const END_MARKER: &str = "<!-- END GENERATED REGISTER -->";

/// Directories never scanned: build output, generated Coma, VCS internals.
const SKIPPED_DIRS: [&str; 3] = ["target", "verif", ".git"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("rex-verify lives two levels below the workspace root")
        .to_path_buf()
}

/// Loads and fully validates the register source.
fn load_register() -> rex_evidence::Register {
    let root = workspace_root();
    let path = root.join(REGISTER_JSON);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {} ({e})", path.display()));
    let register =
        parse(&text).unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()));
    let violations = validate(&register);
    assert!(
        violations.is_empty(),
        "{} violates the register rules:\n{}",
        path.display(),
        violations
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
    register
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
/// first non-attribute, non-comment line within 40 lines of `#[trusted]`.
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

/// Scans the tree for every `#[trusted]`-attributed item: `(name, file:line)`.
fn scan_trusted_items(root: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    collect_rs(root, &mut files);
    let kernel = root.join("crates/rex-verify/src/lib.rs");
    assert!(
        files.iter().any(|p| p == &kernel),
        "the scanner must visit the kernel sources; walked {} files from {}",
        files.len(),
        root.display()
    );

    let mut found = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("unreadable source {} ({e})", path.display()));
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim() != "#[trusted]" {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(path).display();
            match trusted_item_name(&lines, i) {
                Some(name) => found.push((name, format!("{rel}:{}", i + 1))),
                None => panic!(
                    "`#[trusted]` at {rel}:{} has no `fn` item within 40 lines — keep the attributed item adjacent so the gate can name it",
                    i + 1
                ),
            }
        }
    }
    found
}

#[test]
fn the_register_parses_and_validates() {
    load_register();
}

#[test]
fn every_evidence_pointer_resolves() {
    let root = workspace_root();
    let register = load_register();
    let mut dead = Vec::new();
    for obligation in &register.obligations {
        for evidence in &obligation.evidence {
            // Same split the pointer grammar validated: a symbol tail
            // after the last `:`.
            let (path, symbol) = match evidence.pointer.rsplit_once(':') {
                Some((p, s))
                    if !s.is_empty()
                        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
                {
                    (p, Some(s))
                }
                _ => (evidence.pointer.as_str(), None),
            };
            let file = root.join(path);
            if !file.starts_with(&root) || !file.is_file() {
                dead.push(format!(
                    "obligation {}: {} pointer {:?} — no such file under the workspace root",
                    obligation.id, evidence.kind, evidence.pointer
                ));
                continue;
            }
            let Some(symbol) = symbol else {
                continue;
            };
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("unreadable {} ({e})", file.display()));
            let wants_fn = matches!(evidence.kind.as_str(), "test" | "proof");
            let needle = if wants_fn {
                format!("fn {symbol}")
            } else {
                symbol.to_string()
            };
            if !text.contains(&needle) {
                let expected = if wants_fn {
                    "a `fn` item"
                } else {
                    "the symbol verbatim"
                };
                dead.push(format!(
                    "obligation {}: {} pointer {:?} — {} not found in {}",
                    obligation.id, evidence.kind, evidence.pointer, expected, path
                ));
            }
        }
    }
    assert!(
        dead.is_empty(),
        "the register's evidence does not resolve:\n{}",
        dead.join("\n")
    );
}

#[test]
fn every_trusted_item_has_a_register_row() {
    let root = workspace_root();
    let register = load_register();
    let found = scan_trusted_items(&root);

    let mut names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    for window in names.windows(2) {
        assert!(
            window[0] != window[1],
            "two `#[trusted]` items share the name {:?} — the register maps items by name, so names must be unique",
            window[0]
        );
    }

    let offenders: Vec<String> = found
        .iter()
        .filter(|(name, _)| !register.trusted.iter().any(|row| row.item == *name))
        .map(|(name, location)| {
            format!(
                "unregistered `#[trusted]` item `{name}` at {location} — add a trusted row for `{name}` in docs/verification/register.json or un-trust it"
            )
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "the `#[trusted]` policy is violated:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn every_register_trusted_item_is_in_the_tree() {
    let root = workspace_root();
    let register = load_register();
    let found = scan_trusted_items(&root);

    let offenders: Vec<String> = register
        .trusted
        .iter()
        .filter(|row| !found.iter().any(|(name, _)| name == &row.item))
        .map(|row| {
            format!(
                "register trusted item `{item}` (row {row}, reason: {reason}) is not a `#[trusted]` in the tree — trust it in code or drop the row",
                item = row.item,
                row = row.row,
                reason = row.reason
            )
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "the register's trusted list has stale entries:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn docs_block_matches_the_rendered_register() {
    let root = workspace_root();
    let register = load_register();
    let rendered = render_markdown(&register);

    let doc_path = root.join(DOC);
    let doc = std::fs::read_to_string(&doc_path)
        .unwrap_or_else(|e| panic!("cannot read {} ({e})", doc_path.display()));
    let begin_start = doc
        .find(BEGIN_MARKER)
        .unwrap_or_else(|| panic!("{DOC} lost its BEGIN GENERATED REGISTER marker"));
    let end_start = doc
        .find(END_MARKER)
        .unwrap_or_else(|| panic!("{DOC} lost its END GENERATED REGISTER marker"));
    assert!(
        doc.rfind(BEGIN_MARKER) == Some(begin_start) && doc.rfind(END_MARKER) == Some(end_start),
        "{DOC} must contain exactly one pair of register markers"
    );
    let begin_end = begin_start + BEGIN_MARKER.len();
    assert!(
        begin_end <= end_start,
        "{DOC}'s register markers are inverted"
    );

    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        // Splice only the marker span — the rest of the document is
        // hand-written prose and must not be touched.
        let updated = format!("{}\n{}{}", &doc[..begin_end], rendered, &doc[end_start..]);
        std::fs::write(&doc_path, updated).expect("write docs/VERIFICATION.md");
        return;
    }
    let on_disk = &doc[begin_end + 1..end_start];
    assert_eq!(
        rendered, on_disk,
        "the generated block in {DOC} drifted from {REGISTER_JSON} — regenerate with REX_UPDATE_FIXTURES=1 cargo test -p rex-verify (hand-edit the JSON, never the tables)"
    );
}
