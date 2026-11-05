use super::command;
use crate::snapshot::Cancellation;
use std::{path::PathBuf, time::Instant};

pub struct PodmanCommand<'a> {
    pub program: PathBuf,
    pub cancel: &'a Cancellation,
    pub deadline: Instant,
}
impl PodmanCommand<'_> {
    pub fn run(&mut self, args: &[String]) -> anyhow::Result<command::Output> {
        command::run_bounded(&self.program, args, Some(b""), self.cancel, self.deadline)
    }
}
