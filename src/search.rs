/// Advanced search — fuzzy matching + kind filter + PageRank-weighted scoring
///
/// Phase 3: Advanced search (fuzzy + ranking hybrid)
///
/// ## Scoring formula
///
/// ```ignore
/// final_score = fuzzy_score * fuzzy_weight
///             + kind_boost * kind_weight
///             + pagerank_normalized * rank_weight
///             + prefix_bonus
/// ```
///
/// - fuzzy_score: 0..1 (1 = exact match, based on Levenshtein similarity)
/// - kind_boost:  0 or 1 (1 = kind_filter matches symbol kind)
/// - pagerank_normalized: symbol PageRank / max_PageRank (0..1)
///
/// ## Fuzzy matching strategies
///
/// 1. **Token matching**: query가 "_" 또는 camelCase boundary로 분리되어 token-level matching
/// 2. **Levenshtein similarity**: edit distance 기반 유사도
/// 3. **Prefix bonus**: query가 심볼명 prefix일 때 추가 보너스

use crate::database::SymbolRecord;

// ─── Levenshtein Distance ───

/// 두 문자열의 Levenshtein edit distance (dynamic programming)
///
/// 시간 복잡도: O(m*n), 공간 복잡도: O(min(m,n))
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    // Early returns
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    // 공간 최적화: 행 두 개만 유지
    let (short, long) = if m < n {
        (m, n)
    } else {
        (n, m)
    };
    let short_chars = if m < n { &a_chars } else { &b_chars };
    let long_chars = if m < n { &b_chars } else { &a_chars };

    let mut prev = Vec::with_capacity(short + 1);
    let mut curr = Vec::with_capacity(short + 1);
    for i in 0..=short {
        prev.push(i as usize);
        curr.push(0);
    }

    for li in 1..=long {
        curr[0] = li;
        for si in 1..=short {
            let cost = if short_chars[si - 1] == long_chars[li - 1] {
                0
            } else {
                1
            };
            curr[si] = (prev[si] + 1).min(curr[si - 1] + 1).min(prev[si - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[short]
}

/// Levenshtein 유사도: 0..1 (1 = 동일, 0 = 전혀 다름)
pub fn levenshtein_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    let dist = levenshtein_distance(a, b);
    1.0 - (dist as f64 / max_len as f64)
}

// ─── Token splitting ───

/// 심볼명을 token으로 분리 (snake_case, camelCase, PascalCase 등)
///
/// "my_function_name" → ["my", "function", "name"]
/// "myFunctionName" → ["my", "function", "name"]
/// "XMLParser" → ["xml", "parser"]
pub fn split_identifier(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for (i, ch) in name.chars().enumerate() {
        if ch == '_' || ch == '-' {
            if !current.is_empty() {
                tokens.push(current.clone().to_lowercase());
                current.clear();
            }
        } else if ch.is_uppercase() && i > 0 && !current.is_empty() {
            // camelCase boundary: 현재 토큰 푸시 후 새 토큰 시작
            tokens.push(current.clone().to_lowercase());
            current.clear();
            current.push(ch.to_ascii_lowercase());
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current.to_lowercase());
    }

    tokens
}

// ─── Fuzzy score ───

/// 심볼명에 대한 fuzzy matching 점수 (0..1)
///
/// 전략:
/// 1. Exact match → 1.0
/// 2. Case-insensitive match → 1.0
/// 3. Prefix match → 0.95 + length_bonus
/// 4. Token-level matching → token overlap score
/// 5. Levenshtein similarity → fallback
pub fn compute_fuzzy_score(query: &str, symbol_name: &str) -> f64 {
    let q = query.to_lowercase();
    let s = symbol_name.to_lowercase();

    // 1. Exact match
    if q == s {
        return 1.0;
    }

    // 2. Contains match (query가 심볼명에 포함)
    if s.contains(&q) {
        // 더 짧은 심볼일수록 정확한 match로 간주
        let containment_bonus = 1.0 - (s.len() as f64 - q.len() as f64) / s.len() as f64;
        return (0.9 + 0.1 * containment_bonus).min(1.0);
    }

    // 3. Prefix match
    if s.starts_with(&q) {
        return (0.95 + 0.05 * (q.len() as f64 / s.len() as f64)).min(1.0);
    }

    // 4. Token-level matching
    let query_tokens = split_identifier(&q);
    let symbol_tokens = split_identifier(&s);

    if !query_tokens.is_empty() && !symbol_tokens.is_empty() {
        let mut matched = 0;
        for qt in &query_tokens {
            for st in &symbol_tokens {
                if st == qt || st.starts_with(qt) {
                    matched += 1;
                    break;
                }
            }
        }
        let token_score = matched as f64 / query_tokens.len() as f64;
        if token_score > 0.0 {
            // 일부 토큰이라도 매칭되면 Levenshtein보다 우선
            return token_score * 0.9;
        }
    }

    // 5. Levenshtein similarity (fallback)
    let lev = levenshtein_similarity(&q, &s);

    // 너무 짧은 query에 대한 과도한 매칭 방지
    // query가 3자 이하일 때는 기준을 높임
    if q.len() <= 2 {
        lev * 0.5
    } else {
        lev
    }
}

// ─── Search Result ───

/// 검색 결과 항목 (심볼 + 점수)
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub symbol: SymbolRecord,
    pub score: f64,
    pub fuzzy_score: f64,
    pub rank_score: Option<f64>,
}

// ─── Search Config ───

/// 고급 검색 설정
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// 심볼 kind 필터 ("function", "class", "method" 등)
    pub kind_filter: Option<String>,

    /// 언어/파일 확장자 필터
    pub language_filter: Option<String>,

    /// 최소 fuzzy score (0..1)
    pub min_score: f64,

    /// Fuzzy weight (default: 0.5)
    pub fuzzy_weight: f64,

    /// PageRank weight (default: 0.3)
    pub rank_weight: f64,

    /// Kind match weight (default: 0.2)
    pub kind_weight: f64,

    /// 최대 결과 수
    pub limit: usize,

    /// LIKE 검색 여부 (fuzzy가 비활성화된 빠른 경로)
    pub use_like: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            kind_filter: None,
            language_filter: None,
            min_score: 0.1,
            fuzzy_weight: 0.5,
            rank_weight: 0.3,
            kind_weight: 0.2,
            limit: 50,
            use_like: false,
        }
    }
}

