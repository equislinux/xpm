//! `-T` dependency-test translation.

use super::*;

pub(super) fn build_deptest_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
    if !modifiers.is_empty()
        || has_install_flags(flags)
        || has_upgrade_flags(flags)
        || has_remove_flags(flags)
    {
        return None;
    }

    Some(vec![OsString::from("deptest")])
}
