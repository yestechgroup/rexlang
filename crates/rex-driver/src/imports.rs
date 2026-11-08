//! The import-content provider types: [`SchemaImports`] and [`SigilImports`]
//! — twin `(mox path, import path)` providers — bundled in
//! [`DomainImports`], which every compile entry point takes.

/// The JSON content of the `import schema` declarations of a compilation.
///
/// The driver is filesystem-free: a host that reads `.mox` files from disk
/// (the CLI) or embeds them (tests, language servers) provides each
/// import's JSON text through this type. Keys are the **(mox file path,
/// import path)** pair, both exactly as the host names them — the mox path
/// must match the path the file is compiled under, and the import path must
/// match the string written in the `import schema "<path>"` declaration.
/// Content provided for another mox file does not satisfy an import; a
/// declared import without provided content is the error diagnostic
/// ``imported schema '<path>' was not provided``.
///
/// Re-providing a `(mox, import)` pair replaces the earlier entry (last
/// wins).
///
/// ```
/// use rex_driver::{DomainImports, SchemaImports, SigilImports, compile_files};
///
/// let imports = DomainImports {
///     schemas: SchemaImports::new()
///         .provide("demo.mox", "schemas/todo_item.json", r#"{"title": "Todo item"}"#),
///     sigil: SigilImports::new(),
/// };
/// let compilation = compile_files(
///     &[("demo.mox".to_string(),
///        "package demo\n\nimport schema \"schemas/todo_item.json\" as TodoItem\n\nclass C { refers TodoItem t }\n".to_string())],
///     &imports,
/// );
/// assert!(compilation.diagnostics.is_empty());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaImports {
    entries: Vec<(String, String, String)>,
}

impl SchemaImports {
    /// Creates an empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Provides the JSON text for one `(mox path, import path)` pair.
    pub fn provide(
        mut self,
        mox_path: impl Into<String>,
        import_path: impl Into<String>,
        json: impl Into<String>,
    ) -> Self {
        self.insert(mox_path, import_path, json);
        self
    }

    /// Inserts one `(mox path, import path)` pair, replacing any earlier
    /// entry for the same pair.
    pub fn insert(
        &mut self,
        mox_path: impl Into<String>,
        import_path: impl Into<String>,
        json: impl Into<String>,
    ) {
        let mox_path = mox_path.into();
        let import_path = import_path.into();
        let json = json.into();
        match self
            .entries
            .iter_mut()
            .find(|(mox, import, _)| *mox == mox_path && *import == import_path)
        {
            Some(entry) => entry.2 = json,
            None => self.entries.push((mox_path, import_path, json)),
        }
    }

    /// The provided JSON text for the `(mox path, import path)` pair, if any.
    pub fn get(&self, mox_path: &str, import_path: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(mox, import, _)| mox == mox_path && import == import_path)
            .map(|(_, _, json)| json.as_str())
    }

    /// `true` when no content is provided at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The rosetta source texts of the `import sigil` declarations of a
/// compilation.
///
/// Mirrors [`SchemaImports`]: the driver is filesystem-free, so a host that
/// reads `.mox` files from disk (the CLI) or embeds them (tests, language
/// servers) provides each import's rosetta text through this type. Keys are
/// the **(mox file path, import path)** pair, both exactly as the host names
/// them — the mox path must match the path the file is compiled under, and
/// the import path must match the string written in the
/// `import sigil "<path>"` declaration. Content provided for another mox
/// file does not satisfy an import; a declared import without provided
/// content is the error diagnostic
/// ``imported sigil '<path>' was not provided``.
///
/// Additional entries for the same mox path (a different import path each)
/// are **candidate files**: the CLI provides every `*.rosetta` file next to
/// the named import so rosetta-internal namespace imports resolve
/// transitively, and the driver lowers only the closure the named file
/// actually needs (see the crate-internal `sigil` lowering module for the
/// selection rule). Candidate files that end up unselected are never
/// diagnosed.
///
/// Re-providing a `(mox, import)` pair replaces the earlier entry (last
/// wins).
///
/// ```
/// use rex_driver::{DomainImports, SigilImports, compile_str};
///
/// let sigil = SigilImports::new().provide(
///     "demo.mox",
///     "oracle/trade.rosetta",
///     "namespace oracle.basic\n\ntype Trade:\n    id string (1..1)\n",
/// );
/// let compilation = compile_str(
///     "demo.mox",
///     "package demo\n\nimport sigil \"oracle/trade.rosetta\"\n\nclass Book { refers oracle.basic.Trade about }\n",
///     &DomainImports {
///         schemas: rex_driver::SchemaImports::new(),
///         sigil,
///     },
/// );
/// assert!(compilation.diagnostics.is_empty());
/// let model = compilation.model.unwrap();
/// assert_eq!(model.packages.len(), 2);
/// assert_eq!(model.packages[1].name, "oracle.basic");
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SigilImports {
    entries: Vec<(String, String, String)>,
}

impl SigilImports {
    /// Creates an empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Provides the rosetta text for one `(mox path, import path)` pair.
    pub fn provide(
        mut self,
        mox_path: impl Into<String>,
        import_path: impl Into<String>,
        rosetta: impl Into<String>,
    ) -> Self {
        self.insert(mox_path, import_path, rosetta);
        self
    }

    /// Inserts one `(mox path, import path)` pair, replacing any earlier
    /// entry for the same pair.
    pub fn insert(
        &mut self,
        mox_path: impl Into<String>,
        import_path: impl Into<String>,
        rosetta: impl Into<String>,
    ) {
        let mox_path = mox_path.into();
        let import_path = import_path.into();
        let rosetta = rosetta.into();
        match self
            .entries
            .iter_mut()
            .find(|(mox, import, _)| *mox == mox_path && *import == import_path)
        {
            Some(entry) => entry.2 = rosetta,
            None => self.entries.push((mox_path, import_path, rosetta)),
        }
    }

    /// The provided rosetta text for the `(mox path, import path)` pair, if
    /// any.
    pub fn get(&self, mox_path: &str, import_path: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(mox, import, _)| mox == mox_path && import == import_path)
            .map(|(_, _, rosetta)| rosetta.as_str())
    }

    /// Every provided entry as `(mox path, import path, rosetta text)`, in
    /// insertion order.
    pub fn entries(&self) -> &[(String, String, String)] {
        &self.entries
    }

    /// `true` when no content is provided at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
/// The import content provided to a compilation: the JSON of `import
/// schema` declarations and the rosetta of `import sigil` declarations,
/// each keyed by (declaring file path, import path as written). The driver
/// stays filesystem-free — hosts read the files and thread the bundle
/// through. Every public compile entry point takes one.
///
/// The empty bundle ([`DomainImports::default`]) is the no-import
/// compilation: a declaration without a provider entry is the
/// ``imported ... '<path>' was not provided`` error.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainImports {
    /// The `import schema` content of the compilation's files.
    pub schemas: SchemaImports,
    /// The `import sigil` content of the compilation's files.
    pub sigil: SigilImports,
}