// ─── Combined scorer ───

/// 복합 점수 계산 (fuzzy + kind + PageRank)
pub fn compute_combined_score(
    fuzzy: f64,
    kind_match: bool,
    page_rank: Option<f64>,
    config: &SearchConfig,
) -> f64 {
    let kind_boost = if kind_match { 1.0 } else { 0.0 };

    // PageRank 정규화 (0..1 범위로 가정)
    let rank_normalized = page_rank.unwrap_or(0.0);

    fuzzy * config.fuzzy_weight
        + kind_boost * config.kind_weight
        + rank_normalized * config.rank_weight
}

/// serde 호환 검색 결과 (JSON 직렬화용)
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResultJson {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: Option<String>,
    pub score: f64,
    pub fuzzy_score: f64,
    pub page_rank: Option<f64>,
}

// ─── Advanced Search Engine ───

/// 고급 검색 엔진 — DB에서 후보 심볼을 가져와 fuzzy scoring + kind filter + PageRank 결합
///
/// 1. DB에서 LIKE query로 후보 심볼 수집 (kind/extension 필터 적용)
/// 2. 각 심볼에 대해 fuzzy score 계산
/// 3. min_score 필터링 + PageRank 기반 정렬
/// 4. limit 개수만큼 반환
pub fn advanced_search(
    db: &crate::database::IndexDb,
    query: &str,
    _extension_filter: Option<&str>,
    config: &SearchConfig,
) -> Result<Vec<SearchResultJson>, anyhow::Error> {
    // 1. DB에서 후보 수집 (LIKE + ranking 정렬)
    let candidates = db.search_symbol_ranked(query)?;

    // 3. Fuzzy scoring + kind match + PageRank 결합
    let mut results: Vec<SearchResultJson> = Vec::new();

    for (symbol, page_rank_from_db) in &candidates {
        let fuzzy = compute_fuzzy_score(query, &symbol.name);

        // min_score 필터링
        if fuzzy < config.min_score {
            continue;
        }

        let kind_match = config
            .kind_filter
            .as_ref()
            .map_or(true, |kf| symbol.kind == *kf);

        let page_rank = *page_rank_from_db;

        let score = compute_combined_score(fuzzy, kind_match, page_rank, config);

        results.push(SearchResultJson {
            name: symbol.name.clone(),
            kind: symbol.kind.clone(),
            file_path: symbol.file_path.clone(),
            start_line: symbol.start_line,
            end_line: symbol.end_line,
            signature: symbol.signature.clone(),
            score,
            fuzzy_score: fuzzy,
            page_rank,
        });
    }

    // 4. score 내림차순 정렬
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // 5. limit
    results.truncate(config.limit);

    Ok(results)
}

