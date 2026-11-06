//! The `.deploy` grammar: settings, components, connections,
//! applications, targets, policies, mappings, profiles,
//! deployments, and the file parser.

use crate::deploy;
use crate::lexer::{lex, Token};

use super::imports::import_decl;
use super::*;
/// A contextual `.deploy` keyword: an ordinary identifier that is special
/// only in the grammar position it is parsed here in (the `.ddd`
/// discipline). Escaped forms (`^component`) are different token variants
/// and never match.
pub(crate) fn deploy_keyword<'src>(
    keyword: &'static str,
) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { Token::Ident(text) = e if text == keyword => e.span() }
}

/// The `->` arrow of `connects` entries and mapping entries.
pub(crate) fn deploy_arrow<'src>() -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone
{
    kw(Token::Arrow)
}

/// The outcome of parsing a `.deploy` source: as much of the file as could
/// be recovered, plus all encountered errors. Parsing never panics and
/// always produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployParseResult {
    /// The recovered deployment file, or `None` if no output could be
    /// produced at all.
    pub ast: Option<DeployFile>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

/// Lex and parse a `.deploy` source text: `import` declarations followed by
/// `application`, `profile`, and `deployment` declarations (which may be
/// interleaved; imports must come first).
///
/// This function never panics and always recovers as much of the AST as
/// possible; check [`DeployParseResult::errors`] for syntax problems.
pub fn parse_deploy(source: &str) -> DeployParseResult {
    let tokens = match lex(source) {
        Ok(tokens) => tokens,
        Err(error) => {
            return DeployParseResult {
                ast: None,
                errors: vec![ParseError {
                    message: error.to_string(),
                    span: error.span,
                }],
            }
        }
    };
    let (ast, errors) = deploy_file()
        .parse(Tokens::new(&tokens))
        .into_output_errors();
    DeployParseResult {
        ast,
        errors: errors
            .into_iter()
            .map(|error| ParseError {
                message: error.to_string(),
                span: *error.span(),
            })
            .collect(),
    }
}

/// One `path: value` setting line. `path` is the qualified name as written
/// (a single segment inside a component body, a `component.setting` pair
/// inside `defaults`/`configure`). Committed once the path matched: a
/// missing `:` or value is a reported error and the line still recovers.
/// A bare-word value may be any keyword token (`container` is a `.mox`
/// keyword but an ordinary word in value position — the qname-segment
/// rule: keywords are names outside statement position).
pub(crate) fn deploy_setting<'src>(
) -> impl Parser<'src, Tokens<'src>, DeploySettingDecl, MoxExtra<'src>> + Clone {
    let word = name().or(
        any().try_map(|token: Token<'src>, span| match token.keyword() {
            Some(keyword) => Ok(Name {
                text: keyword.to_string(),
                span,
                escaped: false,
            }),
            None => Err(Rich::custom(span, "expected a setting value")),
        }),
    );
    let value = string_lit()
        .map(DeploySettingValue::Str)
        .or(int_lit().map(DeploySettingValue::Int))
        .or(word.map(DeploySettingValue::Word));
    qname()
        .then(kw(Token::Other(':')).or_not())
        .then(value.or_not())
        .map_with(|((path, colon), value), e| (path, colon, value, e.span()))
        .validate(|(path, colon, value, span), _e, emitter| {
            if colon.is_none() {
                emitter.emit(Rich::custom(
                    path.span,
                    "expected `:` after the setting path",
                ));
            }
            if value.is_none() {
                emitter.emit(Rich::custom(span, "expected a setting value"));
            }
            DeploySettingDecl {
                path,
                value: value.unwrap_or(DeploySettingValue::Word(Name {
                    text: String::new(),
                    span,
                    escaped: false,
                })),
                span,
            }
        })
}

