use super::*;

#[test]
fn diff_compares_against_generation_capture() {
    let fixture = Fixture::new();
    fixture.install_into_db("hello", "0.9-1");

    let generation_dir = fixture.root.join("var/lib/x/generations/0007");
    fs::create_dir_all(&generation_dir).expect("generation dir");
    fs::write(
        generation_dir.join("packages.tsv"),
        "hello 1.0-1\nother 2.0-1\n",
    )
    .expect("packages.tsv");

    let run = fixture.run(&["diff", "0007", "--json"]);
    assert!(
        run.status.success(),
        "diff failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("\"generation\":\"0007\""), "json: {stdout}");
    assert!(
        stdout.contains("\"name\":\"other\"") && stdout.contains("\"removed\""),
        "expected removed package in diff: {stdout}"
    );
    assert!(
        stdout.contains("\"name\":\"hello\"") && stdout.contains("\"from\":\"1.0-1\""),
        "expected changed package in diff: {stdout}"
    );
}

#[test]
fn history_links_generation_and_diff_current_resolves_it() {
    let fixture = Fixture::new();
    fixture.install_into_db("hello", "0.9-1");

    // `current` points at generation 0009 which captured the same package.
    let state = fixture.root.join("var/lib/x");
    let generation_dir = state.join("generations/0009");
    fs::create_dir_all(&generation_dir).expect("generation dir");
    fs::write(state.join("current"), "0009\n").expect("current");
    fs::write(generation_dir.join("packages.tsv"), "hello 0.9-1\n").expect("packages.tsv");

    let diff = fixture.run(&["diff", "current"]);
    assert!(
        diff.status.success(),
        "diff current failed: {}",
        String::from_utf8_lossy(&diff.stderr)
    );
    assert!(
        String::from_utf8_lossy(&diff.stdout).contains("No differences"),
        "expected a clean diff: {}",
        String::from_utf8_lossy(&diff.stdout)
    );

    // Journal with a linked generation shows the marker in history.
    let mut journal = fixture.record_journal(vec![JournalPackage::install("hello", "0.9-1")]);
    journal.generation = Some("0009".to_string());
    journal.persist().expect("persist with generation");

    let history = fixture.run(&["history"]);
    assert!(
        String::from_utf8_lossy(&history.stdout).contains("gen:0009"),
        "history must show the generation: {}",
        String::from_utf8_lossy(&history.stdout)
    );
}
