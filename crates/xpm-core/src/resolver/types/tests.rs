use super::*;

#[test]
fn intern_name_roundtrip() {
    let mut pool = PackagePool::new();
    let id = pool.intern_name("bash");
    assert_eq!(pool.name_str(id), "bash");
    // Re-interning returns same ID
    assert_eq!(pool.intern_name("bash"), id);
}

#[test]
fn intern_string_roundtrip() {
    let mut pool = PackagePool::new();
    let id = pool.intern_string("some error");
    assert_eq!(pool.string_str(id), "some error");
    assert_eq!(pool.intern_string("some error"), id);
}

#[test]
fn add_candidate_and_lookup() {
    let mut pool = PackagePool::new();
    let sid = pool.add_candidate(PackageCandidate {
        name: "bash".to_string(),
        version: Version::parse("5.2-1"),
        repo: None,
        depends: vec![],
        conflicts: vec![],
        provides: vec![],
        replaces: vec![],
        optdepends: vec![],
    });

    let c = pool.candidate(sid);
    assert_eq!(c.name, "bash");
    assert_eq!(c.version.to_string(), "5.2-1");
}

#[test]
fn candidates_for_name() {
    let mut pool = PackagePool::new();
    let _s1 = pool.add_candidate(PackageCandidate {
        name: "bash".to_string(),
        version: Version::parse("5.1-1"),
        repo: None,
        depends: vec![],
        conflicts: vec![],
        provides: vec![],
        replaces: vec![],
        optdepends: vec![],
    });
    let _s2 = pool.add_candidate(PackageCandidate {
        name: "bash".to_string(),
        version: Version::parse("5.2-1"),
        repo: None,
        depends: vec![],
        conflicts: vec![],
        provides: vec![],
        replaces: vec![],
        optdepends: vec![],
    });

    let name_id = pool.intern_name("bash");
    let candidates = pool.candidates_for_name(name_id).unwrap();
    assert_eq!(candidates.len(), 2);
}

#[test]
fn version_set_roundtrip() {
    let mut pool = PackagePool::new();
    let name_id = pool.intern_name("glibc");
    let constraint = DepConstraint::parse("glibc>=2.38");
    let vs_id = pool.intern_version_set(name_id, constraint.clone());
    let entry = pool.version_set(vs_id);
    assert_eq!(entry.name_id, name_id);
    assert_eq!(entry.constraint, constraint);
}