/// A `requires <capability> (, <capability>)*` clause of a component body.
/// Committed once `requires` matched (the `capability`-clause precedent):
/// a missing name list is a reported error.
pub(crate) fn deploy_requires<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<Name>, MoxExtra<'src>> + Clone {
    deploy_keyword(deploy::REQUIRES)
        .ignore_then(name().or_not())
        .then(
            kw(Token::Comma)
                .ignore_then(name())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map_with(|(first, rest), e| (first, rest, e.span()))
        .validate(|(first, rest, span), _e, emitter| {
            if first.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "`requires` requires at least one capability name",
                ));
            }
            first.into_iter().chain(rest).collect::<Vec<_>>()
        })
}

/// A `design "<path>[#<Module>]"` or `flow "<path>[#<Module>]"` binding of
/// a component body. The reference is kept verbatim (escapes applied);
/// splitting at `#` is the driver's job.
pub(crate) fn deploy_binding<'src>(
    is_design: bool,
) -> impl Parser<'src, Tokens<'src>, DeployBindingDecl, MoxExtra<'src>> + Clone {
    let keyword = if is_design {
        deploy_keyword(deploy::DESIGN)
    } else {
        deploy_keyword(deploy::FLOW)
    };
    keyword
        .ignore_then(string_lit().or_not())
        .map_with(|reference, e| (reference, e.span()))
        .validate(move |(reference, span), _e, emitter| {
            if reference.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    if is_design {
                        "expected a design path string after `design`"
                    } else {
                        "expected a flow path string after `flow`"
                    },
                ));
            }
            DeployBindingDecl {
                reference: reference.unwrap_or_default(),
                is_design,
                span,
            }
        })
}

/// One `NAME = "value"` entry of an `env` block. The name is a bare word —
/// keyword tokens included (the qname-segment rule: keywords are names
/// outside statement position) — and the value is a quoted string, kept
/// verbatim. Committed once the `=` matched: a missing value is a reported
/// error and the entry still recovers. A bare word with no `=` is *not* an
/// entry: the entry loop ends there, so the enclosing component body
/// re-reads the word as its next member (whose own scanners report the
/// shape) — and a block whose `}` never came is reported by
/// [`deploy_env_block`].
pub(crate) fn deploy_env_entry<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployEnvEntry, MoxExtra<'src>> + Clone {
    let word = name().or(
        any().try_map(|token: Token<'src>, span| match token.keyword() {
            Some(keyword) => Ok(Name {
                text: keyword.to_string(),
                span,
                escaped: false,
            }),
            None => Err(Rich::custom(span, "expected an env name")),
        }),
    );
    word.then(kw(Token::Eq).ignore_then(string_lit().or_not()))
        .map_with(|(name, value), e| (name, value, e.span()))
        .validate(|(name, value, span), _e, emitter| {
            if value.is_none() {
                emitter.emit(Rich::custom(span, "expected a string value"));
            }
            DeployEnvEntry {
                name,
                value: value.unwrap_or_default(),
                span,
            }
        })
}

/// An `env { NAME = "value" ... }` block of a component body. Committed once
/// the `env` word matched: a missing `{` lets the member choice fall through
/// (the setting-line twin reports the shape), and a missing closing `}` is a
/// reported error while the block still lowers what it parsed — later
/// members keep parsing.
pub(crate) fn deploy_env_block<'src>(
) -> impl Parser<'src, Tokens<'src>, Vec<DeployEnvEntry>, MoxExtra<'src>> + Clone {
    deploy_keyword(deploy::ENV)
        .ignore_then(kw(Token::LBrace))
        .ignore_then(deploy_env_entry().repeated().collect::<Vec<_>>())
        .then(kw(Token::RBrace).or_not())
        .map_with(|(entries, close), e| (entries, close, e.span()))
        .validate(|(entries, close, span), _e, emitter| {
            if close.is_none() {
                emitter.emit(Rich::custom(span, "expected `}` to close the `env` block"));
            }
            entries
        })
}

