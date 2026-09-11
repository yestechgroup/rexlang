//! `rexlang` — command-line front end for the rexlang compiler driver.
//!
//! Subcommands:
//!
//! * `rexlang check <file>` — compile and render diagnostics; exits `1` on
//!   errors. Accepts `.mox` models and `.actor` policy files (the latter
//!   compile against the domain models they import).
//! * `rexlang ir <file> [-o <out>]` — compile and emit the Core IR JSON to
//!   stdout or to a file; exits `1` on errors. On `.actor` files the
//!   standalone ActorModel artifact is emitted.
//! * `rexlang vocab fetch <file> [--provider file:<DIR>|http]` — fetch and
//!   vendor vocabulary snapshots, updating `model.lock`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rex_driver::{compile_actors_str, compile_str, render, ActorCompilation, Compilation};
use rex_vocab::{FileProvider, HttpProvider, LockEntry, Lockfile, VocabularyProvider};
/// rexlang compiler command-line interface.
#[derive(Debug, Parser)]
#[command(name = "rexlang", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse, resolve, and validate a `.mox` or `.actor` file, rendering
    /// diagnostics.
    Check {
        /// Path to the `.mox` or `.actor` source file.
        file: PathBuf,
    },
    /// Compile a `.mox` or `.actor` file to the Core IR and emit it as JSON
    /// (`.actor` files emit the standalone ActorModel artifact).
    Ir {
        /// Path to the `.mox` or `.actor` source file.
        file: PathBuf,
        /// Write the JSON to this path instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Code generation from a compiled `.mox` file.
    Gen {
        #[command(subcommand)]
        target: GenTarget,
    },
    /// Format `.mox` files in place, preserving comments.
    Fmt {
        /// List files whose formatting would change instead of rewriting
        /// them; exits `1` when any file is unformatted.
        #[arg(long)]
        check: bool,
        /// Files to format, or `-` to read stdin and write stdout. With no
        /// files, stdin is formatted.
        files: Vec<PathBuf>,
    },
    /// Serve the rexlang language server over stdin/stdout.
    Lsp,
    /// Vocabulary snapshot tooling.
    Vocab {
        #[command(subcommand)]
        action: VocabAction,
    },
}