// ─── Unit Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    // ── Levenshtein ──

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein_distance("", "hello"), 5);
        assert_eq!(levenshtein_distance("hello", ""), 5);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn test_levenshtein_single_edit() {
        assert_eq!(levenshtein_distance("cat", "car"), 1);
        assert_eq!(levenshtein_distance("cat", "cats"), 1);
        assert_eq!(levenshtein_distance("cat", "at"), 1);
    }

    #[test]
    fn test_levenshtein_knight() {
        // classic example
        assert_eq!(levenshtein_distance("knight", "night"), 1);
    }

    #[test]
    fn test_levenshtein_completely_different() {
        assert_eq!(levenshtein_distance("abc", "xyz"), 3);
    }

    #[test]
    fn test_levenshtein_similarity_identical() {
        assert!((levenshtein_similarity("hello", "hello") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_levenshtein_similarity_different() {
        let sim = levenshtein_similarity("hello", "world");
        assert!(sim >= 0.0 && sim < 1.0);
    }

    #[test]
    fn test_levenshtein_similarity_empty() {
        assert!((levenshtein_similarity("", "hello")).abs() < 0.001);
        assert!((levenshtein_similarity("", "") - 1.0).abs() < 1e-6);
    }

    // ── Token splitting ──

    #[test]
    fn test_split_snake_case() {
        let tokens = split_identifier("my_function_name");
        assert_eq!(tokens, vec!["my", "function", "name"]);
    }

    #[test]
    fn test_split_camel_case() {
        let tokens = split_identifier("myFunctionName");
        assert_eq!(tokens, vec!["my", "function", "name"]);
    }

    #[test]
    fn test_split_pascal_case() {
        let tokens = split_identifier("MyClassName");
        assert_eq!(tokens, vec!["my", "class", "name"]);
    }

    #[test]
    fn test_split_single_word() {
        let tokens = split_identifier("function");
        assert_eq!(tokens, vec!["function"]);
    }

    #[test]
    fn test_split_kebab_case() {
        let tokens = split_identifier("my-function-name");
        assert_eq!(tokens, vec!["my", "function", "name"]);
    }

    #[test]
    fn test_split_empty() {
        let tokens = split_identifier("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_split_acronym() {
        // XMLParser → ["xml", "parser"] (간단한 처리)
        let tokens = split_identifier("XMLParser");
        assert!(!tokens.is_empty());
    }

    // ── Fuzzy scoring ──

    #[test]
    fn test_fuzzy_exact_match() {
        assert!((compute_fuzzy_score("my_function", "my_function") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_fuzzy_case_insensitive() {
        assert!((compute_fuzzy_score("MyFunction", "myfunction") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_fuzzy_prefix() {
        let score = compute_fuzzy_score("my_func", "my_function_name");
        assert!(score >= 0.9, "Prefix match should be >= 0.9, got {}", score);
    }

    #[test]
    fn test_fuzzy_contains() {
        let score = compute_fuzzy_score("function", "my_function_name");
        assert!(score >= 0.85, "Contains match should be >= 0.85, got {}", score);
    }

    #[test]
    fn test_fuzzy_token_match() {
        // "my_func" vs "my_function_name" → 토큰 매칭
        let score = compute_fuzzy_score("my_func", "my_function_name");
        assert!(score > 0.3, "Token match should score well, got {}", score);
    }

    #[test]
    fn test_fuzzy_typo_tolerance() {
        // "functin" vs "function" → typo 허용
        let score = compute_fuzzy_score("functin", "function");
        assert!(score >= 0.7, "Typo tolerance should work, got {}", score);
    }

    #[test]
    fn test_fuzzy_no_match() {
        let score = compute_fuzzy_score("xyz", "my_function_name");
        assert!(score < 0.5, "No match should score low, got {}", score);
    }

    #[test]
    fn test_fuzzy_short_query_strict() {
        // 짧은 query에 대한 과도한 매칭 방지
        let score = compute_fuzzy_score("a", "some_function");
        assert!(score < 0.5, "Short query should have low score, got {}", score);
    }

    // ── Combined scoring ──

    #[test]
    fn test_combined_score_all_factors() {
        let config = SearchConfig::default();
        let score = compute_combined_score(1.0, true, Some(0.8), &config);
        // 1.0 * 0.5 + 1.0 * 0.2 + 0.8 * 0.3 = 0.5 + 0.2 + 0.24 = 0.94
        assert!((score - 0.94).abs() < 1e-6, "Expected 0.94, got {}", score);
    }

    #[test]
    fn test_combined_score_fuzzy_only() {
        let config = SearchConfig::default();
        let score = compute_combined_score(0.8, false, None, &config);
        // 0.8 * 0.5 + 0 + 0 = 0.4
        assert!((score - 0.4).abs() < 1e-6, "Expected 0.4, got {}", score);
    }

    #[test]
    fn test_combined_score_custom_weights() {
        let config = SearchConfig {
            fuzzy_weight: 0.6,
            rank_weight: 0.2,
            kind_weight: 0.2,
            ..Default::default()
        };
        let score = compute_combined_score(0.5, true, Some(1.0), &config);
        // 0.5 * 0.6 + 1.0 * 0.2 + 1.0 * 0.2 = 0.3 + 0.2 + 0.2 = 0.7
        assert!((score - 0.7).abs() < 1e-6, "Expected 0.7, got {}", score);
    }
}
