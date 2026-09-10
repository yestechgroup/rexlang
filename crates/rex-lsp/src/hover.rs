//! Hover rendering: turns a navigation-index [`Definition`] into markdown.

use rex_driver::navigation::{DefId, FeatureSymbolKind, NavigationIndex, SymbolKind};

/// Renders the hover markdown for the definition `def_id` of `index`.
/// Shapes (see the tests for the exact contract):
///
/// * `**class Book**` / `**class Shelf**` + `\nextends Base, Other`
/// * `**interface Named**`, `**enum Mood**`, `**datatype Date**`,
///   `**vocabulary Currency**`
/// * features: `**books**: Book[]` plus a kind line, e.g.
///   `contains — opposite: `library`` / `refers — opposite: `books`` /
///   `container` / `attribute` / `op (abstract)` / `derived`
/// * enum literals: `**Mystery** = 0 (BookCategory)`
pub fn hover_markdown(index: &NavigationIndex, def_id: DefId) -> String {
    let definition = index.definition(def_id);
    match definition.kind {
        SymbolKind::Class => match &definition.extends_text {
            Some(extends) => format!("**class {}**\nextends {extends}", definition.name),
            None => format!("**class {}**", definition.name),
        },
        SymbolKind::Interface => format!("**interface {}**", definition.name),
        SymbolKind::Enum => format!("**enum {}**", definition.name),
        SymbolKind::Datatype => format!("**datatype {}**", definition.name),
        SymbolKind::Vocabulary => format!("**vocabulary {}**", definition.name),
        SymbolKind::EnumLiteral => {
            let value = definition.value_text.as_deref();
            let owner = definition
                .owner
                .map(|owner| index.definition(owner).name.as_str());
            match (value, owner) {
                (Some(value), Some(owner)) => {
                    format!("**{}** = {value} ({owner})", definition.name)
                }
                (_, Some(owner)) => format!("**{}** ({owner})", definition.name),
                _ => format!("**{}**", definition.name),
            }
        }
        SymbolKind::Feature(feature_kind) => {
            let mut out = format!("**{}**", definition.name);
            if let Some(type_text) = &definition.type_text {
                out.push_str(": ");
                out.push_str(type_text);
                if let Some(multiplicity_text) = &definition.multiplicity_text {
                    out.push_str(multiplicity_text);
                }
            }
            out.push('\n');
            out.push_str(match feature_kind {
                FeatureSymbolKind::Attribute => "attribute",
                FeatureSymbolKind::Operation => "op (abstract)",
                FeatureSymbolKind::Derived => "derived",
                FeatureSymbolKind::Containment => "contains",
                FeatureSymbolKind::CrossReference => "refers",
                FeatureSymbolKind::Container => "container",
            });
            if let Some(opposite) = &definition.opposite_text {
                match feature_kind {
                    FeatureSymbolKind::Containment
                    | FeatureSymbolKind::CrossReference
                    | FeatureSymbolKind::Container => {
                        out.push_str(" — opposite: `");
                        out.push_str(opposite);
                        out.push('`');
                    }
                    _ => {}
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_id_of(index: &NavigationIndex, name: &str) -> DefId {
        index
            .definitions()
            .find(|(_, definition)| definition.name == name)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no definition named '{name}'"))
    }

    fn hover_of(source: &str, name: &str) -> String {
        let index = NavigationIndex::build_or_empty(source);
        hover_markdown(&index, def_id_of(&index, name))
    }

    #[test]
    fn classes_render_with_optional_extends_clause() {
        assert_eq!(
            hover_of("package demo\n\nclass Book {}", "Book"),
            "**class Book**"
        );
        assert_eq!(
            hover_of(
                "package demo\n\nclass Base {}\nclass Shelf extends Base, Other {}",
                "Shelf"
            ),
            "**class Shelf**\nextends Base, Other"
        );
    }

    #[test]
    fn other_top_level_kinds_render_with_their_keyword() {
        assert_eq!(
            hover_of("package demo\n\ninterface Named {}", "Named"),
            "**interface Named**"
        );
        assert_eq!(
            hover_of("package demo\n\nenum Mood { Happy = 0 }", "Mood"),
            "**enum Mood**"
        );
        assert_eq!(
            hover_of("package demo\n\ntype Date wraps opaque", "Date"),
            "**datatype Date**"
        );
        assert_eq!(
            hover_of(
                "package demo\n\nvocabulary Currency from \"iso:4217\" { key code }",
                "Currency"
            ),
            "**vocabulary Currency**"
        );
    }

    #[test]
    fn features_render_type_multiplicity_and_kind_line() {
        let source = "package demo\n\n\
            class Library {\n\
            \x20   String name\n\
            \x20   int[3] stars\n\
            \x20   contains Book[] books opposite library\n\
            \x20   refers Writer[] authors opposite books\n\
            \x20   container Room room opposite occupants\n\
            \x20   op Book find(String title)\n\
            \x20   derived String summary\n\
            }\n\n\
            class Book {}\nclass Writer {}\nclass Room {}\n";
        assert_eq!(
            hover_of(source, "name"),
            "**name**: String\nattribute"
        );
        assert_eq!(hover_of(source, "stars"), "**stars**: int[3]\nattribute");
        assert_eq!(
            hover_of(source, "books"),
            "**books**: Book[]\ncontains — opposite: `library`"
        );
        assert_eq!(
            hover_of(source, "authors"),
            "**authors**: Writer[]\nrefers — opposite: `books`"
        );
        assert_eq!(
            hover_of(source, "room"),
            "**room**: Room\ncontainer — opposite: `occupants`"
        );
        assert_eq!(hover_of(source, "find"), "**find**: Book\nop (abstract)");
        assert_eq!(
            hover_of(source, "summary"),
            "**summary**: String\nderived"
        );
    }

    #[test]
    fn containment_and_reference_without_opposite_show_a_plain_kind_line() {
        let source = "package demo\n\n\
            class Library {\n\
            \x20   contains Book[] books\n\
            \x20   refers Writer[] authors\n\
            }\n\n\
            class Book {}\nclass Writer {}\n";
        assert_eq!(hover_of(source, "books"), "**books**: Book[]\ncontains");
        assert_eq!(hover_of(source, "authors"), "**authors**: Writer[]\nrefers");
    }

    #[test]
    fn enum_literals_render_value_and_owner() {
        assert_eq!(
            hover_of(
                "package demo\n\nenum BookCategory { Mystery as \"M\" = 0 }",
                "Mystery"
            ),
            "**Mystery** = 0 (BookCategory)"
        );
        // A literal without an `=` clause has no value part.
        assert_eq!(
            hover_of("package demo\n\nenum Mood { Happy }", "Happy"),
            "**Happy** (Mood)"
        );
    }
}
