/// AST parser module — tree-sitter backends for multiple languages
///
/// Extract symbols (functions, classes, variables, imports, exports) and references (calls, imports, inheritance)
/// from source code. Supports Python, JavaScript, Rust, TypeScript, and Go.


// ---------------------------------------------------------------------------
// Shared types (language-agnostic)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Symbol kind
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Function,
    Class,
    Variable,
    Method,
    Import,
    Export,
    Decorator,
    Module,
    Enum,
    TypeAlias,
    Section,
    CodeBlock,
    Link,
    Table,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Class => "class",
            SymbolKind::Variable => "variable",
            SymbolKind::Method => "method",
            SymbolKind::Import => "import",
            SymbolKind::Export => "export",
            SymbolKind::Decorator => "decorator",
            SymbolKind::Module => "module",
            SymbolKind::Enum => "enum",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Section => "section",
            SymbolKind::CodeBlock => "code_block",
            SymbolKind::Link => "link",
            SymbolKind::Table => "table",
        }
    }
}

/// Extracted symbol
#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub parent: Option<String>,
}

/// Extracted reference
#[derive(Debug, Clone)]
pub struct ParsedReference {
    pub caller_symbol: Option<String>,
    pub callee_symbol: String,
    pub ref_kind: String,
    pub line: usize,
}

impl ParsedReference {
    /// Compute confidence score for this reference.
    ///
    /// Higher confidence means the reference is more likely to be accurate.
    /// - `call`, `import`, `inherit`: 1.0 (statically verifiable)
    /// - `dynamic_dispatch`: 0.7 (method calls on interfaces/traits)
    /// - `string_ref`: 0.3 (string-based references)
    /// - Unknown kinds default to 0.5
    pub fn confidence(&self) -> f64 {
        match self.ref_kind.as_str() {
            "call" | "import" | "inherit" | "export" | "use" | "require" | "include" => 1.0,
            "dynamic_dispatch" | "virtual_call" => 0.7,
            "string_ref" => 0.3,
            _ => 0.5,
        }
    }

    /// Whether this reference is dynamic (runtime-resolved).
    pub fn is_dynamic(&self) -> bool {
        matches!(self.ref_kind.as_str(), "dynamic_dispatch" | "virtual_call" | "string_ref")
    }
}

/// File parse result
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub symbols: Vec<ParsedSymbol>,
    pub references: Vec<ParsedReference>,
    pub imports: Vec<(String, String, String)>,
}

// ---------------------------------------------------------------------------
// Parser trait
// ---------------------------------------------------------------------------

/// Language-agnostic source code parser
///
/// Aligns with masterplan §8 (Parser Abstraction Layer / LanguageBackend).
/// The `parse()` method corresponds to `parse_symbols()` + `extract_refs()`.
/// The `slice_symbol()` and `estimate_tokens()` methods correspond to the
/// masterplan's `LanguageBackend::slice_symbol()` and `estimate_tokens()`.
pub trait SourceCodeParser {
    /// Language identifier (e.g. "python", "javascript")
    fn language(&self) -> &str;

    /// File extensions this parser handles (e.g. ["py"])
    fn file_extensions(&self) -> &[&str];

    /// Parse source code and extract symbols, references, imports
    fn parse(&mut self, source: &str) -> Result<ParseResult, anyhow::Error>;

    /// Slice a specific symbol for semantic compression.
    ///
    /// Default implementation delegates to `crate::compression::compress_symbol()`.
    /// Language-specific parsers can override for AST-aware slicing.
    ///
    /// Aligns with masterplan §8.1 `LanguageBackend::slice_symbol()`.
    fn slice_symbol(
        &self,
        source: &str,
        symbol: &ParsedSymbol,
        level: crate::compression::CompressionMode,
    ) -> crate::compression::SymbolSlice {
        crate::compression::compress_symbol(source, symbol.start_line, symbol.end_line, level)
    }

    /// Estimate token count for a code snippet.
    ///
    /// Default implementation delegates to `crate::token_estimation::estimate_token_count()`.
    /// Language-specific parsers can override for language-aware estimation.
    ///
    /// Aligns with masterplan §8.1 `LanguageBackend::estimate_tokens()`.
    fn estimate_tokens(&self, snippet: &str) -> usize {
        crate::token_estimation::estimate_token_count(snippet)
    }

    /// Detect if a reference represents a dynamic (runtime-resolved) reference.
    ///
    /// Default implementation checks common dynamic reference kinds:
    /// `dynamic_dispatch`, `virtual_call`, `string_ref`.
    /// Language-specific parsers can override for AST-aware detection.
    ///
    /// Aligns with masterplan §8.1 `LanguageBackend::is_dynamic_ref()`.
    fn is_dynamic_ref(&self, ref_kind: &str) -> bool {
        matches!(
            ref_kind,
            "dynamic_dispatch" | "virtual_call" | "string_ref"
        )
    }
}
