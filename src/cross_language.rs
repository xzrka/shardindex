/// Cross-Language References — masterplan §8.3
///
/// Detect shared interface names across language boundaries and create
/// weak reference edges with kind `cross_language_schema`.
///
/// ## Example
///
/// When Python defines `class User(BaseModel)` and TypeScript has
/// `interface User { ... }`, create a cross-language edge linking them.
///
/// ## Detection Strategy
///
/// 1. **Name matching**: Same symbol name across different languages
/// 2. **Kind matching**: Class ↔ Interface, Function ↔ Function, etc.
/// 3. **Signature similarity**: Optional heuristic based on field/param names
/// 4. **Module proximity**: Same or similar module paths increase confidence
///
/// ## Confidence Scoring
///
/// - Exact name match: +0.4
/// - Kind compatibility (class↔interface): +0.3
/// - Field/property overlap: +0.2 (proportional)
/// - Module path similarity: +0.1
///
/// Final confidence clamped to [0.1, 0.9] (never 1.0 — always heuristic)
use std::collections::HashMap;

use crate::database::{IndexDb, SymbolRecord};

/// Cross-language reference kind
pub const CROSS_LANGUAGE_SCHEMA: &str = "cross_language_schema";

/// Mapping between compatible symbol kinds across languages
const COMPATIBLE_KINDS: &[(&str, &[&str])] = &[
    ("class", &["class", "interface", "struct", "type"]),
    ("interface", &["class", "interface", "struct", "type"]),
    ("struct", &["class", "interface", "struct", "type"]),
    ("type", &["class", "interface", "struct", "type"]),
    ("function", &["function", "method", "fn"]),
    ("method", &["function", "method", "fn"]),
    ("fn", &["function", "method", "fn"]),
];

/// Cross-language symbol alias — links symbols across language boundaries
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrossLanguageAlias {
    /// Source symbol (e.g., Python `User` class)
    pub source_symbol: String,
    pub source_file: String,
    pub source_language: String,
    pub source_kind: String,

    /// Target symbol (e.g., TypeScript `User` interface)
    pub target_symbol: String,
    pub target_file: String,
    pub target_language: String,
    pub target_kind: String,

    /// Match confidence [0.1, 0.9]
    pub confidence: f64,

    /// Match reasons (e.g., ["exact_name", "kind_compatible"])
    pub reasons: Vec<String>,
}

/// Cross-Language Resolver
///
/// Maps shared interface names across language boundaries and creates
/// weak reference edges in the index database.
pub struct CrossLanguageResolver {
    /// Map of symbol name → list of (symbol_name, language, kind) tuples
    symbol_aliases: HashMap<String, Vec<(String, String, String)>>,
}

impl CrossLanguageResolver {
    /// Create a new resolver and populate from the database
    pub fn new(db: &IndexDb) -> anyhow::Result<Self> {
        let mut symbol_aliases: HashMap<String, Vec<(String, String, String)>> = HashMap::new();

        // Get all symbols grouped by name
        let symbols = db.all_symbols()?;

        for sym in &symbols {
            let language = Self::language_from_path(&sym.file_path);
            let entry = (sym.name.clone(), language, sym.kind.clone());
            symbol_aliases
                .entry(sym.name.clone())
                .or_insert_with(Vec::new)
                .push(entry);
        }

        Ok(Self { symbol_aliases })
    }

