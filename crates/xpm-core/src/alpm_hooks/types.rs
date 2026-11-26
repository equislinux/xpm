//! ALPM hook data types and trigger matching.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookWhen {
    PreTransaction,
    PostTransaction,
}

/// Operation a trigger applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HookOperation {
    Install,
    Upgrade,
    Remove,
}

/// What the trigger targets are matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerType {
    #[default]
    Package,
    Path,
}

/// One `[Trigger]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trigger {
    pub operations: Vec<HookOperation>,
    pub trigger_type: TriggerType,
    pub targets: Vec<String>,
}

/// One parsed `.hook` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlpmHook {
    /// File name (used for ordering and diagnostics).
    pub name: String,
    pub description: Option<String>,
    /// Phases this hook runs in.
    pub when: Vec<HookWhen>,
    /// Command line executed without a shell.
    pub exec: String,
    /// Packages that must be installed for the hook to run.
    pub depends: Vec<String>,
    /// Abort the transaction when a pre hook fails.
    pub abort_on_fail: bool,
    /// Feed the affected package names to the hook's stdin.
    pub needs_targets: bool,
    pub triggers: Vec<Trigger>,
}

/// Context of the transaction a hook is matched against.
#[derive(Debug, Clone, Default)]
pub struct HookTransaction<'a> {
    /// `(package name, operation)` pairs.
    pub packages: Vec<(&'a str, HookOperation)>,
    /// File paths touched by the transaction and the operation that touched
    /// them (for `Path` triggers).
    pub paths: Vec<(String, HookOperation)>,
}

impl AlpmHook {
    /// Whether the hook must run for `tx`: any trigger matches and every
    /// `Depends` is satisfied by `installed`.
    pub fn matches(
        &self,
        tx: &HookTransaction<'_>,
        mut installed: impl FnMut(&str) -> bool,
    ) -> bool {
        if !self.depends.iter().all(|dep| installed(dep)) {
            return false;
        }

        self.triggers.iter().any(|trigger| {
            // Operations: empty means "any".
            let op_ok = trigger.operations.is_empty()
                || trigger
                    .operations
                    .iter()
                    .any(|op| tx.packages.iter().any(|(_, tx_op)| tx_op == op));
            if !op_ok {
                return false;
            }

            match trigger.trigger_type {
                TriggerType::Package => trigger.targets.iter().any(|pattern| {
                    tx.packages
                        .iter()
                        .any(|(name, _)| glob_match(pattern, name))
                }),
                TriggerType::Path => trigger.targets.iter().any(|pattern| {
                    tx.paths.iter().any(|(path, op)| {
                        (trigger.operations.is_empty() || trigger.operations.contains(op))
                            && glob_match(pattern, path.trim_start_matches('/'))
                    })
                }),
            }
        })
    }

    /// Whether the hook runs in `when`.
    pub fn runs_at(&self, when: HookWhen) -> bool {
        self.when.contains(&when)
    }

    /// Package names of the transaction that this hook's triggers matched.
    pub fn matched_targets(&self, tx: &HookTransaction<'_>) -> Vec<String> {
        let mut out = Vec::new();
        for trigger in &self.triggers {
            match trigger.trigger_type {
                TriggerType::Package => {
                    for (name, _) in &tx.packages {
                        if trigger
                            .targets
                            .iter()
                            .any(|pattern| glob_match(pattern, name))
                            && !out.iter().any(|seen| seen == name)
                        {
                            out.push((*name).to_string());
                        }
                    }
                }
                TriggerType::Path => {
                    for (path, op) in &tx.paths {
                        if !(trigger.operations.is_empty() || trigger.operations.contains(op)) {
                            continue;
                        }
                        let relative = path.trim_start_matches('/');
                        if trigger
                            .targets
                            .iter()
                            .any(|pattern| glob_match(pattern, relative))
                            && !out.iter().any(|seen| seen == relative)
                        {
                            out.push(relative.to_string());
                        }
                    }
                }
            }
        }
        out.sort();
        out
    }
}

/// A hook that failed during execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookFailure {
    pub name: String,
    pub abort_on_fail: bool,
    pub error: String,
}
