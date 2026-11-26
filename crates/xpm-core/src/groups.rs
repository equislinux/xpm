//! Group index and target expansion helpers.
//!
//! Group membership comes only from the `groups` metadata declared by each
//! package. Group names and member order follow package insertion order, which
//! callers populate in repository order (or local database order). Optional
//! dependencies are unrelated to group membership.
//!
//! `repo/group` parsing and repository filtering are caller-side concerns:
//! [`GroupIndex::resolve_target`] receives the bare target name.

use std::collections::{HashMap, HashSet};

use crate::resolver::Version;

/// A package entry that may belong to one or more groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPackage {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: Version,
    /// Repository name; `None` for installed/local packages.
    pub repo: Option<String>,
    /// Groups declared by this package. Membership is derived from this field.
    pub groups: Vec<String>,
}

/// Result of resolving a target name against packages, providers, and groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetResolution {
    /// A literal package has the target name.
    Package,
    /// A package provides the target name.
    Provider,
    /// No package or provider matched; the group expanded to these members.
    Group(Vec<GroupPackage>),
    /// Nothing matched.
    NotFound,
}

/// Ordered group index for one sync database or the local database.
#[derive(Debug, Clone, Default)]
pub struct GroupIndex {
    entries: Vec<GroupPackage>,
    repo_ranks: HashMap<Option<String>, usize>,
    entry_ranks: Vec<usize>,
    group_members: HashMap<String, Vec<usize>>,
    group_order: Vec<String>,
}

impl GroupIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one package, indexing it under every group it declares.
    ///
    /// Repository precedence is assigned in first-insertion order, so callers
    /// adding entries repository by repository get pacman's repo ordering.
    pub fn add(&mut self, pkg: GroupPackage) {
        let next_rank = self.repo_ranks.len();
        let rank = *self.repo_ranks.entry(pkg.repo.clone()).or_insert(next_rank);
        let entry = self.entries.len();
        let mut seen = HashSet::new();

        for group in &pkg.groups {
            if !seen.insert(group.as_str()) {
                continue;
            }
            if !self.group_members.contains_key(group) {
                self.group_order.push(group.clone());
            }
            self.group_members
                .entry(group.clone())
                .or_default()
                .push(entry);
        }

        self.entries.push(pkg);
        self.entry_ranks.push(rank);
    }

    /// Builds an index from `(repository, package)` entries in repository
    /// order. The repository name from the tuple is stored on the package.
    pub fn from_repo_entries(entries: impl IntoIterator<Item = (String, GroupPackage)>) -> Self {
        let mut index = Self::new();
        for (repo, mut pkg) in entries {
            pkg.repo = Some(repo);
            index.add(pkg);
        }
        index
    }

    /// Builds an index from local database entries, preserving their fields.
    /// Local entries should carry `repo: None`.
    pub fn from_local(packages: &[GroupPackage]) -> Self {
        let mut index = Self::new();
        for pkg in packages {
            index.add(pkg.clone());
        }
        index
    }

    /// Unique group names in first-appearance order.
    pub fn group_names(&self) -> Vec<String> {
        self.group_order.clone()
    }

    /// All members of a group in insertion order. A package present in
    /// multiple repositories is returned once per repository.
    pub fn members(&self, group: &str) -> Vec<&GroupPackage> {
        self.group_members
            .get(group)
            .map(|members| members.iter().map(|&entry| &self.entries[entry]).collect())
            .unwrap_or_default()
    }

    /// Expands a group to its members, visiting repositories in insertion
    /// order and keeping only the first package seen for each name. Member
    /// order is preserved. Returns `None` when the group does not exist.
    pub fn expand(&self, group: &str) -> Option<Vec<GroupPackage>> {
        let members = self.group_members.get(group)?;
        let mut ordered = members.clone();
        ordered.sort_by_key(|&entry| self.entry_ranks[entry]);

        let mut seen = HashSet::new();
        let mut expanded = Vec::new();
        for entry in ordered {
            let pkg = &self.entries[entry];
            if seen.insert(pkg.name.as_str()) {
                expanded.push(pkg.clone());
            }
        }
        Some(expanded)
    }

    /// Resolves an install/remove target using pacman's precedence: literal
    /// package, then provider, then group.
    ///
    /// For `repo/group`, pass only the group name; parsing the repository
    /// prefix and restricting the index to that repository is caller-side.
    pub fn resolve_target(
        &self,
        target: &str,
        package_names: &HashSet<String>,
        providers: &HashSet<String>,
    ) -> TargetResolution {
        if package_names.contains(target) {
            return TargetResolution::Package;
        }
        if providers.contains(target) {
            return TargetResolution::Provider;
        }
        match self.expand(target) {
            Some(members) => TargetResolution::Group(members),
            None => TargetResolution::NotFound,
        }
    }

    /// `(group, package)` pairs for `-Sgg`/`-Qg` output, group-major in
    /// first-appearance order and member insertion order. Packages present in
    /// multiple repositories are emitted once per repository.
    pub fn group_pairs(&self) -> Vec<(String, String)> {
        self.group_order
            .iter()
            .flat_map(|group| self.group_members_pairs(group))
            .collect()
    }

    /// `(group, package)` pairs for one group, or an empty vector when the
    /// group does not exist.
    pub fn group_members_pairs(&self, group: &str) -> Vec<(String, String)> {
        self.members(group)
            .into_iter()
            .map(|pkg| (group.to_string(), pkg.name.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests;
