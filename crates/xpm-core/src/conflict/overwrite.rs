//! `--overwrite` pattern parsing and matching.

use super::*;

#[derive(Debug, Clone)]
struct OverwritePattern {
    negated: bool,
    pattern: Pattern,
}

/// Parsed `--overwrite` patterns.
///
/// Patterns are evaluated from last to first; the first pattern matching
/// either the relative or the rooted path decides the result. A leading
/// `!` negates a pattern, and a leading `\` escapes a literal `!` or `\`.
#[derive(Debug, Clone, Default)]
pub struct OverwritePatterns {
    patterns: Vec<OverwritePattern>,
}

impl OverwritePatterns {
    pub fn parse<I: IntoIterator<Item = S>, S: AsRef<str>>(patterns: I) -> Self {
        let mut parsed = Vec::new();
        for raw in patterns {
            for piece in raw.as_ref().split(',') {
                if piece.is_empty() {
                    continue;
                }

                let (negated, pattern) = if let Some(rest) = piece.strip_prefix('!') {
                    (true, rest)
                } else if let Some(rest) = piece.strip_prefix('\\') {
                    if rest.starts_with('!') || rest.starts_with('\\') {
                        (false, rest)
                    } else {
                        (false, piece)
                    }
                } else {
                    (false, piece)
                };

                if pattern.is_empty() {
                    continue;
                }

                if let Ok(pattern) = Pattern::new(pattern) {
                    parsed.push(OverwritePattern { negated, pattern });
                }
            }
        }
        Self { patterns: parsed }
    }

    pub fn matches(&self, relative: &str, rooted: &str) -> bool {
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: false,
        };

        for overwrite in self.patterns.iter().rev() {
            if overwrite.pattern.matches_with(relative, options)
                || overwrite.pattern.matches_with(rooted, options)
            {
                return !overwrite.negated;
            }
        }
        false
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}
