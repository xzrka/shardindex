# Changelog

All notable changes to ShardIndex will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `--with-string-refs` flag for `search` command to include string-based dynamic references
- String literal collection during AST parsing (`string_literals` table)
- Potential string reference matching (`potential_string_refs` table)
- Cross-reference Engine for detecting dynamic references through string literals

### Changed
- Search results now prioritize exact matches over prefix matches over fuzzy matches
- String reference search uses CASE WHEN ordering for better result quality
- Improved confidence scoring for string-to-symbol matching

### Fixed
- String reference search now returns relevant results instead of false positives
- Fixed issue where `sentry.models` search would return unrelated `render` methods

## [0.2.0] - 2026-05-30

### Added
- MCP server support for AI agent integration
- Deep impact analysis with transitive dependency tracing
- Dead code verification (multi-stage)
- Cross-module move analysis
- Signature migration compatibility check

### Changed
- Schema version updated to v5
- Improved PageRank scoring algorithm
- Better error handling for unindexed projects

---

## Benchmark Results

### String Reference Search (--with-string-refs)

| Query | Before | After | Improvement |
|-------|--------|-------|-------------|
| `User` | 17 matches (mixed) | 17 matches (exact first) | ✅ Exact match prioritized |
| `sentry.models` | render, datetime (false positives) | Relevant model references | ✅ False positive eliminated |
| `send_email` | 0 matches | 0 matches (no string refs) | ⚠️ Needs reindex |

### Performance

| Metric | Value |
|--------|-------|
| Search time (CLI) | ~0.09s |
| Search time (MCP server) | ~0.01s |
| String refs matched | 161 / 496,533 literals (0.03%) |

---

**Note:** For best results with `--with-string-refs`, run a fresh index with `shardindex init -p /path/to/project` to populate the `potential_string_refs` table.
