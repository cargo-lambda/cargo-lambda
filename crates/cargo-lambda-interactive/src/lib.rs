use inquire::{self, error};
use std::fmt::Display;

pub mod command;
pub mod progress;

/// Check if STDIN is a TTY
pub fn is_stdin_tty() -> bool {
    atty::is(atty::Stream::Stdin)
}

/// Check if STDOUT is a TTY
pub fn is_stdout_tty() -> bool {
    atty::is(atty::Stream::Stdout)
}

pub fn choose_option<T: Display>(message: &str, options: Vec<T>) -> error::InquireResult<T> {
    inquire::Select::new(message, options)
        .with_vim_mode(true)
        .with_help_message("Press Ctrl+C to abort and exit cargo-lambda")
        .prompt()
}
