use super::*;

#[test]
fn mark_and_deptest_pacman_mappings() {
    let fixture = Fixture::new();
    fixture.add_package("glibc", "2.39-1");
    fixture.add_package("hello", "1.0-1");
    fixture.write_reason("hello", "explicit");

    fixture.add_package("firefox", "128.0-1");
    fixture.write_list("firefox", "provides", &["browser=128.0"]);

    let satisfied = fixture.run(&["-T", "glibc>=2.38", "browser>=120", "hello"]);
    assert!(satisfied.status.success(), "{}", stderr(&satisfied));
    assert!(stdout(&satisfied).is_empty());

    let unmet = fixture.run(&["-T", "glibc>=3", "browser>=130", "missing"]);
    assert_eq!(unmet.status.code(), Some(127));
    assert_eq!(stdout(&unmet), "glibc>=3\nbrowser>=130\nmissing\n");

    let as_deps = fixture.run(&["-D", "--asdeps", "hello"]);
    assert!(as_deps.status.success(), "{}", stderr(&as_deps));
    assert!(stdout(&as_deps).contains("Marked 1 package(s)"));
    assert_eq!(fixture.read_reason("hello"), "dep");

    let as_explicit = fixture.run(&["-D", "--asexplicit", "hello"]);
    assert!(as_explicit.status.success(), "{}", stderr(&as_explicit));
    assert_eq!(fixture.read_reason("hello"), "explicit");

    let unknown = fixture.run(&["-D", "--asdeps", "does-not-exist"]);
    assert!(!unknown.status.success());
    assert_eq!(fixture.read_reason("hello"), "explicit");
}
