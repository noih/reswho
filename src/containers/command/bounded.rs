use crate::model::FailureReason;
use anyhow::{Context, Result, bail};
use std::{
    io::{ErrorKind, Read, Write},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(super) const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

pub struct Output {
    pub stdout: Vec<u8>,
    pub issue: Option<String>,
    pub timed_out: bool,
    pub exit_code: Option<i32>,
}

impl Output {
    pub fn failure(&self, issue: impl Into<String>) -> anyhow::Error {
        Failure {
            reason: if self.timed_out {
                FailureReason::Timeout
            } else {
                FailureReason::Unknown
            },
            message: issue.into(),
        }
        .into()
    }
}

#[derive(Debug)]
pub struct Failure {
    pub reason: FailureReason,
    pub message: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

pub fn fail(reason: FailureReason, message: impl Into<String>) -> anyhow::Error {
    Failure {
        reason,
        message: message.into(),
    }
    .into()
}

pub fn reason_of(error: &anyhow::Error) -> FailureReason {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<Failure>())
        .map_or(FailureReason::Unknown, |failure| failure.reason)
}

struct OwnedChild {
    process: Child,
    group_cleaned: bool,
    lifeline: Option<std::os::unix::net::UnixStream>,
}

impl OwnedChild {
    fn exited(&self) -> Result<bool> {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.process.id() as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(error.into());
        }
        let info = unsafe { info.assume_init() };
        #[cfg(target_os = "linux")]
        let pid = unsafe { info.si_pid() };
        #[cfg(not(target_os = "linux"))]
        let pid = info.si_pid;
        Ok(pid != 0)
    }
    fn clean_group(&mut self) {
        if !self.group_cleaned {
            if self.lifeline.is_some() {
                self.lifeline.take();
            } else {
                unsafe { libc::kill(-(self.process.id() as i32), libc::SIGKILL) };
            }
            self.group_cleaned = true;
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.clean_group();
        let _ = self.process.wait();
    }
}

fn nonblocking(pipe: &impl AsRawFd) -> Result<()> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        bail!("cannot configure container CLI output pipe");
    }
    Ok(())
}

pub(super) fn drain(pipe: &mut impl Read, buffer: &mut Vec<u8>) -> Result<bool> {
    let mut chunk = [0; 65536];
    match pipe.read(&mut chunk) {
        Ok(0) => Ok(true),
        Ok(length) => {
            if buffer.len() + length > MAX_OUTPUT_BYTES {
                bail!("container CLI output exceeds 8 MiB limit");
            }
            buffer.extend_from_slice(&chunk[..length]);
            Ok(false)
        }
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
            Ok(false)
        }
        Err(_) => bail!("cannot read container CLI output"),
    }
}

pub fn run_bounded(
    program: &std::path::Path,
    args: &[String],
    input: Option<&[u8]>,
    cancel: &crate::snapshot::Cancellation,
    deadline: Instant,
) -> Result<Output> {
    run_observed(program, args, input, cancel, deadline, |_| Ok(()))
}

pub(super) fn run_observed(
    program: &std::path::Path,
    args: &[String],
    input: Option<&[u8]>,
    cancel: &crate::snapshot::Cancellation,
    deadline: Instant,
    started: impl FnOnce(u32) -> Result<()>,
) -> Result<Output> {
    if input.is_some_and(|input| input.len() > MAX_OUTPUT_BYTES) {
        bail!("provider request exceeds 8 MiB limit");
    }
    cancel.check()?;
    if Instant::now() >= deadline {
        return Err(fail(
            FailureReason::Timeout,
            "container CLI collection deadline exceeded",
        ));
    }
    if program.components().nth(1).is_some() && !program.exists() {
        return Err(fail(
            FailureReason::CliNotFound,
            "container CLI not found on PATH",
        ));
    }
    let lifeline = if input.is_some() {
        Some(std::os::unix::net::UnixStream::pair()?)
    } else {
        None
    };
    let guard_fd = lifeline.as_ref().map(|(_, reader)| reader.as_raw_fd());
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env("SSH_ASKPASS", "/usr/bin/false");
    unsafe {
        command.pre_exec(move || {
            #[cfg(target_os = "macos")]
            if let Some(fd) = guard_fd {
                unsafe extern "C" {
                    fn reswho_guard(lifeline: libc::c_int) -> libc::c_int;
                }
                if reswho_guard(fd) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                return Ok(());
            }
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = OwnedChild {
        process: command.spawn().map_err(|error| match error.kind() {
            ErrorKind::NotFound => fail(
                FailureReason::CliNotFound,
                "container CLI not found on PATH",
            ),
            _ => anyhow::anyhow!("cannot start container CLI ({})", error.kind()),
        })?,
        group_cleaned: false,
        lifeline: lifeline.map(|(writer, _)| writer),
    };
    started(child.process.id())?;
    let mut stdin = child.process.stdin.take();
    if let Some(pipe) = &stdin {
        nonblocking(pipe)?;
    }
    let input = input.unwrap_or(&[]);
    let mut sent = 0;
    let mut stdout = child
        .process
        .stdout
        .take()
        .context("container CLI stdout unavailable")?;
    let mut stderr = child
        .process
        .stderr
        .take()
        .context("container CLI stderr unavailable")?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    loop {
        cancel.check()?;
        if let Some(pipe) = &mut stdin {
            match pipe.write(&input[sent..input.len().min(sent + 65536)]) {
                Ok(length) => sent += length,
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                Err(_) => bail!("cannot write provider request"),
            }
            if sent == input.len() {
                stdin.take();
            }
        }
        let out_done = drain(&mut stdout, &mut output)?;
        let err_done = drain(&mut stderr, &mut errors)?;
        if out_done && err_done && child.exited()? {
            child.clean_group();
            let status = child.process.wait()?;
            let issue = if !status.success() {
                Some(format!(
                    "container CLI command failed ({status}); stderr withheld"
                ))
            } else if !errors.is_empty() {
                Some("container CLI command reported diagnostics; stderr withheld".into())
            } else {
                None
            };
            return Ok(Output {
                stdout: output,
                issue,
                timed_out: false,
                exit_code: status.code(),
            });
        }
        if Instant::now() >= deadline {
            return Ok(Output {
                stdout: output,
                issue: Some("container CLI collection deadline exceeded".into()),
                timed_out: true,
                exit_code: None,
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
