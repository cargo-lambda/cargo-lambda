use inquire::{self, error::InquireResult};
use is_terminal::IsTerminal;
use std::fmt::Display;

pub mod command;
pub mod progress;

/// Check if STDIN is a TTY
pub fn is_stdin_tty() -> bool {
    std::io::stdin().is_terminal()
}

/// Check if STDOUT is a TTY
pub fn is_stdout_tty() -> bool {
    std::io::stdout().is_terminal()
}

pub fn choose_option<T: Display>(message: &str, options: Vec<T>) -> InquireResult<T> {
    inquire::Select::new(message, options)
        .with_vim_mode(true)
        .with_help_message("↑↓ to move, press Ctrl+C to abort and exit")
        .prompt()
}

pub fn is_user_cancellation_error(err: &InquireError) -> bool {
    matches!(
        err,
        InquireError::OperationCanceled | InquireError::OperationInterrupted
    )
}

pub use inquire::*;
