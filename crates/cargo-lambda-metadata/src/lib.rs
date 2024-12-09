pub mod cargo;
pub mod config;
pub mod env;
pub mod error;
pub mod fs;
pub mod lambda;

/// Name for the function when no name is provided.
/// This will make the watch command to compile
/// the binary without the `--bin` option, and will
/// assume that the package only has one function,
/// which is the main binary for that package.
pub const DEFAULT_PACKAGE_FUNCTION: &str = "_";

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    pub fn fixture_metadata(name: &str) -> PathBuf {
        format!("../../tests/fixtures/{name}/Cargo.toml").into()
    }
}