/// One member of a component body: a `requires` clause, a design/flow
/// binding, an `env` block, or a setting line. The dedicated members are
/// tried first, so their words (`requires`, `design`, `flow`, `env`) are
/// never mistaken for setting names — escaping (`^requires`) is the way
/// out, like every contextual word.
pub(crate) fn deploy_component_member<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployComponentMember, MoxExtra<'src>> + Clone {
    choice((
        deploy_requires().map(DeployComponentMember::Requires),
        deploy_binding(true).map(DeployComponentMember::Design),
        deploy_binding(false).map(DeployComponentMember::Flow),
        deploy_env_block().map(DeployComponentMember::Env),
        deploy_setting().map(DeployComponentMember::Setting),
    ))
}

/// One member of a component body (the [`deploy_component_member`]
/// alternatives).
#[derive(Debug, Clone)]
pub(crate) enum DeployComponentMember {
    Requires(Vec<Name>),
    Design(DeployBindingDecl),
    Flow(DeployBindingDecl),
    Env(Vec<DeployEnvEntry>),
    Setting(DeploySettingDecl),
}

/// A `component <name>: <kind> { ... }` declaration. Committed once the
/// name matched: a missing kind is a reported error; the body is optional.
pub(crate) fn deploy_component_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployComponentDecl, MoxExtra<'src>> + Clone {
    deploy_keyword(deploy::COMPONENT)
        .ignore_then(name())
        .then(kw(Token::Other(':')).or_not())
        .then(name().or_not())
        .then(
            kw(Token::LBrace)
                .ignore_then(deploy_component_member().repeated().collect::<Vec<_>>())
                .then_ignore(kw(Token::RBrace))
                .map_with(|members, e| (members, e.span()))
                .or_not(),
        )
        .map_with(|(((name, colon), kind), body), e| (name, colon, kind, body, e.span()))
        .validate(|(name, colon, kind, body, span), _e, emitter| {
            if colon.is_none() {
                emitter.emit(Rich::custom(
                    name.span,
                    "expected `:` after the component name",
                ));
            }
            let kind = match kind {
                Some(kind) => kind,
                None => {
                    emitter.emit(Rich::custom(name.span, "expected a component kind"));
                    Name {
                        text: String::new(),
                        span,
                        escaped: false,
                    }
                }
            };
            let (members,) = match body {
                Some((members, _body_span)) => (members,),
                None => (Vec::new(),),
            };
            let mut component = DeployComponentDecl {
                name,
                kind,
                requires: Vec::new(),
                settings: Vec::new(),
                designs: Vec::new(),
                flows: Vec::new(),
                env: Vec::new(),
                span,
            };
            for member in members {
                match member {
                    DeployComponentMember::Requires(names) => component.requires.extend(names),
                    DeployComponentMember::Design(binding) => component.designs.push(binding),
                    DeployComponentMember::Flow(binding) => component.flows.push(binding),
                    DeployComponentMember::Env(entries) => component.env.extend(entries),
                    DeployComponentMember::Setting(setting) => component.settings.push(setting),
                }
            }
            component
        })
}

/// A `connects <from> -> <to>` entry of an application body. Committed once
/// `connects` matched: missing halves are reported errors and the entry
/// still recovers.
pub(crate) fn deploy_connects_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployConnectionDecl, MoxExtra<'src>> + Clone {
    deploy_keyword(deploy::CONNECTS)
        .ignore_then(name().or_not())
        .then(deploy_arrow().or_not())
        .then(name().or_not())
        .map_with(|((from, arrow), to), e| (from, arrow, to, e.span()))
        .validate(|(from, arrow, to, span), _e, emitter| {
            let missing = |span: Span| Name {
                text: String::new(),
                span,
                escaped: false,
            };
            let from = from.unwrap_or_else(|| missing(span));
            if from.text.is_empty() {
                emitter.emit(Rich::custom(
                    span,
                    "expected a component name after `connects`",
                ));
            }
            if arrow.is_none() {
                emitter.emit(Rich::custom(
                    span,
                    "expected `->` after the `connects` source",
                ));
            }
            let to = to.unwrap_or_else(|| missing(span));
            if to.text.is_empty() {
                emitter.emit(Rich::custom(span, "expected a component name after `->`"));
            }
            DeployConnectionDecl { from, to, span }
        })
}