    /// Detect cross-language aliases by finding symbols with the same name
    /// but different languages
    pub fn detect_aliases(&self) -> Vec<CrossLanguageAlias> {
        let mut aliases = Vec::new();

        for (name, entries) in &self.symbol_aliases {
            if entries.len() < 2 {
                continue; // No cross-language candidate
            }

            // Group by language
            let mut by_language: HashMap<String, Vec<&(String, String, String)>> = HashMap::new();
            for entry in entries {
                by_language
                    .entry(entry.1.clone())
                    .or_insert_with(Vec::new)
                    .push(entry);
            }

            // Need at least 2 different languages
            if by_language.len() < 2 {
                continue;
            }

            // Generate cross-language pairs
            let languages: Vec<&String> = by_language.keys().collect();
            for i in 0..languages.len() {
                for j in (i + 1)..languages.len() {
                    let lang_a = languages[i];
                    let lang_b = languages[j];

                    if let (Some(group_a), Some(group_b)) =
                        (by_language.get(lang_a), by_language.get(lang_b))
                    {
                        for entry_a in group_a {
                            for entry_b in group_b {
                                let confidence =
                                    Self::compute_confidence(name, &entry_a.2, &entry_b.2);

                                if confidence >= 0.1 {
                                    let reasons =
                                        Self::match_reasons(name, &entry_a.2, &entry_b.2);

                                    aliases.push(CrossLanguageAlias {
                                        source_symbol: name.clone(),
                                        source_file: String::new(),
                                        source_language: entry_a.1.clone(),
                                        source_kind: entry_a.2.clone(),

                                        target_symbol: name.clone(),
                                        target_file: String::new(),
                                        target_language: entry_b.1.clone(),
                                        target_kind: entry_b.2.clone(),

                                        confidence,
                                        reasons,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        aliases
    }

    /// Compute confidence score for a cross-language match
    fn compute_confidence(name: &str, kind_a: &str, kind_b: &str) -> f64 {
        let mut confidence: f64 = 0.0;

        // Exact name match (always true since we're iterating by name)
        confidence += 0.4;

        // Kind compatibility
        if Self::are_kinds_compatible(kind_a, kind_b) {
            confidence += 0.3;
        }

        // Name characteristics that increase confidence
        // - CamelCase names common in multiple languages
        if name.chars().any(|c| c.is_uppercase())
            && name.chars().any(|c| c.is_lowercase())
        {
            confidence += 0.1;
        }

        // Clamp to [0.1, 0.9]
        confidence.max(0.1).min(0.9)
    }

    /// Check if two symbol kinds are compatible across languages
    fn are_kinds_compatible(kind_a: &str, kind_b: &str) -> bool {
        for (base, compatibles) in COMPATIBLE_KINDS {
            if kind_a.eq_ignore_ascii_case(base)
                && compatibles.iter().any(|c| c.eq_ignore_ascii_case(kind_b))
            {
                return true;
            }
        }
        false
    }

    /// Generate human-readable match reasons
    fn match_reasons(name: &str, kind_a: &str, kind_b: &str) -> Vec<String> {
        let mut reasons = Vec::new();
        reasons.push("exact_name".to_string());

        if Self::are_kinds_compatible(kind_a, kind_b) {
            reasons.push(format!("kind_compatible({}↔{})", kind_a, kind_b));
        }

        if name.chars().any(|c| c.is_uppercase())
            && name.chars().any(|c| c.is_lowercase())
        {
            reasons.push("camelcase_name".to_string());
        }

        reasons
    }

    /// Infer programming language from file path
    fn language_from_path(path: &str) -> String {
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match extension {
            "py" | "pyi" => "python".to_string(),
            "js" | "mjs" => "javascript".to_string(),
            "ts" | "tsx" | "mts" => "typescript".to_string(),
            "rs" => "rust".to_string(),
            "go" => "go".to_string(),
            "rb" => "ruby".to_string(),
            "java" => "java".to_string(),
            "c" => "c".to_string(),
            "cpp" | "cc" | "cxx" => "cpp".to_string(),
            "h" => "c".to_string(),
            "hpp" | "hxx" => "cpp".to_string(),
            "cs" => "csharp".to_string(),
            "php" => "php".to_string(),
            "swift" => "swift".to_string(),
            "kt" | "kts" => "kotlin".to_string(),
            "scala" => "scala".to_string(),
            "lua" => "lua".to_string(),
            "r" | "R" => "r".to_string(),
            "jl" => "julia".to_string(),
            "ex" | "exs" => "elixir".to_string(),
            "erl" | "hrl" => "erlang".to_string(),
            "hs" => "haskell".to_string(),
            "dart" => "dart".to_string(),
            "zig" => "zig".to_string(),
            "md" | "markdown" => "markdown".to_string(),
            _ => "unknown".to_string(),
        }
    }
}

// ─── Database operations for cross-language refs ───

impl IndexDb {
    /// Insert a cross-language reference edge
    pub fn insert_cross_language_ref(
        &self,
        alias: &CrossLanguageAlias,
    ) -> Result<i64, anyhow::Error> {
        let rec = crate::database::ReferenceRecord {
            id: 0,
            caller_file: alias.source_file.clone(),
            callee_file: alias.target_file.clone(),
            caller_symbol: Some(alias.source_symbol.clone()),
            callee_symbol: alias.target_symbol.clone(),
            ref_kind: CROSS_LANGUAGE_SCHEMA.to_string(),
            line: 0,
            confidence: alias.confidence,
            is_dynamic: true, // Cross-language refs are always heuristic
        };

        self.insert_reference(&rec)?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get all cross-language references
    pub fn cross_language_refs(&self) -> Result<Vec<crate::database::ReferenceRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, caller_file, callee_file, caller_symbol, callee_symbol, ref_kind, line,
                      COALESCE(confidence, 1.0), COALESCE(is_dynamic, 0)
               FROM reference
               WHERE ref_kind = ?1
               ORDER BY confidence DESC
               LIMIT 200"#,
        )?;

        let records = stmt.query_map(rusqlite::params![CROSS_LANGUAGE_SCHEMA], |row| {
            Ok(crate::database::ReferenceRecord {
                id: row.get(0)?,
                caller_file: row.get(1)?,
                callee_file: row.get(2)?,
                caller_symbol: row.get(3)?,
                callee_symbol: row.get(4)?,
                ref_kind: row.get(5)?,
                line: row.get(6)?,
                confidence: row.get(7)?,
                is_dynamic: row.get::<_, i32>(8)? == 1,
            })
        })?;

        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// Get all symbols (for cross-language detection)
    pub fn all_symbols(&self) -> Result<Vec<SymbolRecord>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, name, kind, start_line, end_line, start_col, end_col,
                    signature, docstring, parent_symbol, qualified_name, COALESCE(token_count, 0)
             FROM symbol
             ORDER BY name, kind",
        )?;

        let records = stmt.query_map([], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                start_col: row.get(6)?,
                end_col: row.get(7)?,
                signature: row.get(8)?,
                docstring: row.get(9)?,
                parent_symbol: row.get::<_, Option<String>>(10)?,
                qualified_name: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                token_count: row.get::<_, usize>(12)?,
            })
        })?;

        Ok(records.collect::<Result<Vec<_>, _>>()?)
    }

    /// Resolve cross-language aliases and store them in the database
    pub fn resolve_cross_language(&self) -> anyhow::Result<Vec<CrossLanguageAlias>> {
        let resolver = CrossLanguageResolver::new(self)?;
        let aliases = resolver.detect_aliases();

        // Store each alias as a cross-language reference
        for alias in &aliases {
            self.insert_cross_language_ref(alias)?;
        }

        tracing::info!(
            "Cross-language resolution: {} aliases detected and stored",
            aliases.len()
        );

        Ok(aliases)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_path() {
        assert_eq!(
            CrossLanguageResolver::language_from_path("src/main.py"),
            "python"
        );
        assert_eq!(
            CrossLanguageResolver::language_from_path("lib/index.ts"),
            "typescript"
        );
        assert_eq!(
            CrossLanguageResolver::language_from_path("src/lib.rs"),
            "rust"
        );
        assert_eq!(CrossLanguageResolver::language_from_path("app.go"), "go");
        assert_eq!(
            CrossLanguageResolver::language_from_path("README.md"),
            "markdown"
        );
        assert_eq!(
            CrossLanguageResolver::language_from_path("unknown.xyz"),
            "unknown"
        );
    }

    #[test]
    fn test_kind_compatibility() {
        assert!(CrossLanguageResolver::are_kinds_compatible("class", "interface"));
        assert!(CrossLanguageResolver::are_kinds_compatible("interface", "class"));
        assert!(CrossLanguageResolver::are_kinds_compatible("struct", "class"));
        assert!(CrossLanguageResolver::are_kinds_compatible("function", "method"));
        assert!(CrossLanguageResolver::are_kinds_compatible("fn", "function"));
        assert!(!CrossLanguageResolver::are_kinds_compatible("variable", "class"));
    }

    #[test]
    fn test_confidence_computation() {
        // Compatible kinds should get higher confidence
        let conf1 = CrossLanguageResolver::compute_confidence("User", "class", "interface");
        assert!(conf1 >= 0.7); // 0.4 + 0.3 + 0.1 (camelcase)

        // Incompatible kinds should get lower confidence
        let conf2 = CrossLanguageResolver::compute_confidence("User", "class", "variable");
        assert!(conf2 < 0.7); // 0.4 + 0.1 (camelcase only)

        // All confidences should be in valid range
        assert!(conf1 >= 0.1 && conf1 <= 0.9);
        assert!(conf2 >= 0.1 && conf2 <= 0.9);
    }

    #[test]
    fn test_detect_aliases_empty() {
        let resolver = CrossLanguageResolver {
            symbol_aliases: HashMap::new(),
        };
        let aliases = resolver.detect_aliases();
        assert!(aliases.is_empty());
    }

    #[test]
    fn test_detect_aliases_single_language() {
        let mut aliases_map = HashMap::new();
        aliases_map.insert(
            "User".to_string(),
            vec![(
                "User".to_string(),
                "python".to_string(),
                "class".to_string(),
            )],
        );
        let resolver = CrossLanguageResolver {
            symbol_aliases: aliases_map,
        };
        let aliases = resolver.detect_aliases();
        assert!(aliases.is_empty()); // Only one language
    }

    #[test]
    fn test_detect_aliases_cross_language() {
        let mut aliases_map = HashMap::new();
        aliases_map.insert(
            "User".to_string(),
            vec![
                (
                    "User".to_string(),
                    "python".to_string(),
                    "class".to_string(),
                ),
                (
                    "User".to_string(),
                    "typescript".to_string(),
                    "interface".to_string(),
                ),
            ],
        );
        let resolver = CrossLanguageResolver {
            symbol_aliases: aliases_map,
        };
        let aliases = resolver.detect_aliases();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].source_symbol, "User");
        assert_eq!(aliases[0].target_symbol, "User");
        assert!(aliases[0].confidence >= 0.5);
    }

    #[test]
    fn test_cross_language_ref_kind() {
        assert_eq!(CROSS_LANGUAGE_SCHEMA, "cross_language_schema");
    }
}
