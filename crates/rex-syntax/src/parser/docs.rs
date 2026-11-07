//! Doc-comment attachment: leading `///` runs are collected per
//! source line and attached to the declarations they precede,
//! for `.mox`, `.ddd`, `.uc`, and `.dmn` alike.

use super::*;
/// One precomputed comment fact used by doc attachment.
pub(crate) struct DocComment {
    content: Option<String>,
    start_line: usize,
    end_line: usize,
    /// Whether only whitespace precedes the comment on its first line.
    begins_line: bool,
}

/// The byte offsets at which each line of `source` starts.
pub(crate) fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

/// The 0-based line containing the given byte offset.
pub(crate) fn line_of(byte: usize, starts: &[usize]) -> usize {
    starts
        .partition_point(|&start| start <= byte)
        .saturating_sub(1)
}

/// Precomputes the doc-comment facts of a comment list against its source.
pub(crate) fn collect_docs(
    comments: &[crate::lexer::Comment<'_>],
    source: &str,
) -> Vec<DocComment> {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    comments
        .iter()
        .map(|comment| DocComment {
            content: comment.doc_content().filter(|content| !content.is_empty()),
            start_line: line_of(comment.span.start),
            end_line: line_of(comment.span.end.saturating_sub(1)),
            begins_line: source[starts[line_of(comment.span.start)]..comment.span.start]
                .bytes()
                .all(|byte| byte.is_ascii_whitespace()),
        })
        .collect()
}

/// The joined description of the doc run ending directly above `start_line`,
/// or `None`. A doc run is a maximal sequence of doc comments, each on its
/// own line, each starting on the line directly after the previous one ends,
/// whose last comment ends on the line directly above the declaration's
/// first line.
pub(crate) fn doc_run_above(docs: &[DocComment], start_line: usize) -> Option<String> {
    let (last_index, _) = docs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, comment)| comment.end_line + 1 == start_line && comment.content.is_some())?;
    if !docs[last_index].begins_line {
        return None;
    }
    // Walk back through the contiguous doc run above the last comment.
    let mut first = last_index;
    while first > 0 {
        let previous = &docs[first - 1];
        let current = &docs[first];
        let contiguous = previous.end_line + 1 == current.start_line;
        if !(contiguous && previous.content.is_some() && previous.begins_line) {
            break;
        }
        first -= 1;
    }
    let joined = docs[first..=last_index]
        .iter()
        .map(|comment| {
            comment
                .content
                .as_deref()
                .expect("run members are doc comments")
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!joined.is_empty()).then_some(joined)
}

/// Attaches doc comments to the model's package declaration, declarations,
/// features, and enum literals.
pub(crate) fn attach_docs(model: &mut Model, comments: &[crate::lexer::Comment<'_>], source: &str) {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    let docs = collect_docs(comments, source);

    if let Some(package) = &mut model.package {
        let start_line = line_of(package.span.start);
        package.doc = doc_run_above(&docs, start_line);
    }

    for decl in &mut model.declarations {
        let start_line = line_of(decl.span().start);
        match decl {
            Decl::Class(decl) => {
                decl.doc = doc_run_above(&docs, start_line);
                for feature in &mut decl.features {
                    let feature_line = line_of(feature.span().start);
                    if feature.doc().is_none() {
                        feature.set_doc(doc_run_above(&docs, feature_line));
                    }
                }
            }
            Decl::Interface(decl) => decl.doc = doc_run_above(&docs, start_line),
            Decl::Enum(decl) => {
                decl.doc = doc_run_above(&docs, start_line);
                for literal in &mut decl.literals {
                    let literal_line = line_of(literal.span.start);
                    if literal.doc.is_none() {
                        literal.doc = doc_run_above(&docs, literal_line);
                    }
                }
            }
            Decl::Datatype(decl) => decl.doc = doc_run_above(&docs, start_line),
            Decl::Vocabulary(decl) => decl.doc = doc_run_above(&docs, start_line),
            Decl::Annotation(_) | Decl::Actors(_) | Decl::ImportSchema(_) => {}
        }
    }
}

/// Attaches doc comments to a `.ddd` file's services and searches. The
/// contiguous `///` run is joined with newlines into a single description —
/// the same semantic as the `.mox` doc collection ([`doc_run_above`] is
/// shared; no re-joining happens here).
pub(crate) fn attach_ddd_docs(
    file: &mut DddFile,
    comments: &[crate::lexer::Comment<'_>],
    source: &str,
) {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    let docs = collect_docs(comments, source);
    if let Some(application) = &mut file.application {
        for module in &mut application.modules {
            for service in &mut module.services {
                let start_line = line_of(service.span.start);
                service.doc = doc_run_above(&docs, start_line);
            }
            for search in &mut module.searches {
                let start_line = line_of(search.span.start);
                search.doc = doc_run_above(&docs, start_line);
            }
        }
    }
}

/// Attaches doc comments to a `.uc` file's usecases. The contiguous `///`
/// run is joined with newlines into a single description — the same
/// semantic as the `.mox` doc collection ([`doc_run_above`] is shared).
pub(crate) fn attach_uc_docs(
    file: &mut UcFile,
    comments: &[crate::lexer::Comment<'_>],
    source: &str,
) {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    let docs = collect_docs(comments, source);
    for decl in &mut file.use_cases {
        let start_line = line_of(decl.span.start);
        decl.description = doc_run_above(&docs, start_line);
    }
}

/// Attaches doc comments to a `.dmn` file's knowledge sources, models
/// (BKMs), decisions, services, and views. The contiguous `///` run is
/// joined with newlines into a single description — the same semantic as
/// the `.mox` doc collection ([`doc_run_above`] is shared).
pub(crate) fn attach_dmn_docs(
    file: &mut DmnFile,
    comments: &[crate::lexer::Comment<'_>],
    source: &str,
) {
    let starts = line_starts(source);
    let line_of = |byte: usize| line_of(byte, &starts);
    let docs = collect_docs(comments, source);
    if let Some(model) = &mut file.model {
        for source_decl in &mut model.knowledge_sources {
            let start_line = line_of(source_decl.span.start);
            source_decl.description = doc_run_above(&docs, start_line);
        }
        for bkm in &mut model.models {
            let start_line = line_of(bkm.span.start);
            bkm.description = doc_run_above(&docs, start_line);
        }
        for decision in &mut model.decisions {
            let start_line = line_of(decision.span.start);
            decision.description = doc_run_above(&docs, start_line);
        }
        for service in &mut model.services {
            let start_line = line_of(service.span.start);
            // A service's body `description` line wins; the doc run only
            // fills an absent description.
            if service.description.is_none() {
                service.description = doc_run_above(&docs, start_line);
            }
        }
        for view in &mut model.views {
            let start_line = line_of(view.span.start);
            if view.description.is_none() {
                view.description = doc_run_above(&docs, start_line);
            }
        }
    }
}
