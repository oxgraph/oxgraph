//! Build script for Python extension-module link arguments.

/// Adds platform-specific extension-module link arguments for `PyO3`.
fn main() {
    pyo3_build_config::add_extension_module_link_args();
}
