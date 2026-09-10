//! Naming utilities for Rust code generation: case conversion, keyword
//! escaping, Rust-side identifier mapping, and naive English pluralization.

/// Rust keywords and primitive names that cannot be used as bare identifiers.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "box", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod",
    "move", "mut", "pub", "ref", "return", "static", "struct", "trait", "true", "type", "unsafe",
    "use", "where", "while", "yield", "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64",
    "str", "u8", "u16", "u32", "u64", "Self", "self", "super", "crate",
];

/// Returns `name` as a valid Rust identifier, raw-escaping keywords as `r#kw`.
pub fn rust_ident(name: &str) -> String {
    if RUST_KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

/// Converts a `.mox` identifier (usually camelCase) to snake_case.
///
/// `getBook` -> `get_book`, `URLCount` -> `url_count`, `books` -> `books`.
pub fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev_lower_or_digit = index
                .checked_sub(1)
                .map(|p| chars[p].is_ascii_lowercase() || chars[p].is_ascii_digit())
                .unwrap_or(false);
            let next_lower = chars
                .get(index + 1)
                .map(|n| n.is_ascii_lowercase())
                .unwrap_or(false);
            let starts_run_after_word = prev_lower_or_digit;
            let starts_short_run = index > 0 && next_lower;
            if starts_run_after_word || starts_short_run {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Naive English pluralization for slotmap field names (`Book` -> `books`,
/// `Box` -> `boxes`, `Category` -> `categories`).
pub fn pluralize(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    if lower.ends_with('s')
        || lower.ends_with('x')
        || lower.ends_with('z')
        || lower.ends_with("ch")
        || lower.ends_with("sh")
    {
        format!("{word}es")
    } else if lower.ends_with('y')
        && !lower.ends_with("ay")
        && !lower.ends_with("ey")
        && !lower.ends_with("oy")
    {
        format!("{}ies", &word[..word.len() - 1])
    } else {
        format!("{word}s")
    }
}

/// The slotmap field name for a class: pluralized snake_case, e.g. `books`.
pub fn slotmap_field(class_name: &str) -> String {
    let snake = snake_case(class_name);
    let (head, last) = snake.split_at(snake.len() - 1);
    let plural = pluralize(last);
    format!("{head}{plural}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_conversion() {
        assert_eq!(snake_case("books"), "books");
        assert_eq!(snake_case("getBook"), "get_book");
        assert_eq!(snake_case("title"), "title");
        assert_eq!(snake_case("URLCount"), "url_count");
        assert_eq!(snake_case("HTTPServer"), "http_server");
        assert_eq!(snake_case("citation"), "citation");
    }

    #[test]
    fn keyword_escaping() {
        assert_eq!(rust_ident("type"), "r#type");
        assert_eq!(rust_ident("title"), "title");
        assert_eq!(rust_ident("fn"), "r#fn");
    }

    #[test]
    fn pluralization() {
        assert_eq!(pluralize("book"), "books");
        assert_eq!(pluralize("writer"), "writers");
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("category"), "categories");
        assert_eq!(pluralize("library"), "libraries");
        assert_eq!(slotmap_field("Book"), "books");
        assert_eq!(slotmap_field("Library"), "libraries");
        assert_eq!(slotmap_field("Writer"), "writers");
    }
}
