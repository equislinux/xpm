//! Shared command helpers: confirmations, path display and exit codes.

use super::*;

pub(super) fn display_rel_or_abs(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(|p| p.display().to_string()))
        .unwrap_or_else(|| path.display().to_string())
}

pub(super) fn confirm_action(prompt: &str, no_confirm: bool) -> Result<()> {
    if no_confirm {
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        return Err(XpmError::Other(
            "confirmation required but stdin is not interactive; use --no-confirm".to_string(),
        )
        .into());
    }

    print!("{}", prompt);
    io::stdout().flush().context("failed to flush prompt")?;

    let mut input = String::new();
    let bytes = io::stdin()
        .read_line(&mut input)
        .context("failed to read confirmation")?;

    if bytes == 0 {
        return Err(XpmError::Other(
            "confirmation prompt received EOF; use --no-confirm for non-interactive mode"
                .to_string(),
        )
        .into());
    }

    let answer = input.trim().to_ascii_lowercase();
    if answer == "y" || answer == "yes" {
        Ok(())
    } else {
        Err(XpmError::Other("operation cancelled by user".to_string()).into())
    }
}

pub(super) fn confirm_replace(old: &str, repo: &str, new: &str, no_confirm: bool) -> Result<bool> {
    if no_confirm || !io::stdin().is_terminal() {
        return Ok(true);
    }

    loop {
        print!("Replace {old} with {repo}/{new}? [Y/n] ");
        io::stdout().flush().context("failed to flush prompt")?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read replacement answer")?;
        match input.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => continue,
        }
    }
}

pub(super) fn exit_with(code: i32) -> ! {
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    std::process::exit(code);
}
