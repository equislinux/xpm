//! Per-phase progress bars.

use super::*;

// ── Phase progress ───────────────────────────────────────────────────────────

/// One bar per phase step, rendered with the same aligned label/bar layout as
/// the package progress lines.
///
/// This is the shared implementation used by `xpm-aur` for its own phases
/// (`resolve`, `checkout`, `state`, ...), so the helper never grows a second
/// progress style.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhaseProgress {
    label_width: usize,
    bar_width: usize,
    fill: BarFill,
}

impl PhaseProgress {
    /// Build a layout for the labels in one transaction.
    pub fn new<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        let longest = labels
            .into_iter()
            .map(|label| label.chars().count())
            .max()
            .unwrap_or(12);
        Self {
            label_width: longest.clamp(12, 32),
            bar_width: 20,
            fill: BarFill::Slash,
        }
    }

    /// Network setup phases (`sync`, `resolve`): cross fill.
    pub fn new_network<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        let mut progress = Self::new(labels);
        progress.fill = BarFill::Cross;
        progress
    }

    /// Build/install phases (`checkout`, `building`, `installing`): slash fill.
    pub fn new_build<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        Self::new(labels)
    }

    /// Render `label [////----]  50%  phase`.
    pub fn render(&self, label: &str, step: u64, total: u64, phase: &str) -> String {
        let percent = percentage(step, total);
        let label = align_label(label, self.label_width);
        format!(
            "{label} [{}] {percent:>3}%  {phase}",
            bar(percent, self.bar_width, self.fill)
        )
    }

    pub fn print(&self, label: &str, step: u64, total: u64, phase: &str) {
        eprintln!("{}", self.render(label, step, total, phase));
    }
}
