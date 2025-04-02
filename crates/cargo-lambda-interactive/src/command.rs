use miette::{Diagnostic, Result};
use std::process::Stdio;
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};

#[cfg(target_os = "windows")]
pub fn new_command(cmd: &str) -> Command {
    let mut command = Command::new("cmd.exe");
    command.arg("/c");
    command.arg(cmd);
    command
}

#[cfg(not(target_os = "windows"))]
pub fn new_command(cmd: &str) -> Command {
    Command::new(cmd)
}

/// Run a command without producing any output in STDOUT and STDERR
pub async fn silent_command(cmd: &str, args: &[&str]) -> Result<(), CommandError> {
    let child = new_command(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return Err(capture_error(cmd, args, None, Some(child.err().unwrap())).await);
    };

    let result = child.wait().await;
    let Ok(result) = result else {
        return Err(capture_error(cmd, args, Some(&mut child), None).await);
    };

    tracing::trace!(%result);

    if result.success() {
        Ok(())
    } else {
        Err(capture_error(cmd, args, Some(&mut child), None).await)
    }
}

async fn capture_error(
    cmd: &str,
    args: &[&str],
    child: Option<&mut Child>,
    error: Option<std::io::Error>,
) -> CommandError {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    if let Some(child) = child {
        let mut reader = child.stdout.take().expect("stdout is not captured");
        reader
            .read_to_end(&mut stdout)
            .await
            .expect("Failed to read stdout");

        let mut reader = child.stderr.take().expect("stderr is not captured");
        reader
            .read_to_end(&mut stderr)
            .await
            .expect("Failed to read stderr");
    }

    CommandError {
        command: format!("{} {}", cmd, args.join(" ")),
        stdout,
        stderr,
        error,
    }
}

#[derive(Debug, Default, Diagnostic)]
#[diagnostic(code(command_error))]
pub struct CommandError {
    command: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    error: Option<std::io::Error>,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Command `{}` failed", self.command)?;

        if let Some(error) = &self.error {
            write!(f, ": {}", error)?;
        }

        if !self.stdout.is_empty() {
            write!(f, "\n{}", String::from_utf8_lossy(&self.stdout))?;
        }

        if !self.stderr.is_empty() {
            write!(f, "\n{}", String::from_utf8_lossy(&self.stderr))?;
        }

        Ok(())
    }
}

impl std::error::Error for CommandError {}
