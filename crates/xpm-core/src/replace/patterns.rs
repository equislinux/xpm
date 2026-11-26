//! `IgnorePkg`/`IgnoreGroup` pattern matching.

use super::*;

#[derive(Debug, Clone)]
struct IgnorePattern {
    negated: bool,
    pattern: Pattern,
}

#[derive(Debug, Clone, Default)]
pub(super) struct IgnorePatterns {
    patterns: Vec<IgnorePattern>,
}

impl IgnorePatterns {
    pub(super) fn new(patterns: &[String]) -> Self {
        let patterns = patterns
            .iter()
            .filter_map(|raw| {
                let (negated, pattern) = if let Some(rest) = raw.strip_prefix('!') {
                    (true, rest)
                } else if let Some(rest) = raw.strip_prefix('\\') {
                    if rest.starts_with('!') || rest.starts_with('\\') {
                        (false, rest)
                    } else {
                        (false, raw.as_str())
                    }
                } else {
                    (false, raw.as_str())
                };

                if pattern.is_empty() {
                    return None;
                }

                Pattern::new(pattern)
                    .ok()
                    .map(|pattern| IgnorePattern { negated, pattern })
            })
            .collect();
        Self { patterns }
    }

    pub(super) fn matches(&self, value: &str) -> bool {
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: false,
        };

        for entry in self.patterns.iter().rev() {
            if entry.pattern.matches_with(value, options) {
                return !entry.negated;
            }
        }
        false
    }
}

pub(super) fn is_ignored(
    name: &str,
    groups: &[String],
    ignore_pkg: &IgnorePatterns,
    ignore_group: &IgnorePatterns,
) -> bool {
    ignore_pkg.matches(name) || groups.iter().any(|group| ignore_group.matches(group))
}

/// Whether any of `groups` matches an `IgnoreGroup` pattern.
///
/// The same glob and `!`-negation semantics as `IgnorePkg` are used, so the
/// CLI's `--ignoregroup` can share the config-based matching.
pub fn matches_ignore_group(groups: &[String], patterns: &[String]) -> bool {
    let patterns = IgnorePatterns::new(patterns);
    groups.iter().any(|group| patterns.matches(group))
}
