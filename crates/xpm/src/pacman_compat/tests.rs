use super::rewrite_args;
use std::ffi::OsString;

fn rewrite(input: &[&str]) -> Vec<String> {
    rewrite_args(input.iter().map(OsString::from))
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn assert_rewrite(input: &[&str], expected: &[&str]) {
    assert_eq!(rewrite(input), expected, "input: {input:?}");
}

#[test]
fn rewrites_full_upgrade() {
    assert_rewrite(
        &["xpm", "-Syu", "--noconfirm"],
        &["xpm", "--no-confirm", "upgrade"],
    );
    assert_rewrite(&["xpm", "-Suy"], &["xpm", "upgrade"]);
    assert_rewrite(&["xpm", "-Su"], &["xpm", "upgrade"]);
}

#[test]
fn rewrites_sync() {
    assert_rewrite(&["xpm", "-Sy"], &["xpm", "sync"]);
    assert_rewrite(&["xpm", "-Syy"], &["xpm", "sync", "--force"]);
}

#[test]
fn rewrites_upgrade_ignore_after_subcommand() {
    assert_rewrite(
        &["xpm", "-Syu", "--ignore", "linux"],
        &["xpm", "upgrade", "--ignore", "linux"],
    );
}

#[test]
fn rewrites_install_flags() {
    assert_rewrite(
        &["xpm", "-S", "--needed", "--noconfirm", "firefox"],
        &["xpm", "--no-confirm", "install", "--needed", "firefox"],
    );
    assert_rewrite(&["xpm", "-S", "firefox"], &["xpm", "install", "firefox"]);
}

#[test]
fn rewrites_install_print() {
    assert_rewrite(
        &["xpm", "-Sp", "firefox"],
        &["xpm", "install", "--print", "firefox"],
    );
}

#[test]
fn rewrites_local_package_install() {
    assert_rewrite(
        &["xpm", "-U", "/tmp/foo.pkg.tar.zst"],
        &["xpm", "install", "/tmp/foo.pkg.tar.zst"],
    );
}

#[test]
fn rewrites_remove_bundles() {
    assert_rewrite(
        &["xpm", "-Rns", "pkg"],
        &["xpm", "remove", "--recursive", "--nosave", "pkg"],
    );
    assert_rewrite(
        &["xpm", "-Rsn", "pkg"],
        &["xpm", "remove", "--recursive", "--nosave", "pkg"],
    );
    assert_rewrite(&["xpm", "-Rdd", "pkg"], &["xpm", "remove", "-dd", "pkg"]);
    assert_rewrite(
        &["xpm", "-Rc", "pkg"],
        &["xpm", "remove", "--cascade", "pkg"],
    );
    assert_rewrite(
        &["xpm", "-Ru", "pkg"],
        &["xpm", "remove", "--unneeded", "pkg"],
    );
}

#[test]
fn rewrites_query_variants() {
    assert_rewrite(&["xpm", "-Qe"], &["xpm", "query", "--explicit"]);
    assert_rewrite(&["xpm", "-Qd"], &["xpm", "query", "--deps"]);
    assert_rewrite(&["xpm", "-Qq"], &["xpm", "query", "--quiet"]);
    assert_rewrite(&["xpm", "-Qi", "pkg"], &["xpm", "info", "pkg"]);
    assert_rewrite(&["xpm", "-Ql", "pkg"], &["xpm", "files", "pkg"]);
}

#[test]
fn rewrites_query_repair_and_database_operations() {
    assert_rewrite(
        &["xpm", "-Qo", "/usr/bin/foo"],
        &["xpm", "query", "--owner", "/usr/bin/foo"],
    );
    assert_rewrite(
        &["xpm", "-Qoq", "--", "/usr/bin/foo"],
        &["xpm", "query", "--quiet", "--owner=/usr/bin/foo"],
    );
    assert_rewrite(&["xpm", "-Qm"], &["xpm", "query", "--foreign"]);
    assert_rewrite(&["xpm", "-Qmq"], &["xpm", "query", "--foreign", "--quiet"]);
    assert_rewrite(&["xpm", "-Qt"], &["xpm", "query", "--unrequired"]);
    assert_rewrite(
        &["xpm", "-Qdt"],
        &["xpm", "query", "--unrequired", "--deps"],
    );
    assert_rewrite(
        &["xpm", "-Qtd"],
        &["xpm", "query", "--unrequired", "--deps"],
    );
    assert_rewrite(
        &["xpm", "-Qteq"],
        &["xpm", "query", "--unrequired", "--explicit", "--quiet"],
    );
    assert_rewrite(&["xpm", "-Qg"], &["xpm", "groups", "--local"]);
    assert_rewrite(&["xpm", "-Qqg"], &["xpm", "groups", "--local", "--quiet"]);
    assert_rewrite(&["xpm", "-Qk"], &["xpm", "check"]);
    assert_rewrite(&["xpm", "-Qkk"], &["xpm", "check", "--full"]);
    assert_rewrite(
        &["xpm", "-Qkk", "hello"],
        &["xpm", "check", "--full", "hello"],
    );
    assert_rewrite(
        &["xpm", "-D", "--asdeps", "hello"],
        &["xpm", "mark", "--as-deps", "hello"],
    );
    assert_rewrite(
        &["xpm", "-D", "--asexplicit", "hello"],
        &["xpm", "mark", "--as-explicit", "hello"],
    );
    assert_rewrite(
        &["xpm", "-T", "glibc>=2.39", "bash"],
        &["xpm", "deptest", "glibc>=2.39", "bash"],
    );
}

#[test]
fn rewrites_sync_listing_group_and_clean_operations() {
    assert_rewrite(&["xpm", "-Sl"], &["xpm", "list"]);
    assert_rewrite(&["xpm", "-Slq"], &["xpm", "list", "--quiet"]);
    assert_rewrite(&["xpm", "-Sl", "core"], &["xpm", "list", "core"]);
    assert_rewrite(&["xpm", "-Sg"], &["xpm", "groups", "--names"]);
    assert_rewrite(&["xpm", "-Sgg"], &["xpm", "groups"]);
    assert_rewrite(
        &["xpm", "-Sgq", "desktop"],
        &["xpm", "groups", "--names", "--quiet", "desktop"],
    );
    assert_rewrite(&["xpm", "-Sc"], &["xpm", "clean"]);
    assert_rewrite(&["xpm", "-Scc"], &["xpm", "clean", "--all"]);
}

#[test]
fn rewrites_download_only_sync_and_upgrade() {
    assert_rewrite(
        &["xpm", "-Sw", "firefox"],
        &["xpm", "install", "--download-only", "firefox"],
    );
    assert_rewrite(&["xpm", "-Suw"], &["xpm", "upgrade", "--download-only"]);
    assert_rewrite(
        &["xpm", "-Syu", "--downloadonly"],
        &["xpm", "upgrade", "--download-only"],
    );
}

#[test]
fn rewrites_native_file_and_changelog_queries() {
    assert_rewrite(&["xpm", "-Qn"], &["xpm", "query", "--native"]);
    assert_rewrite(&["xpm", "-Qnq"], &["xpm", "query", "--native", "--quiet"]);
    assert_rewrite(
        &["xpm", "-Qnt"],
        &["xpm", "query", "--native", "--unrequired"],
    );
    assert_rewrite(
        &["xpm", "-Qc", "hello"],
        &["xpm", "query", "--changelog", "hello"],
    );
    assert_rewrite(
        &["xpm", "-Qcq", "hello"],
        &["xpm", "query", "--changelog", "--quiet", "hello"],
    );
    assert_rewrite(
        &["xpm", "-Qp", "/tmp/hello.pkg.tar.zst"],
        &["xpm", "query", "--file", "/tmp/hello.pkg.tar.zst"],
    );
    assert_rewrite(
        &["xpm", "-Qpq", "/tmp/hello.pkg.tar.zst"],
        &[
            "xpm",
            "query",
            "--quiet",
            "--file",
            "/tmp/hello.pkg.tar.zst",
        ],
    );
}

#[test]
fn rewrites_files_database_queries() {
    assert_rewrite(
        &["xpm", "-F", "usr/bin/hello"],
        &["xpm", "file-db", "usr/bin/hello"],
    );
    assert_rewrite(
        &["xpm", "-Fl", "hello"],
        &["xpm", "file-db", "--list", "hello"],
    );
    assert_rewrite(
        &["xpm", "-Fq", "usr/bin/hello"],
        &["xpm", "file-db", "--quiet", "usr/bin/hello"],
    );
    assert_rewrite(&["xpm", "-Fy"], &["xpm", "file-db", "--refresh"]);
    assert_rewrite(
        &["xpm", "-Fyy"],
        &["xpm", "file-db", "--refresh", "--refresh"],
    );
}

#[test]
fn rewrites_transaction_options_and_sysroot() {
    assert_rewrite(
        &[
            "xpm",
            "-S",
            "--ignoregroup",
            "kernel",
            "--assume-installed",
            "linux=6.9",
            "pkg",
        ],
        &[
            "xpm",
            "install",
            "--ignoregroup",
            "kernel",
            "--assume-installed",
            "linux=6.9",
            "pkg",
        ],
    );
    assert_rewrite(
        &["xpm", "-Syu", "--ignoregroup", "kernel"],
        &["xpm", "upgrade", "--ignoregroup", "kernel"],
    );
    assert_rewrite(
        &["xpm", "--sysroot", "/mnt", "-Syu"],
        &["xpm", "--sysroot", "/mnt", "upgrade"],
    );
}

#[test]
fn rewrites_version() {
    assert_rewrite(&["xpm", "-V"], &["xpm", "--version"]);
}

#[test]
fn leaves_native_invocations_unchanged() {
    assert_rewrite(&["xpm", "install", "foo"], &["xpm", "install", "foo"]);
    assert_rewrite(&["xpm", "Q", "foo"], &["xpm", "Q", "foo"]);
    assert_rewrite(
        &["xpm", "--root", "/mnt", "install", "foo"],
        &["xpm", "--root", "/mnt", "install", "foo"],
    );
}

#[test]
fn leaves_unsupported_operations_unchanged() {
    for args in [
        vec!["xpm", "-Qkkk"],
        vec!["xpm", "-Qo", "-Qm"],
        vec!["xpm", "-Qgl"],
        vec!["xpm", "-D", "pkg"],
        vec!["xpm", "-D", "--asdeps", "--asexplicit", "pkg"],
        vec!["xpm", "-T", "--needed", "pkg"],
        vec!["xpm", "-Fx", "usr/bin/.*"],
        vec!["xpm", "-Fz"],
        vec!["xpm", "--gpgdir", "/tmp/gnupg", "-Syu"],
    ] {
        assert_eq!(rewrite(&args), args, "input: {args:?}");
    }
}

#[test]
fn global_root_and_dbpath_pass_through() {
    assert_rewrite(
        &["xpm", "--root", "/mnt", "--dbpath", "/mnt/db", "-Syu"],
        &["xpm", "--root", "/mnt", "--dbpath", "/mnt/db", "upgrade"],
    );
    assert_rewrite(
        &["xpm", "-r", "/mnt", "-b", "/mnt/db", "-Q"],
        &["xpm", "-r", "/mnt", "-b", "/mnt/db", "query"],
    );
}

#[test]
fn rewrites_global_aliases() {
    assert_rewrite(
        &["xpm", "-S", "--asdeps", "--downloadonly", "foo"],
        &["xpm", "install", "--as-deps", "--download-only", "foo"],
    );
    assert_rewrite(
        &["xpm", "-Syu", "--color", "never", "--noconfirm"],
        &["xpm", "--no-confirm", "--no-color", "upgrade"],
    );
}

#[test]
fn unknown_color_mode_is_rejected() {
    let args = vec!["xpm", "-Syu", "--color", "always"];
    assert_eq!(rewrite(&args), args);
}

#[test]
fn preserves_double_dash_and_positional_order() {
    assert_rewrite(
        &["xpm", "-S", "--", "-odd-name", "normal"],
        &["xpm", "install", "--", "-odd-name", "normal"],
    );
}

#[test]
fn rejects_multiple_operation_bundles() {
    let args = vec!["xpm", "-Syu", "-Rns", "pkg"];
    assert_eq!(rewrite(&args), args);
}

#[test]
fn rewritten_commands_parse_with_clap() {
    use crate::cli::Cli;
    use clap::Parser;

    let cases = [
        vec!["xpm", "-Syu", "--noconfirm"],
        vec!["xpm", "-Sy"],
        vec!["xpm", "-Syy"],
        vec!["xpm", "-Syu", "--ignore", "linux"],
        vec!["xpm", "-S", "--needed", "--noconfirm", "firefox"],
        vec!["xpm", "-Sp", "firefox"],
        vec!["xpm", "-U", "/tmp/foo.pkg.tar.zst"],
        vec!["xpm", "-Rns", "pkg"],
        vec!["xpm", "-Rdd", "pkg"],
        vec!["xpm", "-R", "-ns", "pkg"],
        vec!["xpm", "-Rc", "pkg"],
        vec!["xpm", "-Ru", "pkg"],
        vec!["xpm", "-Qe"],
        vec!["xpm", "-Qd"],
        vec!["xpm", "-Qq"],
        vec!["xpm", "-Qo", "/usr/bin/foo"],
        vec!["xpm", "-Qm"],
        vec!["xpm", "-Qmq"],
        vec!["xpm", "-Qt"],
        vec!["xpm", "-Qdt"],
        vec!["xpm", "-Qg"],
        vec!["xpm", "-Qqg"],
        vec!["xpm", "-Qk"],
        vec!["xpm", "-Qkk"],
        vec!["xpm", "-Qkk", "hello"],
        vec!["xpm", "-D", "--asdeps", "hello"],
        vec!["xpm", "-D", "--asexplicit", "hello"],
        vec!["xpm", "-T", "glibc>=2.39"],
        vec!["xpm", "-T"],
        vec!["xpm", "-Qi", "pkg"],
        vec!["xpm", "-Ql", "pkg"],
        vec!["xpm", "-Ss", "query"],
        vec!["xpm", "-Si", "pkg"],
        vec!["xpm", "-S", "--asdeps", "--downloadonly", "foo"],
        vec!["xpm", "-Syu", "--color", "never"],
        vec!["xpm", "--root", "/mnt", "--dbpath", "/mnt/db", "-Syu"],
        vec!["xpm", "-S", "--", "-odd-name"],
        vec!["xpm", "-Sw", "firefox"],
        vec!["xpm", "-Suw"],
        vec!["xpm", "-Sl"],
        vec!["xpm", "-Slq", "core"],
        vec!["xpm", "-Sg"],
        vec!["xpm", "-Sgg"],
        vec!["xpm", "-Sgq", "desktop"],
        vec!["xpm", "-Sc"],
        vec!["xpm", "-Scc"],
        vec!["xpm", "-Qn"],
        vec!["xpm", "-Qnq"],
        vec!["xpm", "-Qnt"],
        vec!["xpm", "-Qc", "hello"],
        vec!["xpm", "-Qcq", "hello"],
        vec!["xpm", "-Qp", "/tmp/hello.pkg.tar.zst"],
        vec!["xpm", "-Qpq", "/tmp/hello.pkg.tar.zst"],
        vec!["xpm", "-F", "usr/bin/hello"],
        vec!["xpm", "-Fl", "hello"],
        vec!["xpm", "-Fq", "usr/bin/hello"],
        vec!["xpm", "-Fy"],
        vec!["xpm", "-Fyy"],
        vec!["xpm", "-S", "--ignoregroup", "kernel", "pkg"],
        vec!["xpm", "-Syu", "--ignoregroup", "kernel"],
        vec!["xpm", "-S", "--assume-installed", "linux=6.9", "pkg"],
        vec!["xpm", "--sysroot", "/mnt", "-Syu"],
    ];

    for input in cases {
        let rewritten = rewrite_args(input.iter().map(OsString::from));
        let display = rewritten
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        if let Err(err) = Cli::try_parse_from(rewritten) {
            panic!("{input:?} rewrote to `{display}`, which clap rejected: {err}");
        }
    }
}
