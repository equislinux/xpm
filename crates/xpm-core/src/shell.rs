//! POSIX shell quoting shared by scriptlet and hook execution.

/// Quotes `value` for safe use inside a single-quoted shell argument.
pub(crate) fn shell_quote(value: &str) -> String {
    let mut quoted = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}
