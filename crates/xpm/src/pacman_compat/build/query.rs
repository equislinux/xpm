//! `-Q` query translation.

use super::*;

pub(super) fn build_query_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
    if modifiers.iter().any(|c| {
        !matches!(
            c,
            'e' | 'd' | 'l' | 'i' | 'q' | 'u' | 't' | 'o' | 'k' | 'g' | 'm' | 'n' | 'c' | 'p'
        )
    }) {
        return None;
    }

    if has_install_flags(flags) || has_upgrade_flags(flags) || has_remove_flags(flags) {
        return None;
    }

    let e = count(modifiers, 'e');
    let d = count(modifiers, 'd');
    let l = count(modifiers, 'l');
    let i = count(modifiers, 'i');
    let q = count(modifiers, 'q');
    let u = count(modifiers, 'u');
    let t = count(modifiers, 't');
    let o = count(modifiers, 'o');
    let k = count(modifiers, 'k');
    let g = count(modifiers, 'g');
    let m = count(modifiers, 'm');
    let n = count(modifiers, 'n');
    let c = count(modifiers, 'c');
    let p = count(modifiers, 'p');

    // -Qc is an action: the package name follows as a positional argument.
    if c > 0 {
        if c > 1 || e + d + l + i + u + t + o + k + g + m + n + p > 0 || q > 1 {
            return None;
        }
        let mut command = vec![OsString::from("query"), OsString::from("--changelog")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        return Some(command);
    }

    // -Qp takes the package file as the next positional argument. Flags that
    // consume a value must therefore come before `--file`.
    if p > 0 {
        if p > 1 || e + d + l + i + u + t + o + k + g + m + n + c > 0 || q > 1 {
            return None;
        }
        let mut command = vec![OsString::from("query")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        command.push(OsString::from("--file"));
        return Some(command);
    }

    if o > 0 {
        if o > 1 || e + d + l + i + u + t + k + g + m + n > 0 {
            return None;
        }
        let mut command = vec![OsString::from("query")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        command.push(OsString::from("--owner"));
        return Some(command);
    }

    if k > 0 {
        if k > 2 || e + d + l + i + q + u + t + o + g + m + n > 0 {
            return None;
        }
        let mut command = vec![OsString::from("check")];
        if k == 2 {
            command.push(OsString::from("--full"));
        }
        return Some(command);
    }

    if g > 0 {
        if g > 1 || e + d + l + i + u + t + o + k + m + n > 0 {
            return None;
        }
        let mut command = vec![OsString::from("groups"), OsString::from("--local")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        return Some(command);
    }

    if l > 0 {
        if l > 1 || e > 0 || d > 0 || i > 0 || u > 0 || t > 0 || q > 0 || m > 0 || n > 0 {
            return None;
        }
        return Some(vec![OsString::from("files")]);
    }

    if i > 0 {
        if i > 1 || e > 0 || d > 0 || u > 0 || t > 0 || q > 0 || m > 0 || n > 0 {
            return None;
        }
        return Some(vec![OsString::from("info")]);
    }

    if n > 1 || m > 1 {
        return None;
    }

    if u > 0 && (t > 0 || m > 0) {
        return None;
    }

    if t > 0 {
        if t > 1 {
            return None;
        }
        let mut command = vec![OsString::from("query")];
        if m > 0 {
            command.push(OsString::from("--foreign"));
        }
        if n > 0 {
            command.push(OsString::from("--native"));
        }
        command.push(OsString::from("--unrequired"));
        if d > 0 {
            command.push(OsString::from("--deps"));
        }
        if e > 0 {
            command.push(OsString::from("--explicit"));
        }
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        return Some(command);
    }

    let mut command = vec![OsString::from("query")];
    if e > 0 {
        command.push(OsString::from("--explicit"));
    }
    if d > 0 {
        command.push(OsString::from("--deps"));
    }
    if m > 0 {
        command.push(OsString::from("--foreign"));
    }
    if n > 0 {
        command.push(OsString::from("--native"));
    }
    if u > 0 {
        command.push(OsString::from("--upgrades"));
    }
    if q > 0 {
        command.push(OsString::from("--quiet"));
    }
    Some(command)
}