/// One item of an application body.
#[derive(Debug, Clone)]
pub(crate) enum DeployApplicationItem {
    Component(DeployComponentDecl),
    Connects(DeployConnectionDecl),
}

/// An `application <name> { ... }` declaration.
pub(crate) fn deploy_application_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployApplicationDecl, MoxExtra<'src>> + Clone {
    let item = choice((
        deploy_component_decl().map(DeployApplicationItem::Component),
        deploy_connects_decl().map(DeployApplicationItem::Connects),
    ));
    deploy_keyword(deploy::APPLICATION)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(item.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, items), e| {
            let mut application = DeployApplicationDecl {
                name,
                components: Vec::new(),
                connections: Vec::new(),
                span: e.span(),
            };
            for item in items {
                match item {
                    DeployApplicationItem::Component(component) => {
                        application.components.push(component)
                    }
                    DeployApplicationItem::Connects(connection) => {
                        application.connections.push(connection)
                    }
                }
            }
            application
        })
}

/// A `target: <name>` line of a profile body. Committed once `target`
/// matched: a missing `:` or target name is a reported error.
pub(crate) fn deploy_target_line<'src>(
) -> impl Parser<'src, Tokens<'src>, Name, MoxExtra<'src>> + Clone {
    deploy_keyword(deploy::TARGET)
        .ignore_then(kw(Token::Other(':')).or_not())
        .then(name().or_not())
        .map_with(|(colon, name), e| (colon, name, e.span()))
        .validate(|(colon, name, span), _e, emitter| {
            if colon.is_none() {
                emitter.emit(Rich::custom(span, "expected `:` after `target`"));
            }
            match name {
                Some(name) => name,
                None => {
                    emitter.emit(Rich::custom(span, "expected a target name after `target:`"));
                    Name {
                        text: String::new(),
                        span,
                        escaped: false,
                    }
                }
            }
        })
}

/// A `require (...)`/`prohibit (...)` policy of a profile body. Committed
/// once the keyword matched: missing parens are a reported error. The
/// parenthesized condition is captured raw (the `when`-condition
/// convention); the driver slices strictly inside.
pub(crate) fn deploy_policy_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployPolicyDecl, MoxExtra<'src>> + Clone {
    deploy_keyword(deploy::REQUIRE)
        .map(|span| (false, span))
        .or(deploy_keyword(deploy::PROHIBIT).map(|span| (true, span)))
        .then(raw_parens().or_not())
        .map_with(|((prohibit, keyword), expr), e| (prohibit, keyword, expr, e.span()))
        .validate(|(prohibit, keyword, expr, span), _e, emitter| {
            if expr.is_none() {
                emitter.emit(Rich::custom(
                    keyword,
                    if prohibit {
                        "expected a parenthesized condition after `prohibit`"
                    } else {
                        "expected a parenthesized condition after `require`"
                    },
                ));
            }
            DeployPolicyDecl {
                prohibit,
                expr: expr.unwrap_or((keyword.end..keyword.end).into()),
                span,
            }
        })
}

/// Consumes a run of tokens that cannot start or continue a profile member,
/// stopping before the body's closing `}` and before identifiers (which
/// could begin the next member). The recovery alternative that keeps a
/// malformed member (e.g. a policy without parens) from failing the whole
/// profile. Always consumes at least one token.
pub(crate) fn junk_deploy_profile_member<'src>(
) -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone {
    let first = select! { t if !matches!(t, Token::RBrace) => () };
    let rest = select! {
        t if !matches!(t, Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)) => ()
    };
    first
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(
                e.span(),
                "expected a target, defaults, mappings, or policy member",
            ));
        })
}

/// A `<kind> -> <platform.resource>` entry of a `mappings` block. Committed
/// once the kind matched: a missing arrow or resource is a reported error.
pub(crate) fn deploy_mapping_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployMappingDecl, MoxExtra<'src>> + Clone {
    name()
        .then(deploy_arrow().or_not())
        .then(qname().or_not())
        .map_with(|((kind, arrow), resource), e| (kind, arrow, resource, e.span()))
        .validate(|(kind, arrow, resource, span), _e, emitter| {
            if arrow.is_none() {
                emitter.emit(Rich::custom(
                    kind.span,
                    "expected `->` after the mapping kind",
                ));
            }
            let resource = match resource {
                Some(resource) => resource,
                None => {
                    emitter.emit(Rich::custom(span, "expected a resource after `->`"));
                    QualifiedName {
                        segments: Vec::new(),
                        span: (span.end..span.end).into(),
                    }
                }
            };
            DeployMappingDecl {
                kind,
                resource,
                span,
            }
        })
}

