//! `.hook` file parsing and directory discovery.

use super::*;

/// Parses one `.hook` file.
pub fn parse_hook(name: &str, content: &str) -> XpmResult<AlpmHook> {
    let mut hook = AlpmHook {
        name: name.to_string(),
        description: None,
        when: Vec::new(),
        exec: String::new(),
        depends: Vec::new(),
        abort_on_fail: false,
        needs_targets: false,
        triggers: Vec::new(),
    };

    #[derive(Clone, Copy, PartialEq)]
    enum Section {
        Trigger,
        Action,
    }

    let mut section: Option<Section> = None;
    let mut trigger: Option<Trigger> = None;

    let flush_trigger = |hook: &mut AlpmHook, trigger: &mut Option<Trigger>| {
        if let Some(trigger) = trigger.take() {
            hook.triggers.push(trigger);
        }
    };

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            flush_trigger(&mut hook, &mut trigger);
            let header = line[1..line.len() - 1].trim().to_ascii_lowercase();
            section = match header.as_str() {
                "trigger" => {
                    trigger = Some(Trigger::default());
                    Some(Section::Trigger)
                }
                "action" => Some(Section::Action),
                other => {
                    return Err(XpmError::Other(format!(
                        "{name}: unknown section [{other}]"
                    )))
                }
            };
            continue;
        }

        let Some(section) = section else {
            continue;
        };

        // Flags have no value.
        match line {
            "AbortOnFail" => {
                hook.abort_on_fail = true;
                continue;
            }
            "NeedsTargets" => {
                hook.needs_targets = true;
                continue;
            }
            _ => {}
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(XpmError::Other(format!(
                "{name}: expected `Key = value`, got `{line}`"
            )));
        };
        let key = key.trim();
        let value = value.trim();

        match section {
            Section::Trigger => {
                let Some(trigger) = trigger.as_mut() else {
                    continue;
                };
                match key {
                    "Operation" => trigger.operations.push(parse_operation(value, name)?),
                    "Type" => {
                        trigger.trigger_type = match value {
                            "Package" => TriggerType::Package,
                            "Path" => TriggerType::Path,
                            other => {
                                return Err(XpmError::Other(format!(
                                    "{name}: unknown trigger type `{other}`"
                                )))
                            }
                        }
                    }
                    "Target" => trigger.targets.push(value.to_string()),
                    _ => {}
                }
            }
            Section::Action => match key {
                "Description" => hook.description = Some(value.to_string()),
                "When" => {
                    for token in value.split(['|', ' ', ',']) {
                        match token.trim() {
                            "" => {}
                            "PreTransaction" => hook.when.push(HookWhen::PreTransaction),
                            "PostTransaction" => hook.when.push(HookWhen::PostTransaction),
                            other => {
                                return Err(XpmError::Other(format!(
                                    "{name}: unknown When `{other}`"
                                )))
                            }
                        }
                    }
                }
                "Exec" => hook.exec = value.to_string(),
                "Depends" => hook.depends.push(value.to_string()),
                _ => {}
            },
        }
    }

    flush_trigger(&mut hook, &mut trigger);

    if hook.exec.is_empty() {
        return Err(XpmError::Other(format!("{name}: missing Exec")));
    }
    if hook.triggers.is_empty() {
        return Err(XpmError::Other(format!("{name}: no [Trigger] section")));
    }
    if hook.when.is_empty() {
        // Pacman defaults to PostTransaction when When is omitted.
        hook.when.push(HookWhen::PostTransaction);
    }
    for trigger in &mut hook.triggers {
        if trigger.targets.is_empty() {
            trigger.targets.push("*".to_string());
        }
    }

    Ok(hook)
}

fn parse_operation(value: &str, name: &str) -> XpmResult<HookOperation> {
    match value {
        "Install" => Ok(HookOperation::Install),
        "Upgrade" => Ok(HookOperation::Upgrade),
        "Remove" => Ok(HookOperation::Remove),
        other => Err(XpmError::Other(format!(
            "{name}: unknown Operation `{other}`"
        ))),
    }
}

/// Loads and parses every `.hook` file from the given directories.
///
/// Files are merged by file name with later directories overriding earlier
/// ones (so `/etc/pacman.d/hooks` wins over `/usr/share/libalpm/hooks`), then
/// sorted by name — the execution order pacman uses.
pub fn load_hooks(dirs: &[PathBuf]) -> XpmResult<Vec<AlpmHook>> {
    let mut by_name: std::collections::BTreeMap<String, PathBuf> =
        std::collections::BTreeMap::new();

    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("hook") {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                by_name.insert(name.to_string(), path);
            }
        }
    }

    let mut hooks = Vec::with_capacity(by_name.len());
    for (name, path) in by_name {
        let content = fs::read_to_string(&path)?;
        let hook = parse_hook(&name, &content)
            .map_err(|e| XpmError::Other(format!("failed to parse {}: {e}", path.display())))?;
        hooks.push(hook);
    }
    Ok(hooks)
}

/// Directories to load hooks from: `XPM_ALPM_HOOKS_DIRS` when set, otherwise
/// the pacman defaults.
pub fn hook_dirs() -> Vec<PathBuf> {
    match std::env::var(ENV_HOOK_DIRS) {
        Ok(value) if !value.is_empty() => value.split(':').map(PathBuf::from).collect(),
        _ => DEFAULT_HOOK_DIRS.iter().map(PathBuf::from).collect(),
    }
}

/// Hook directories for a given installation root.
///
/// With `root == "/"` these are the pacman defaults. For an alternative root
/// the system and user hook directories are resolved inside that root, and
/// hook commands are executed chrooted (same policy as scriptlets).
pub fn hook_dirs_for(root: &Path) -> Vec<PathBuf> {
    if let Ok(value) = std::env::var(ENV_HOOK_DIRS) {
        if !value.is_empty() {
            return value.split(':').map(PathBuf::from).collect();
        }
    }
    if root == Path::new("/") {
        return DEFAULT_HOOK_DIRS.iter().map(PathBuf::from).collect();
    }
    vec![
        root.join("usr/share/libalpm/hooks"),
        root.join("etc/pacman.d/hooks"),
    ]
}
