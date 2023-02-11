use super::Compiler;
use crate::TargetArch;
use cargo_options::Build;
use miette::Result;
use std::{collections::VecDeque, ffi::OsStr, process::Command};

pub(crate) struct Cross;

#[async_trait::async_trait]
impl Compiler for Cross {
    async fn command(&self, cargo: &Build, _target_arch: &TargetArch) -> Result<Command> {
        tracing::debug!("compiling with Cross");

        let cmd = cargo.command();
        let args = cmd.get_args().collect::<VecDeque<&OsStr>>();

        let mut cmd = Command::new("cross");
        cmd.args(args);

        Ok(cmd)
    }
}