/// One member of a profile body.
#[derive(Debug, Clone)]
pub(crate) enum DeployProfileMember {
    Target(Name),
    Defaults(Vec<DeploySettingDecl>),
    Mappings(Vec<DeployMappingDecl>),
    Policy(DeployPolicyDecl),
    Junk,
}

/// A `profile <name> { ... }` declaration.
pub(crate) fn deploy_profile_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployProfileDecl, MoxExtra<'src>> + Clone {
    let defaults = deploy_keyword(deploy::DEFAULTS)
        .ignore_then(
            kw(Token::LBrace)
                .ignore_then(deploy_setting().repeated().collect::<Vec<_>>())
                .then_ignore(kw(Token::RBrace)),
        )
        .map(DeployProfileMember::Defaults);
    let mappings = deploy_keyword(deploy::MAPPINGS)
        .ignore_then(
            kw(Token::LBrace)
                .ignore_then(deploy_mapping_decl().repeated().collect::<Vec<_>>())
                .then_ignore(kw(Token::RBrace)),
        )
        .map(DeployProfileMember::Mappings);
    let member = choice((
        deploy_target_line().map(DeployProfileMember::Target),
        defaults,
        mappings,
        deploy_policy_decl().map(DeployProfileMember::Policy),
    ))
    .or(junk_deploy_profile_member().to(DeployProfileMember::Junk));
    deploy_keyword(deploy::PROFILE)
        .ignore_then(name())
        .then_ignore(kw(Token::LBrace))
        .then(member.repeated().collect::<Vec<_>>())
        .then_ignore(kw(Token::RBrace))
        .map_with(|(name, members), e| (name, members, e.span()))
        .validate(|(name, members, span), _e, _emitter| {
            let mut profile = DeployProfileDecl {
                name,
                target: None,
                defaults: Vec::new(),
                policies: Vec::new(),
                mappings: Vec::new(),
                span,
            };
            for member in members {
                match member {
                    DeployProfileMember::Target(target) => profile.target = Some(target),
                    DeployProfileMember::Defaults(defaults) => profile.defaults = defaults,
                    DeployProfileMember::Mappings(mappings) => profile.mappings = mappings,
                    DeployProfileMember::Policy(policy) => profile.policies.push(policy),
                    DeployProfileMember::Junk => {}
                }
            }
            profile
        })
}

/// A `deployment <name> for <application> { use <profile> ... }`
/// declaration. Committed once the `for` clause matched: a missing
/// application name is a reported error.
pub(crate) fn deploy_deployment_decl<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployDeploymentDecl, MoxExtra<'src>> + Clone {
    let use_line = deploy_keyword(deploy::USE)
        .ignore_then(name().or_not())
        .map_with(|name, e| (name, e.span()))
        .validate(|(name, span), _e, emitter| match name {
            Some(name) => Some(name),
            None => {
                emitter.emit(Rich::custom(span, "expected a profile name after `use`"));
                None
            }
        });
    let configure = deploy_keyword(deploy::CONFIGURE).ignore_then(
        kw(Token::LBrace)
            .ignore_then(deploy_setting().repeated().collect::<Vec<_>>())
            .then_ignore(kw(Token::RBrace)),
    );
    deploy_keyword(deploy::DEPLOYMENT)
        .ignore_then(name())
        .then(deploy_keyword(deploy::FOR).or_not())
        .then(name().or_not())
        .then_ignore(kw(Token::LBrace))
        .then(use_line.or_not())
        .then(configure.or_not())
        .then_ignore(kw(Token::RBrace))
        .map_with(
            |((((name, for_kw), application), use_line), configure), e| {
                (name, for_kw, application, use_line, configure, e.span())
            },
        )
        .validate(
            |(name, for_kw, application, use_line, configure, span), _e, emitter| {
                if for_kw.is_none() {
                    emitter.emit(Rich::custom(
                        name.span,
                        "expected `for` after the deployment name",
                    ));
                }
                let application = match application {
                    Some(application) => application,
                    None => {
                        emitter.emit(Rich::custom(
                            name.span,
                            "expected an application after `for`",
                        ));
                        Name {
                            text: String::new(),
                            span,
                            escaped: false,
                        }
                    }
                };
                if use_line.is_none() {
                    emitter.emit(Rich::custom(span, "expected a `use <profile>` line"));
                }
                DeployDeploymentDecl {
                    name,
                    application,
                    profile: use_line.flatten(),
                    configure: configure.unwrap_or_default(),
                    span,
                }
            },
        )
}

