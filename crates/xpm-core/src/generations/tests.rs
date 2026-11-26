use super::*;
use tempfile::TempDir;

fn write_state(root: &Path, current: &str, id: &str, packages: &str) -> PathBuf {
    let state = root.join(DEFAULT_STATE_REL);
    fs::create_dir_all(state.join(GENERATIONS_DIR).join(id)).expect("create state");
    fs::write(state.join(CURRENT_FILE), format!("{current}\n")).expect("write current");
    let tsv = state.join(GENERATIONS_DIR).join(id).join(PACKAGES_FILE);
    fs::write(&tsv, packages).expect("write packages.tsv");
    tsv
}

#[test]
fn state_dir_defaults_to_root_relative() {
    let root = Path::new("/mnt/x");
    assert_eq!(
        resolve_state_dir(root, None),
        PathBuf::from("/mnt/x/var/lib/x")
    );
    assert_eq!(
        resolve_state_dir(root, Some(OsStr::new("/custom/state"))),
        PathBuf::from("/custom/state")
    );
    assert_eq!(
        resolve_state_dir(root, Some(OsStr::new(""))),
        PathBuf::from("/mnt/x/var/lib/x"),
        "empty override falls back to the root-relative default"
    );
}

#[test]
fn reads_current_and_packages() {
    // `read_current` uses the environment; write the state under a temp
    // root and point the function there through X_GEN_STATE is racy in
    // parallel tests, so exercise the pieces directly.
    let tmp = TempDir::new().expect("tmp");
    let tsv = write_state(
        tmp.path(),
        "0003",
        "0003",
        "bash 5.2.037-1\nkitty 0.44.0-1\n",
    );
    assert!(tsv.exists());

    let packages = parse_packages_tsv(&fs::read_to_string(&tsv).expect("read"));
    assert_eq!(
        packages,
        vec![
            GenerationPackage {
                name: "bash".into(),
                version: "5.2.037-1".into()
            },
            GenerationPackage {
                name: "kitty".into(),
                version: "0.44.0-1".into()
            },
        ]
    );
}

#[test]
fn parse_skips_blanks_and_ignores_extra_columns() {
    let packages = parse_packages_tsv("\nfoo 1.0-1 extra\n\nbar 2.0-1\n");
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].version, "1.0-1");
}

#[test]
fn rejects_traversal_ids() {
    assert!(!valid_id(".."));
    assert!(!valid_id("a/b"));
    assert!(!valid_id(""));
    assert!(!valid_id(&"x".repeat(65)));
    assert!(valid_id("0001"));
    assert!(valid_id("0002-pinned"));
    assert!(generation_dir(Path::new("/"), "../etc").is_none());
}

#[test]
fn diff_classifies_added_removed_and_changed() {
    let installed: HashMap<String, String> = [
        ("bash".to_string(), "5.3.0-1".to_string()),
        ("kitty".to_string(), "0.44.0-1".to_string()),
        ("new".to_string(), "1.0-1".to_string()),
    ]
    .into_iter()
    .collect();
    let captured = vec![
        GenerationPackage {
            name: "bash".into(),
            version: "5.2.0-1".into(),
        },
        GenerationPackage {
            name: "kitty".into(),
            version: "0.44.0-1".into(),
        },
        GenerationPackage {
            name: "old".into(),
            version: "1.0-1".into(),
        },
    ];

    let diff = diff_installed(&installed, &captured, "0003");
    assert_eq!(diff.generation, "0003");
    assert!(!diff.is_empty());
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.added[0].name, "new");
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(diff.removed[0].name, "old");
    assert_eq!(diff.changed.len(), 1);
    assert_eq!(diff.changed[0].from, "5.2.0-1");
    assert_eq!(diff.changed[0].to, "5.3.0-1");
}

#[test]
fn diff_is_empty_when_identical_and_json_roundtrips() {
    let installed: HashMap<String, String> = [("kitty".to_string(), "0.44.0-1".to_string())]
        .into_iter()
        .collect();
    let captured = vec![GenerationPackage {
        name: "kitty".into(),
        version: "0.44.0-1".into(),
    }];
    let diff = diff_installed(&installed, &captured, "0001");
    assert!(diff.is_empty());

    let json = diff.to_json();
    assert!(json.contains("\"generation\":\"0001\""));
    assert!(json.contains("\"added\":[]"));
}

#[test]
fn missing_capture_reports_clear_error() {
    let tmp = TempDir::new().expect("tmp");
    let err = read_generation_packages(tmp.path(), "0001").expect_err("must fail");
    let message = err.to_string();
    assert!(
        message.contains("no package capture"),
        "unexpected message: {message}"
    );
}
