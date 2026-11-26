//! Requirement resolution and dependency ordering.

use super::*;

/// Resolve `requirements` against `candidates`, returning the install plan in
/// dependency order. The boolean marks packages requested explicitly (`true`)
/// versus pulled in as dependencies (`false`).
///
/// Dependencies may be satisfied by real package names or by unversioned
/// `provides` entries.
pub fn resolve_closure(
    candidates: Vec<PackageCandidate>,
    requirements: &[Requirement],
) -> XpmResult<Vec<(PackageCandidate, bool)>> {
    Ok(
        resolve_closure_with_installed(candidates, Vec::new(), requirements)?
            .into_iter()
            .map(|(candidate, explicit, _installed)| (candidate, explicit))
            .collect(),
    )
}

/// Resolve `requirements` while treating `installed` as already-present
/// packages.
///
/// Installed candidates are added to the pool and marked as favored for their
/// package name and every name they provide, except for names explicitly
/// requested by the user. This keeps satisfied dependencies in place (like
/// pacman) while still allowing the solver to pick a newer repository
/// candidate when a version constraint requires it.
///
/// The returned boolean pair is `(explicit, installed)`. Callers should skip
/// installed entries that are not being changed.
pub fn resolve_closure_with_installed(
    candidates: Vec<PackageCandidate>,
    installed: Vec<PackageCandidate>,
    requirements: &[Requirement],
) -> XpmResult<Vec<(PackageCandidate, bool, bool)>> {
    if requirements.is_empty() {
        return Ok(Vec::new());
    }

    let explicit_names: HashSet<&str> = requirements.iter().map(|req| req.name.as_str()).collect();

    // Repository-qualified requests (`repo/name`) restrict both the literal
    // package and its repository. Providers are not considered for these
    // requests: the user explicitly asked for a package in that repository.
    // When the same name is also requested unqualified, the restriction is
    // dropped for that name.
    let mut repo_scoped: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut unscoped_names: HashSet<&str> = HashSet::new();
    for requirement in requirements {
        match requirement.repo.as_deref() {
            Some(repo) => {
                repo_scoped
                    .entry(requirement.name.as_str())
                    .or_default()
                    .insert(repo);
            }
            None => {
                unscoped_names.insert(requirement.name.as_str());
            }
        }
    }
    let repo_only: HashMap<&str, &HashSet<&str>> = repo_scoped
        .iter()
        .filter(|(name, _)| !unscoped_names.contains(*name))
        .map(|(name, repos)| (*name, repos))
        .collect();

    for requirement in requirements.iter().filter(|req| req.repo.is_some()) {
        let repo = requirement.repo.as_deref().unwrap_or_default();
        let constraint = requirement.constraint();
        let found = candidates.iter().any(|candidate| {
            candidate.name == requirement.name
                && candidate.repo.as_deref() == Some(repo)
                && constraint.matches(&candidate.version)
        });
        if !found {
            return Err(XpmError::PackageNotFound {
                name: requirement.display(),
            });
        }
    }

    // Exclude candidates that conflict with an installed package. The solver
    // does not model packages that remain installed but are not part of the
    // solution, so this check has to happen before it runs. Explicitly
    // requested names are skipped: the caller asked to replace/upgrade them,
    // and the final dependency check will still report real conflicts.
    let mut blocked: Vec<(PackageCandidate, String, DepConstraint)> = Vec::new();
    let mut usable_candidates = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        match conflict_with_installed(&candidate, &installed, &explicit_names) {
            Some((installed_name, conflict)) => {
                blocked.push((candidate, installed_name, conflict));
            }
            None => usable_candidates.push(candidate),
        }
    }
    let candidates = usable_candidates;

    let mut pool = PackagePool::new();
    for candidate in &candidates {
        if let Some(repos) = repo_only.get(candidate.name.as_str()) {
            if !candidate
                .repo
                .as_deref()
                .is_some_and(|repo| repos.contains(repo))
            {
                continue;
            }
        }
        let solvable = pool.add_candidate(candidate.clone());
        let provides = filtered_provides(candidate, &repo_only);
        pool.add_provides(solvable, &provides);
    }

    for candidate in &installed {
        // An installed package cannot satisfy `repo/name`; force the
        // repository candidate so the request keeps its meaning.
        if repo_only.contains_key(candidate.name.as_str()) {
            continue;
        }
        let solvable = pool.add_candidate(candidate.clone());
        let provides = filtered_provides(candidate, &repo_only);
        pool.add_provides(solvable, &provides);
        pool.mark_installed(solvable);

        if !explicit_names.contains(candidate.name.as_str()) {
            if let Some(name_id) = pool.name_to_id.get(&candidate.name).copied() {
                pool.set_favored(name_id, solvable);
            }
        }
        for provide in &provides {
            if !explicit_names.contains(provide.name.as_str()) {
                if let Some(name_id) = pool.name_to_id.get(&provide.name).copied() {
                    pool.set_favored(name_id, solvable);
                }
            }
        }
    }

    // Intern dependency and conflict version sets for every candidate.
    for candidate in candidates.iter().chain(installed.iter()) {
        for dep in &candidate.depends {
            intern_once(&mut pool, dep, false);
        }
        for conflict in &candidate.conflicts {
            intern_once(&mut pool, conflict, true);
        }
    }

    // Reject requirements that nothing can satisfy before invoking the solver.
    let mut requirement_ids = Vec::new();
    for requirement in requirements {
        let constraint = requirement.constraint();
        let satisfiable = candidates
            .iter()
            .chain(installed.iter())
            .any(|candidate| requirement_satisfied_by(candidate, requirement));
        if !satisfiable {
            if let Some((candidate, installed_name, conflict)) = blocked
                .iter()
                .find(|(candidate, _, _)| requirement_satisfied_by(candidate, requirement))
            {
                return Err(XpmError::DependencyConflict(format!(
                    "cannot install {}: {} conflicts with installed package {} ('{}')",
                    requirement.display(),
                    candidate.name,
                    installed_name,
                    conflict
                )));
            }
            return Err(XpmError::PackageNotFound {
                name: requirement.display(),
            });
        }
        let name_id = pool.intern_name(&constraint.name);
        let version_set = match find_version_set(&pool, name_id, &constraint, false) {
            Some(id) => id,
            None => pool.intern_version_set(name_id, constraint),
        };
        requirement_ids.push(version_set.into());
    }

    let installed_indexes: HashSet<usize> = (0..pool.solvables.len())
        .filter(|&index| pool.is_installed(SolvableId::from_usize(index)))
        .collect();
    let snapshot = pool.solvables.clone();
    let problem = Problem::new().requirements(requirement_ids);
    let mut solver = Solver::new(XpmProvider::new(pool));
    let solution = solver
        .solve(problem)
        .map_err(|e| XpmError::DependencyConflict(format!("{e:?}")))?;

    let solved: Vec<usize> = solution.iter().map(|sid| sid.to_usize()).collect();
    let solved_set: HashSet<usize> = solved.iter().copied().collect();

    check_solution_conflicts(&solved, &solved_set, &snapshot, &installed_indexes)?;

    // Index the solution by package name and by the virtual names provided.
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for &idx in &solved {
        by_name.insert(snapshot[idx].name.as_str(), idx);
        for provide in &snapshot[idx].provides {
            by_name.entry(provide.name.as_str()).or_insert(idx);
        }
    }

    let mut visited = HashSet::new();
    let mut order = Vec::new();
    for &idx in &solved {
        order_dfs(
            idx,
            &snapshot,
            &by_name,
            &solved_set,
            &mut visited,
            &mut order,
        );
    }

    Ok(order
        .into_iter()
        .map(|idx| {
            let candidate = snapshot[idx].clone();
            let explicit = explicit_names.contains(candidate.name.as_str())
                || candidate
                    .provides
                    .iter()
                    .any(|provide| explicit_names.contains(provide.name.as_str()));
            let is_installed = installed_indexes.contains(&idx);
            (candidate, explicit, is_installed)
        })
        .collect())
}

