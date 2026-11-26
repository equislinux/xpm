//! Package metadata inspection hook.

use super::*;

/// Load package metadata for inspection during hooks.
///
/// **Note:** Requires full package::reader integration.
pub struct MetadataLoadHook;

impl Hook for MetadataLoadHook {
    fn name(&self) -> &str {
        "metadata-load"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        if let Some(pkg_file) = &context.pkg_file {
            if !pkg_file.exists() {
                return Err(XpmError::Package(format!(
                    "package file not found: {}",
                    pkg_file.display()
                )));
            }
            // The install loop already parsed the archive; only read it here
            // for callers that did not provide cached metadata.
            if context.metadata.is_none() {
                crate::package::read_metadata(pkg_file).map_err(|e| {
                    XpmError::Package(format!(
                        "invalid package metadata in {}: {e}",
                        pkg_file.display()
                    ))
                })?;
            }
        }
        Ok(())
    }
}
