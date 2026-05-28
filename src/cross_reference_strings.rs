use crate::database::IndexDb;
use anyhow::Context;
use tracing::debug;

/// 심볼-문자열 교차 매칭 엔진
///
/// 인덱싱된 심볼명과 AST에서 추출한 문자열 리터럴을
/// 문자열 매칭 + AST 컨텍스트 가중치로 연결.
pub struct CrossReferenceStrings {
    db: IndexDb,
}

impl CrossReferenceStrings {
    pub fn new(db: IndexDb) -> Self {
        Self { db }
    }

    /// 교차 매칭 실행 — 심볼 유사 문자열 리터럴을 심볼에 매핑
    pub fn run(&self) -> Result<usize, anyhow::Error> {
        // 심볼 유사 문자열 리터럴 조회
        let literals = self.db.get_symbol_like_literals()?;
        debug!("Found {} symbol-like string literals", literals.len());

        if literals.is_empty() {
            return Ok(0);
        }

        // 심볼명 컬렉션 (매칭용)
        let symbols = self.db.all_symbols()?;
        let symbol_names: Vec<String> = symbols.iter().map(|s| s.name.clone()).collect();
        let qualified_names: Vec<String> = symbols.iter().map(|s| s.qualified_name.clone()).collect();

        let mut match_count = 0;

        for (literal_id, _file_path, string_value, _line, context, _parent_fn) in &literals {
            // 심볼명과 문자열 매칭 시도
            for (idx, sym) in symbols.iter().enumerate() {
                let confidence = self.calculate_confidence(
                    string_value,
                    &sym.name,
                    &qualified_names[idx],
                    context.as_str(),
                );

                if confidence > 0.0 {
                    self.db.insert_potential_string_ref(
                        *literal_id,
                        sym.id,
                        confidence,
                        if confidence >= 0.8 {
                            "exact"
                        } else if confidence >= 0.5 {
                            "partial"
                        } else {
                            "fuzzy"
                        },
                    )?;
                    match_count += 1;
                }
            }
        }

        debug!("Created {} potential string references", match_count);
        Ok(match_count)
    }

    /// 매칭 신뢰도 계산
    ///
    /// - exact_match: 1.0 (완전 일치)
    /// - suffix_match: 0.8 (마지막 세그먼트 일치 — "sentry.User" vs "User")
    /// - partial_match: 0.5 (부분 일치)
    /// - 컨텍스트 보정: function_arg +0.1, sequence_element +0.05
    fn calculate_confidence(
        &self,
        string_value: &str,
        symbol_name: &str,
        qualified_name: &str,
        context: &str,
    ) -> f64 {
        // 기본 매칭
        let base = if string_value == symbol_name {
            1.0
        } else if string_value == qualified_name {
            1.0
        } else if self.suffix_matches(string_value, symbol_name) {
            0.8
        } else if self.suffix_matches(string_value, qualified_name) {
            0.7
        } else if self.contains_as_segment(string_value, symbol_name) {
            0.5
        } else {
            0.0
        };

        if base == 0.0 {
            return 0.0;
        }

        // 컨텍스트 보정
        let context_bonus = match context {
            "function_arg" => 0.1,
            "assignment_rhs" => 0.05,
            "sequence_element" => 0.05,
            _ => 0.0,
        };

        let result: f64 = base + context_bonus;
        result.min(1.0)
    }

    /// 마지막 세그먼트가 일치하는지 확인
    /// "sentry.models.user.User" vs "User" → true
    fn suffix_matches(&self, string_value: &str, symbol_name: &str) -> bool {
        if string_value == symbol_name {
            return true;
        }
        let last_segment = string_value.rsplit('.').next();
        last_segment == Some(symbol_name)
    }

    /// 문자열이 심볼명을 포함하는지 확인
    /// "sentry.User" vs "User" → true
    fn contains_as_segment(&self, string_value: &str, symbol_name: &str) -> bool {
        string_value
            .split('.')
            .any(|seg| seg == symbol_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> CrossReferenceStrings {
        CrossReferenceStrings::new(IndexDb::open_in_memory().unwrap())
    }

    #[test]
    fn test_exact_match() {
        let engine = make_engine();
        let conf = engine.calculate_confidence("User", "User", "models.User", "function_arg");
        assert!((conf - 1.0).abs() < 0.01); // 1.0 (ceiling)
    }

    #[test]
    fn test_qualified_match() {
        let engine = make_engine();
        let conf = engine.calculate_confidence(
            "models.User",
            "User",
            "models.User",
            "function_arg",
        );
        assert!((conf - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_suffix_match() {
        let engine = make_engine();
        let conf = engine.calculate_confidence(
            "sentry.models.user.User",
            "User",
            "models.User",
            "function_arg",
        );
        assert!(conf >= 0.8 && conf <= 0.9); // 0.8 + 0.1 context
    }

    #[test]
    fn test_segment_contain() {
        let engine = make_engine();
        // "sentry.User" vs "User" → suffix_match (0.7 for qualified)
        let conf = engine.calculate_confidence("sentry.User", "User", "models.User", "unknown");
        assert!(conf >= 0.5 && conf < 1.0);
    }

    #[test]
    fn test_no_match() {
        let engine = make_engine();
        let conf = engine.calculate_confidence("hello world", "User", "models.User", "unknown");
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn test_context_bonus() {
        let engine = make_engine();
        let conf_no_context = engine.calculate_confidence("sentry.User", "User", "models.User", "unknown");
        let conf_with_context = engine.calculate_confidence("sentry.User", "User", "models.User", "function_arg");
        assert!(conf_with_context > conf_no_context);
    }

    #[test]
    fn test_suffix_matches() {
        let engine = make_engine();
        assert!(engine.suffix_matches("sentry.models.User", "User"));
        assert!(engine.suffix_matches("User", "User"));
        assert!(!engine.suffix_matches("sentry.models.User", "Admin"));
    }

    #[test]
    fn test_contains_as_segment() {
        let engine = make_engine();
        assert!(engine.contains_as_segment("sentry.User", "User"));
        assert!(engine.contains_as_segment("sentry.models.User", "User"));
        assert!(!engine.contains_as_segment("sentry.User", "Admin"));
    }
}