#[derive(Debug, Subcommand)]
enum VocabAction {
    /// Fetch each declared vocabulary snapshot, vendor it next to the model,
    /// and pin its digest in `model.lock`.
    Fetch {
        /// Path to the `.mox` source file.
        file: PathBuf,
        /// `file:<DIR>` to fetch from a directory of snapshots, or `http` to
        /// fetch from `--url-template`. Defaults to `file:<model-dir>/vocab`.
        #[arg(long, value_name = "PROVIDER")]
        provider: Option<String>,
        /// URL template for `--provider http`, with `{source}` and
        /// `{version}` placeholders.
        #[arg(long, value_name = "TEMPLATE")]
        url_template: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum GenTarget {
    /// Generate arena-based Rust model code (`models.rs`).
    Rust {
        /// Path to the `.mox` source file.
        file: PathBuf,
        /// Directory to write generated files into.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Generate JSON Schema (draft 2020-12) for the model (`schema.json`).
    JsonSchema {
        /// Path to the `.mox` source file.
        file: PathBuf,
        /// Which schema flavor to emit.
        #[arg(long, value_enum, default_value_t = SchemaProfile::Wire)]
        profile: SchemaProfile,
        /// Directory to write generated files into.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Generate Cedar policies + schema for the `actors` blocks of a
    /// `.mox` file, or of a `.actor` file plus its imported domain models
    /// (`<Block>.cedar` + `<Block>.cedarschema.json`).
    Cedar {
        /// Path to the `.mox` or `.actor` source file.
        file: PathBuf,
        /// Directory to write generated files into.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
    },
}

/// The JSON Schema flavor to generate.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SchemaProfile {
    /// Validates the canonical instance document (envelope, ids, `$ref`s).
    Wire,
    /// Flattened DTO projection for OpenAPI-style consumers.
    Api,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    match cli.command {
        Command::Check { file } => {
            if is_actor_file(&file) {
                let pair = read_actor_pair(&file)?;
                let compilation = compile_actors_str(&pair.path, &pair.source, &pair.domains);
                report_actor_diagnostics(&pair, &compilation);
                if compilation.model.is_some() {
                    println!("OK {}", pair.path);
                    Ok(ExitCode::SUCCESS)
                } else {
                    Ok(ExitCode::FAILURE)
                }
            } else {
                let (path, source) = read_source(&file)?;
                let compilation = compile_str(&path, &source);
                report_diagnostics(&path, &source, &compilation);
                if compilation.model.is_some() {
                    println!("OK {path}");
                    Ok(ExitCode::SUCCESS)
                } else {
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        Command::Ir { file, out } => {
            if is_actor_file(&file) {
                let pair = read_actor_pair(&file)?;
                let compilation = compile_actors_str(&pair.path, &pair.source, &pair.domains);
                report_actor_diagnostics(&pair, &compilation);
                let Some(actor_model) = compilation.model else {
                    return Ok(ExitCode::FAILURE);
                };
                let json = actor_model.to_json_pretty()?;
                match out {
                    Some(out_path) => std::fs::write(out_path, json)?,
                    None => println!("{json}"),
                }
                Ok(ExitCode::SUCCESS)
            } else {
                let (path, source) = read_source(&file)?;
                let compilation = compile_str(&path, &source);
                report_diagnostics(&path, &source, &compilation);
                let Some(model) = compilation.model else {
                    return Ok(ExitCode::FAILURE);
                };
                let json = model.to_json_pretty()?;
                match out {
                    Some(out_path) => std::fs::write(out_path, json)?,
                    None => println!("{json}"),
                }
                Ok(ExitCode::SUCCESS)
            }
        }
        Command::Gen {
            target: GenTarget::Rust { file, out },
        } => {
            let (path, source) = read_source(&file)?;
            let compilation = compile_str(&path, &source);
            report_diagnostics(&path, &source, &compilation);
            let Some(model) = compilation.model else {
                return Ok(ExitCode::FAILURE);
            };
            rex_backend_rust::generate_to_dir(&model, &out)?;
            println!("generated Rust model code into {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Gen {
            target: GenTarget::JsonSchema { file, profile, out },
        } => {
            let (path, source) = read_source(&file)?;
            let compilation = compile_str(&path, &source);
            report_diagnostics(&path, &source, &compilation);
            let Some(model) = compilation.model else {
                return Ok(ExitCode::FAILURE);
            };
            let profile = match profile {
                SchemaProfile::Wire => rex_backend_jsonschema::Profile::Wire,
                SchemaProfile::Api => rex_backend_jsonschema::Profile::Api,
            };
            rex_backend_jsonschema::generate_to_dir(&model, profile, &out)?;
            println!("generated JSON Schema into {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Gen {
            target: GenTarget::Cedar { file, out },
        } => {
            if is_actor_file(&file) {
                let pair = read_actor_pair(&file)?;
                let compilation = compile_actors_str(&pair.path, &pair.source, &pair.domains);
                report_actor_diagnostics(&pair, &compilation);
                let Some(actor_model) = compilation.model else {
                    return Ok(ExitCode::FAILURE);
                };
                let domain_model = merged_domain_model(&pair.domains);
                rex_backend_cedar::generate_to_dir(&actor_model, &domain_model, &out)?;
                println!("generated Cedar policies and schema into {}", out.display());
                Ok(ExitCode::SUCCESS)
            } else {
                let (path, source) = read_source(&file)?;
                let compilation = compile_str(&path, &source);
                report_diagnostics(&path, &source, &compilation);
                let Some(model) = compilation.model else {
                    return Ok(ExitCode::FAILURE);
                };
                let files = rex_backend_cedar::generate_for_model(&model)?;
                std::fs::create_dir_all(&out)?;
                for (relative, contents) in &files {
                    std::fs::write(out.join(relative), contents)?;
                }
                println!("generated Cedar policies and schema into {}", out.display());
                Ok(ExitCode::SUCCESS)
            }
        }
        Command::Fmt { check, files } => {
            if files.is_empty() || files.iter().any(|file| file == Path::new("-")) {
                let mut source = String::new();
                std::io::stdin()
                    .read_to_string(&mut source)
                    .map_err(|error| anyhow::anyhow!("cannot read stdin: {error}"))?;
                let formatted = rex_syntax::fmt::format(&source)?;
                std::io::stdout()
                    .write_all(formatted.as_bytes())
                    .map_err(|error| anyhow::anyhow!("cannot write stdout: {error}"))?;
                return Ok(ExitCode::SUCCESS);
            }
            let mut unformatted = false;
            for file in &files {
                let source = std::fs::read_to_string(file)
                    .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
                let formatted = if is_actor_file(file) {
                    rex_syntax::fmt::format_actors(&source)?
                } else {
                    rex_syntax::fmt::format(&source)?
                };
                if formatted != source {
                    unformatted = true;
                    if check {
                        println!("would reformat: {}", file.display());
                    } else {
                        std::fs::write(file, formatted).map_err(|error| {
                            anyhow::anyhow!("cannot write {}: {error}", file.display())
                        })?;
                    }
                }
            }
            if check && unformatted {
                Ok(ExitCode::FAILURE)
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }
        Command::Vocab {
            action:
                VocabAction::Fetch {
                    file,
                    provider,
                    url_template,
                },
        } => {
            vocab_fetch(&file, provider.as_deref(), url_template.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Lsp => {
            rex_lsp::run_stdio()?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// One `vocabulary` declaration as `vocab fetch` needs it: the vocabulary's
/// name plus everything providers and validation work with.
struct DeclaredVocabulary {
    name: String,
    declaration: rex_vocab::VocabularyDeclaration,
}

/// Extracts `vocabulary` declarations from a parsed model, checking the
/// shape `vocab fetch` depends on (key present, primitive facet types).
fn declared_vocabularies(model: &rex_syntax::Model) -> anyhow::Result<Vec<DeclaredVocabulary>> {
    let mut out = Vec::new();
    for decl in &model.declarations {
        let rex_syntax::Decl::Vocabulary(vocab) = decl else {
            continue;
        };
        let name = vocab.name.text.clone();

        let Some(key) = &vocab.key else {
            anyhow::bail!(
                "vocabulary '{name}' is missing its `key` declaration; add `key <facetName>` \
                 naming the field that uniquely identifies each entry"
            );
        };

        let mut facets = Vec::new();
        for facet in &vocab.facets {
            let Some(type_) = primitive_facet_type(&facet.type_ref) else {
                anyhow::bail!(
                    "facet '{}' of vocabulary '{name}' must have a primitive type, found '{}'; \
                     facet types must be primitives (String/int/long/short/float/double/boolean/byte/char)",
                    facet.name.text,
                    facet.type_ref.name.full_name()
                );
            };
            facets.push(rex_vocab::VocabularyFacet {
                name: facet.name.text.clone(),
                type_,
            });
        }

        out.push(DeclaredVocabulary {
            name,
            declaration: rex_vocab::VocabularyDeclaration {
                source: vocab.source.clone(),
                version: vocab.version.clone(),
                key: key.text.clone(),
                facets,
            },
        });
    }
    Ok(out)
}

/// The primitive type of a facet type reference, when it names one.
fn primitive_facet_type(type_ref: &rex_syntax::TypeRef) -> Option<rex_ir::PrimitiveType> {
    let [segment] = type_ref.name.segments.as_slice() else {
        return None;
    };
    match segment.text.to_ascii_lowercase().as_str() {
        "string" => Some(rex_ir::PrimitiveType::String),
        "int" => Some(rex_ir::PrimitiveType::Int),
        "long" => Some(rex_ir::PrimitiveType::Long),
        "short" => Some(rex_ir::PrimitiveType::Short),
        "float" => Some(rex_ir::PrimitiveType::Float),
        "double" => Some(rex_ir::PrimitiveType::Double),
        "boolean" => Some(rex_ir::PrimitiveType::Boolean),
        "byte" => Some(rex_ir::PrimitiveType::Byte),
        "char" => Some(rex_ir::PrimitiveType::Char),
        _ => None,
    }
}

/// Implements `rexlang vocab fetch`: fetches every declared vocabulary
/// snapshot from the provider, validates it against the declaration, vendors
/// it into `<model-dir>/vocab/`, and upserts its pin in
/// `<model-dir>/model.lock`.
///
/// Parsing is syntax-only (no driver compile): fetching must work before any
/// snapshots exist to compile against.
fn vocab_fetch(
    file: &Path,
    provider: Option<&str>,
    url_template: Option<&str>,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let parsed = rex_syntax::parse(&source);
    if !parsed.errors.is_empty() {
        anyhow::bail!(
            "cannot parse {}: {} syntax error(s); run `rexlang check {}` for diagnostics",
            file.display(),
            parsed.errors.len(),
            file.display()
        );
    }
    let model = parsed
        .ast
        .ok_or_else(|| anyhow::anyhow!("cannot parse {}", file.display()))?;
    let vocabularies = declared_vocabularies(&model)?;
    if vocabularies.is_empty() {
        println!("no vocabulary declarations in {}", file.display());
        return Ok(());
    }

    let model_dir = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let provider: Box<dyn VocabularyProvider> = match provider {
        None => Box::new(FileProvider {
            root: model_dir.join("vocab"),
        }),
        Some("http") => Box::new(HttpProvider {
            url_template: url_template
                .ok_or_else(|| {
                    anyhow::anyhow!("--provider http requires --url-template <TEMPLATE>")
                })?
                .to_string(),
        }),
        Some(spec) if spec.starts_with("file:") => Box::new(FileProvider {
            root: PathBuf::from(&spec["file:".len()..]),
        }),
        Some(other) => anyhow::bail!("unknown provider '{other}'; expected 'file:<DIR>' or 'http'"),
    };

    let lock_path = model_dir.join("model.lock");
    let mut lockfile = if lock_path.exists() {
        Lockfile::read(&lock_path)?
    } else {
        Lockfile::default()
    };

    for vocabulary in &vocabularies {
        let declaration = &vocabulary.declaration;
        let version = declaration
            .version
            .clone()
            .or_else(|| {
                lockfile
                    .entry_for(&declaration.source)
                    .map(|entry| entry.version.clone())
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "vocabulary '{}' has no version: pin a version with `version \"...\"` in the \
                     declaration, or fetch once from a lockfile that pins one",
                    vocabulary.name
                )
            })?;

        let snapshot = provider.fetch(&declaration.source, Some(&version))?;
        let parsed_snapshot = rex_vocab::parse_snapshot(&snapshot.bytes).map_err(|error| {
            anyhow::anyhow!(
                "snapshot for vocabulary '{}' is invalid: {error}",
                vocabulary.name
            )
        })?;
        let entries =
            rex_vocab::validate_entries(&parsed_snapshot, declaration).map_err(|error| {
                anyhow::anyhow!(
                    "snapshot for vocabulary '{}' is invalid: {error}",
                    vocabulary.name
                )
            })?;

        let vocab_dir = model_dir.join("vocab");
        std::fs::create_dir_all(&vocab_dir)?;
        let file_name = format!(
            "{}@{}.json",
            rex_vocab::sanitize_source(&declaration.source),
            snapshot.version
        );
        std::fs::write(vocab_dir.join(&file_name), &snapshot.bytes)?;

        let digest = rex_vocab::digest(&snapshot.bytes);
        lockfile.insert(LockEntry {
            source: declaration.source.clone(),
            version: snapshot.version.clone(),
            digest: digest.clone(),
            fetched_at: rex_vocab::rfc3339_now(),
        });

        // `vendored iso:4217@2024-01-01 (5 entries, sha256:0123456789abcdef…)`.
        println!(
            "vendored {source}@{version} ({count} entries, {digest_prefix}…)",
            source = declaration.source,
            version = snapshot.version,
            count = entries.len(),
            digest_prefix = &digest[..digest.len().min(7 + 16)],
        );
    }

    lockfile.write(&lock_path)?;
    Ok(())
}

fn read_source(file: &PathBuf) -> anyhow::Result<(String, String)> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    Ok((file.display().to_string(), source))
}

/// `true` for `.actor` paths: the standalone actor-policy surface, compiled
/// against the domain models it imports.
fn is_actor_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("actor")
}

/// An `.actor` file plus the domain sources it imports, ready for
/// [`rex_driver::compile_actors_str`].
struct ActorPair {
    /// The actor file's path as given on the command line.
    path: String,
    /// The actor file's source text.
    source: String,
    /// Each imported domain as `(import path as written, source)`; the
    /// driver resolves an actor-file import by that exact string.
    domains: Vec<(String, String)>,
}

impl ActorPair {
    /// The source text of the file with the given driver path, if it is the
    /// actor file or an imported domain.
    fn source_of(&self, path: &str) -> Option<&str> {
        if path == self.path {
            return Some(&self.source);
        }
        self.domains
            .iter()
            .find(|(name, _)| name == path)
            .map(|(_, source)| source.as_str())
    }
}

/// Reads an `.actor` file plus every domain it imports.
///
/// Import paths resolve relative to the actor file's own directory, but are
/// passed to the driver **exactly as written** (the driver matches imports
/// by that string). A missing import file is a clean error naming the
/// resolved path. Duplicate imports are read once.
fn read_actor_pair(file: &Path) -> anyhow::Result<ActorPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut domains: Vec<(String, String)> = Vec::new();
    if let Some(ast) = &rex_syntax::parse_actors(&source).ast {
        let dir = file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for import in &ast.imports {
            if domains.iter().any(|(existing, _)| *existing == import.path) {
                continue;
            }
            let resolved = dir.join(&import.path);
            let text = std::fs::read_to_string(&resolved).map_err(|error| {
                anyhow::anyhow!(
                    "cannot read imported file {} (imported by {}): {error}",
                    resolved.display(),
                    path
                )
            })?;
            domains.push((import.path.clone(), text));
        }
    }
    Ok(ActorPair {
        path,
        source,
        domains,
    })
}

/// Merges the lowered models of every imported domain into one domain model
/// for Cedar class lookup: packages keep first-appearance order; same-named
/// packages merge their classes. Domains that failed to compile contribute
/// nothing (their diagnostics already failed the actor compilation).
fn merged_domain_model(domains: &[(String, String)]) -> rex_ir::Model {
    let mut merged = rex_ir::Model::new();
    for (path, source) in domains {
        let Some(model) = compile_str(path, source).model else {
            continue;
        };
        for package in model.packages {
            match merged
                .packages
                .iter_mut()
                .find(|existing| existing.name == package.name)
            {
                Some(existing) => existing.classes.extend(package.classes),
                None => merged.packages.push(package),
            }
        }
    }
    merged
}

fn report_diagnostics(path: &str, source: &str, compilation: &Compilation) {
    if !compilation.diagnostics.is_empty() {
        eprint!("{}", render(path, source, &compilation.diagnostics));
    }
}

/// Renders an actor compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics.
fn report_actor_diagnostics(pair: &ActorPair, compilation: &ActorCompilation) {
    if compilation.diagnostics.is_empty() {
        return;
    }
    let mut groups: Vec<(&str, Vec<rex_driver::Diagnostic>)> = Vec::new();
    for (path, diagnostic) in &compilation.diagnostics {
        match groups.last_mut() {
            Some((group_path, group)) if *group_path == path.as_str() => {
                group.push(diagnostic.clone())
            }
            _ => groups.push((path.as_str(), vec![diagnostic.clone()])),
        }
    }
    for (path, group) in groups {
        let Some(source) = pair.source_of(path) else {
            continue;
        };
        eprint!("{}", render(path, source, &group));
    }
}
