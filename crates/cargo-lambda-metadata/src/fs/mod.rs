#[cfg(not(target_os = "linux"))]
pub use std::fs::rename;

#[cfg(target_os = "linux")]
mod rename_linux;
#[cfg(target_os = "linux")]
pub use rename_linux::*;
