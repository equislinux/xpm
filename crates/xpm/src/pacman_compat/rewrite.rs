//! Fallback pacman-to-xpm rewrite orchestration.

use super::*;

pub(super) fn try_rewrite(args: &[OsString]) -> Option<Vec<OsString>> {
    let mut flags = Flags::default();
    let mut operation = None;
    let mut modifiers = Vec::new();
    let mut pending_modifiers = Vec::new();
    let mut positionals = Vec::new();
    let mut after_double_dash = false;
    let mut saw_double_dash = false;
    let mut index = 1;

    while index < args.len() {
        let arg = &args[index];

        if after_double_dash {
            positionals.push(arg.clone());
            index += 1;
            continue;
        }

        let Some(arg_str) = arg.to_str() else {
            positionals.push(arg.clone());
            index += 1;
            continue;
        };

        if arg_str == "--" {
            after_double_dash = true;
            saw_double_dash = true;
            index += 1;
            continue;
        }

        if arg_str.starts_with("--") {
            index = parse_long_option(args, index, &mut flags)?;
            continue;
        }

        if let Some(rest) = arg_str.strip_prefix('-') {
            if rest.is_empty() {
                positionals.push(arg.clone());
                index += 1;
                continue;
            }

            if let Some(bundle) = parse_bundle(arg_str) {
                if operation.is_some() {
                    return None;
                }
                operation = Some(bundle.operation);
                modifiers = bundle.modifiers;
                index += 1;
                continue;
            }

            let chars: Vec<char> = rest.chars().collect();

            if chars.iter().all(|c| *c == 'v') {
                flags.globals.push(arg.clone());
                index += 1;
                continue;
            }

            if arg_str == "-h" {
                flags.help = true;
                index += 1;
                continue;
            }

            if chars.first() == Some(&'c') {
                if operation == Some('R') {
                    if chars.iter().all(|c| REMOVE_MODIFIERS.contains(c)) {
                        modifiers.extend(chars);
                        index += 1;
                        continue;
                    }
                    return None;
                }

                if chars.len() == 1 {
                    let value = args.get(index + 1)?.clone();
                    if os_starts_with_dash(&value) {
                        return None;
                    }
                    flags.globals.push(arg.clone());
                    flags.globals.push(value);
                    index += 2;
                    continue;
                }

                flags.globals.push(arg.clone());
                index += 1;
                continue;
            }

            if chars.len() == 1 {
                let letter = chars[0];
                if letter == 'r' || letter == 'b' {
                    let value = args.get(index + 1)?.clone();
                    if os_starts_with_dash(&value) {
                        return None;
                    }
                    flags.globals.push(arg.clone());
                    flags.globals.push(value);
                    index += 2;
                    continue;
                }
            }

            if chars.len() > 1 && (chars[0] == 'r' || chars[0] == 'b') {
                flags.globals.push(arg.clone());
                index += 1;
                continue;
            }

            if chars.iter().all(|c| MODIFIER_LETTERS.contains(c)) {
                if operation.is_some() {
                    modifiers.extend(chars);
                } else {
                    pending_modifiers.extend(chars);
                }
                index += 1;
                continue;
            }

            return None;
        }

        positionals.push(arg.clone());
        index += 1;
    }

    let operation = operation?;
    modifiers.extend(pending_modifiers);

    let mut command = build_command(operation, &modifiers, &flags)?;

    // `--owner` and `--file` take a value in xpm's CLI, while pacman takes the
    // path as a positional operand. When pacman callers use `--` to protect a
    // path, fold the value into the option instead of leaving `--owner -- path`
    // (which clap rejects).
    if saw_double_dash && positionals.len() == 1 {
        if let Some(index) = command
            .iter()
            .rposition(|arg| arg == "--owner" || arg == "--file")
        {
            if index + 1 == command.len() {
                let mut combined = command[index].clone();
                combined.push("=");
                combined.push(&positionals[0]);
                command[index] = combined;
                positionals.clear();
                saw_double_dash = false;
            }
        }
    }

    let mut output = Vec::with_capacity(args.len() + command.len() + 2);
    output.push(args[0].clone());
    output.extend(flags.globals);
    if flags.no_confirm {
        output.push(OsString::from("--no-confirm"));
    }
    if flags.no_color {
        output.push(OsString::from("--no-color"));
    }
    if flags.noscriptlet {
        output.push(OsString::from("--noscriptlet"));
    }
    output.extend(command);
    if flags.help {
        output.push(OsString::from("--help"));
    }
    if saw_double_dash {
        output.push(OsString::from("--"));
    }
    output.extend(positionals);

    Some(output)
}
