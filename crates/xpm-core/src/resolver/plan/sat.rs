//! SAT pool interning helpers.

use super::*;

pub(super) fn intern_once(pool: &mut PackagePool, constraint: &DepConstraint, negated: bool) {
    let name_id = pool.intern_name(&constraint.name);
    if find_version_set(pool, name_id, constraint, negated).is_none() {
        if negated {
            pool.intern_conflict_version_set(name_id, constraint.clone());
        } else {
            pool.intern_version_set(name_id, constraint.clone());
        }
    }
}

pub(super) fn find_version_set(
    pool: &PackagePool,
    name_id: NameId,
    constraint: &DepConstraint,
    negated: bool,
) -> Option<VersionSetId> {
    pool.version_sets
        .iter()
        .enumerate()
        .find(|(_, entry)| {
            entry.name_id == name_id && entry.negated == negated && entry.constraint == *constraint
        })
        .map(|(index, _)| VersionSetId::from_usize(index))
}