/// Consumes a run of tokens up to the next `import` keyword or the
/// contextual `application`/`profile`/`deployment` words (or end of input).
/// Declaration-level recovery for `.deploy` files: always consumes at least
/// one token (guaranteeing progress) and emits an error for the skipped
/// region.
pub(crate) fn junk_deploy_file<'src>() -> impl Parser<'src, Tokens<'src>, (), MoxExtra<'src>> + Clone
{
    let rest = select! {
        t if !matches!(
            t,
            Token::Import
                | Token::Ident(deploy::APPLICATION)
                | Token::Ident(deploy::PROFILE)
                | Token::Ident(deploy::DEPLOYMENT)
        ) =>
        ()
    };
    any()
        .ignore_then(rest.repeated().ignored())
        .validate(|(), e, emitter| {
            emitter.emit(Rich::custom(
                e.span(),
                "expected an import, application, profile, or deployment declaration",
            ));
        })
}

/// One top-level item of a `.deploy` file.
#[derive(Clone)]
pub(crate) enum DeployFileItem {
    Import(ImportDecl),
    Application(DeployApplicationDecl),
    Profile(DeployProfileDecl),
    Deployment(DeployDeploymentDecl),
    Junk,
}

pub(crate) fn fold_deploy_file(items: Vec<DeployFileItem>) -> (DeployFile, Option<Span>) {
    let mut imports = Vec::new();
    let mut applications = Vec::new();
    let mut profiles = Vec::new();
    let mut deployments = Vec::new();
    // Span of the first `import` that follows a declaration (a syntax error
    // reported by the caller's `validate`).
    let mut late_import = None;
    for item in items {
        match item {
            DeployFileItem::Import(decl) => {
                if applications.is_empty() && profiles.is_empty() && deployments.is_empty() {
                    imports.push(decl);
                } else if late_import.is_none() {
                    late_import = Some(decl.span);
                }
            }
            DeployFileItem::Application(decl) => applications.push(decl),
            DeployFileItem::Profile(decl) => profiles.push(decl),
            DeployFileItem::Deployment(decl) => deployments.push(decl),
            DeployFileItem::Junk => {}
        }
    }
    (
        DeployFile {
            imports,
            applications,
            profiles,
            deployments,
        },
        late_import,
    )
}

pub(crate) fn deploy_file<'src>(
) -> impl Parser<'src, Tokens<'src>, DeployFile, MoxExtra<'src>> + Clone {
    let item = choice((
        import_decl().map(DeployFileItem::Import),
        deploy_application_decl().map(DeployFileItem::Application),
        deploy_profile_decl().map(DeployFileItem::Profile),
        deploy_deployment_decl().map(DeployFileItem::Deployment),
    ))
    .or(junk_deploy_file().to(DeployFileItem::Junk));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map(fold_deploy_file)
        .validate(|(file, late_import), _, emitter| {
            if let Some(span) = late_import {
                emitter.emit(Rich::custom(
                    span,
                    "`import` after an application, profile, or deployment declaration",
                ));
            }
            file
        })
}

// --- .uc use case files -------------------------------------------------------
