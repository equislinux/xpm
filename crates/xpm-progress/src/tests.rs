use super::*;

#[test]
fn percentage_reports_ratio_and_clamps() {
    assert_eq!(percentage(0, 100), 0);
    assert_eq!(percentage(52, 100), 52);
    assert_eq!(percentage(100, 100), 100);
    assert_eq!(percentage(150, 100), 100);
}

#[test]
fn percentage_handles_zero_total() {
    assert_eq!(percentage(0, 0), 0);
    assert_eq!(percentage(42, 0), 0);
}

#[test]
fn bar_fills_proportionally_and_clamps() {
    assert_eq!(bar(0, 20, BarFill::Slash), "--------------------");
    assert_eq!(bar(52, 20, BarFill::Slash), "//////////----------");
    assert_eq!(bar(100, 20, BarFill::Slash), "////////////////////");
    assert_eq!(bar(150, 20, BarFill::Slash), "////////////////////");
    assert_eq!(bar(0, 0, BarFill::Slash), "");
}

#[test]
fn mib_rendering_uses_one_decimal() {
    assert_eq!(format_mib(0), "0.0 MiB");
    assert_eq!(format_mib(1024 * 1024), "1.0 MiB");
    assert_eq!(format_mib(24_641_536), "23.5 MiB");
}

#[test]
fn download_bars_use_cross_fill() {
    assert_eq!(bar(0, 20, BarFill::Cross), "--------------------");
    assert_eq!(bar(52, 20, BarFill::Cross), "xxxxxxxxxx----------");
    assert_eq!(bar(100, 20, BarFill::Cross), "xxxxxxxxxxxxxxxxxxxx");
    assert!(!bar(50, 20, BarFill::Cross).contains('#'));
    let line = ProgressStyle {
        label_width: 11,
        bar_width: 20,
        fill: BarFill::Cross,
    }
    .render("pkg.tar.zst", 13 * 1024 * 1024, 26 * 1024 * 1024);
    assert_eq!(
        line,
        "pkg.tar.zst [xxxxxxxxxx----------]  50%  13.0/26.0 MiB"
    );
}

#[test]
fn operation_bars_use_slash_fill() {
    assert_eq!(bar(52, 20, BarFill::Slash), "//////////----------");
    assert!(!bar(50, 20, BarFill::Slash).contains('x'));
    let progress = OperationProgress::new("Verifying signatures", 2, BarFill::Slash);
    let line = progress.line().expect("operation line");
    assert!(line.contains("[--------------------]"));
    progress.advance();
    progress.advance();
    let line = progress.line().expect("operation line");
    assert!(line.contains("[////////////////////]"));
}

#[test]
fn phase_progress_selects_fill_by_phase_type() {
    let network = PhaseProgress::new_network(["sync"]);
    assert!(network.render("sync", 1, 2, "databases").contains('x'));
    let build = PhaseProgress::new_build(["pkg"]);
    assert!(build.render("pkg", 1, 2, "building").contains('/'));
}

#[test]
fn parallel_progress_aggregates_bytes_and_completed_jobs() {
    let progress = ParallelDownloadProgress::new(2);
    progress.add_total(300);
    progress.add_downloaded(100);
    progress.complete_job();
    assert!(!progress.is_done());
    progress.add_downloaded(200);
    progress.complete_job();
    assert!(progress.is_done());
}

#[test]
fn progress_line_handles_zero_total() {
    let line = ProgressStyle {
        label_width: 11,
        bar_width: 20,
        fill: BarFill::Cross,
    }
    .render("empty", 0, 0);
    assert_eq!(line, "empty       [--------------------]   0%  0.0/0.0 MiB");
}

#[test]
fn aligned_labels_put_bars_in_the_same_column() {
    let style = ProgressStyle {
        label_width: 16,
        bar_width: 12,
        fill: BarFill::Cross,
    };
    let short = style.render("vim", 1, 2);
    let long = style.render("linux-firmware", 1, 2);
    assert_eq!(short.find('['), long.find('['));
    assert_eq!(short.find('['), Some(17));
}

#[test]
fn long_labels_are_truncated_to_the_column() {
    let style = ProgressStyle {
        label_width: 10,
        bar_width: 10,
        fill: BarFill::Cross,
    };
    let line = style.render("a-very-long-package-name", 1, 2);
    assert!(line.starts_with("a-very-l.. ["));
    assert_eq!(line.find('['), Some(11));
}

#[test]
fn style_respects_terminal_width_floor() {
    let style = ProgressStyle::for_download();
    assert!(style.label_width >= 12);
    assert!(style.bar_width >= 10);
}

#[test]
fn phase_bars_share_one_column() {
    let progress = PhaseProgress::new(["foo", "a-very-long-package"]);
    let short = progress.render("foo", 1, 3, "building");
    let long = progress.render("a-very-long-package", 1, 3, "building");
    assert_eq!(short.find('['), long.find('['));
}

#[test]
fn phase_long_labels_are_truncated() {
    let progress = PhaseProgress::new(["x"]);
    let line = progress.render("a-very-long-package-name", 1, 3, "building");
    assert!(line.starts_with("a-very-lon.."));
    assert_eq!(line.find('['), Some(13));
}

#[test]
fn phase_zero_total_is_zero_percent() {
    let progress = PhaseProgress::new(["x"]);
    let line = progress.render("pkg", 3, 0, "idle");
    assert!(line.contains("  0%"));
}