/// Provides to register for a candidate, excluding names requested as
/// `repo/name` (those must resolve to the literal package in that repository).
fn filtered_provides(
    candidate: &PackageCandidate,
    repo_only: &HashMap<&str, &HashSet<&str>>,
) -> Vec<DepConstraint> {
    candidate
        .provides
        .iter()
        .filter(|provide| !repo_only.contains_key(provide.name.as_str()))
        .cloned()
        .collect()
}

/// Whether `candidate` satisfies a CLI requirement (name/provides, version and
/// repository qualifier).
fn requirement_satisfied_by(candidate: &PackageCandidate, requirement: &Requirement) -> bool {
    let constraint = requirement.constraint();
    match requirement.repo.as_deref() {
        Some(repo) => {
            candidate.repo.as_deref() == Some(repo)
                && candidate.name == requirement.name
                && constraint.matches(&candidate.version)
        }
        None => package_satisfies(candidate, &constraint),
    }
}

/// ALPM `depcmp`: package name first, then `provides`. A versioned dependency
/// only accepts a versioned provision whose version satisfies the constraint.
pub(super) fn package_satisfies(candidate: &PackageCandidate, dependency: &DepConstraint) -> bool {
    if candidate.name == dependency.name && dependency.matches(&candidate.version) {
        return true;
    }

    candidate.provides.iter().any(|provide| {
        if provide.name != dependency.name {
            return false;
        }
        match &dependency.version {
            None => true,
            Some(_) => provide
                .version
                .as_ref()
                .is_some_and(|version| dependency.matches(version)),
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn order_dfs(
    idx: usize,
    snapshot: &[PackageCandidate],
    by_name: &HashMap<&str, usize>,
    solved_set: &HashSet<usize>,
    visited: &mut HashSet<usize>,
    out: &mut Vec<usize>,
) {
    if !visited.insert(idx) {
        return;
    }
    for dep in &snapshot[idx].depends {
        if let Some(&dep_idx) = by_name.get(dep.name.as_str()) {
            if solved_set.contains(&dep_idx) {
                order_dfs(dep_idx, snapshot, by_name, solved_set, visited, out);
            }
        }
    }
    out.push(idx);
}
