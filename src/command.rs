use miette::{IntoDiagnostic, Result, WrapErr};
use std::process::Stdio;
use tokio::process::Command;

#[cfg(target_os = "windows")]
pub(crate) fn new_command(cmd: &str) -> Command {
    let mut command = Command::new("cmd.exe");
    command.arg("/c");
    command.arg(cmd);
    command
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn new_command(cmd: &str) -> Command {
    Command::new(cmd)
}

/// Run a command without producing any output in STDOUT and STDERR
pub(crate) async fn silent_command(cmd: &str, args: &[&str]) -> Result<()> {
    let mut child = new_command(cmd)
        .args(args)
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to run `{} {}`", cmd, args.join(" ")))?;

    child
        .wait()
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to wait on {cmd} process"))
        .map(|_| ())
}
