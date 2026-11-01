use anyhow::{Context, Result, bail};
use std::{
    io::{ErrorKind, Read},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct Output {
    pub stdout: Vec<u8>,
    pub issue: Option<String>,
}

pub(super) trait Runner {
    fn run(&mut self, args: &[String]) -> Result<Output>;
}

pub(super) struct DockerCommand {
    pub deadline: Instant,
    pub program: std::path::PathBuf,
}

struct OwnedChild {
    process: Child,
    reaped: bool,
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            unsafe { libc::kill(-(self.process.id() as i32), libc::SIGKILL) };
            let _ = self.process.wait();
        }
    }
}

fn nonblocking(pipe: &impl AsRawFd) -> Result<()> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        bail!("cannot configure Docker output pipe");
    }
    Ok(())
}

fn drain(pipe: &mut impl Read, buffer: &mut Vec<u8>) -> Result<bool> {
    let mut chunk = [0; 65536];
    match pipe.read(&mut chunk) {
        Ok(0) => Ok(true),
        Ok(length) => {
            if buffer.len() + length > MAX_OUTPUT_BYTES {
                bail!("Docker output exceeds 8 MiB limit");
            }
            buffer.extend_from_slice(&chunk[..length]);
            Ok(false)
        }
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
            Ok(false)
        }
        Err(_) => bail!("cannot read Docker output"),
    }
}

impl Runner for DockerCommand {
    fn run(&mut self, args: &[String]) -> Result<Output> {
        if Instant::now() >= self.deadline {
            bail!("Docker collection deadline exceeded");
        }
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("SSH_ASKPASS_REQUIRE", "never")
            .env("SSH_ASKPASS", "/usr/bin/false");
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = OwnedChild {
            process: command.spawn().map_err(|error| match error.kind() {
                ErrorKind::NotFound => anyhow::anyhow!("Docker CLI not found on PATH"),
                _ => anyhow::anyhow!("cannot start Docker CLI ({})", error.kind()),
            })?,
            reaped: false,
        };
        let mut stdout = child
            .process
            .stdout
            .take()
            .context("Docker stdout unavailable")?;
        let mut stderr = child
            .process
            .stderr
            .take()
            .context("Docker stderr unavailable")?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let mut output = Vec::new();
        let mut errors = Vec::new();
        loop {
            let out_done = drain(&mut stdout, &mut output)?;
            let err_done = drain(&mut stderr, &mut errors)?;
            if out_done
                && err_done
                && let Some(status) = child.process.try_wait()?
            {
                child.reaped = true;
                let issue = if !status.success() {
                    Some(format!("Docker command failed ({status}); stderr withheld"))
                } else if !errors.is_empty() {
                    Some("Docker command reported diagnostics; stderr withheld".into())
                } else {
                    None
                };
                return Ok(Output {
                    stdout: output,
                    issue,
                });
            }
            if Instant::now() >= self.deadline {
                return Ok(Output {
                    stdout: output,
                    issue: Some("Docker collection deadline exceeded".into()),
                });
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_reaps_owned_child_and_preserves_partial_output() {
        let mut runner = DockerCommand {
            program: "/bin/sh".into(),
            deadline: Instant::now() + Duration::from_millis(200),
        };
        let start = Instant::now();
        let result = runner
            .run(&["-c".into(), "echo $$; exec /bin/sleep 30".into()])
            .unwrap();
        assert!(result.issue.unwrap().contains("deadline"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let pid: i32 = String::from_utf8(result.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[test]
    fn bounded_reader_and_failed_command_do_not_expose_stderr() {
        let mut output = vec![0; MAX_OUTPUT_BYTES];
        assert!(drain(&mut std::io::Cursor::new(b"extra"), &mut output).is_err());
        let mut runner = DockerCommand {
            program: "/bin/sh".into(),
            deadline: Instant::now() + Duration::from_secs(2),
        };
        let result = runner
            .run(&["-c".into(), "echo private-credential >&2; exit 7".into()])
            .unwrap();
        let issue = result.issue.unwrap();
        assert!(issue.contains('7'));
        assert!(!issue.contains("private-credential"));
        assert!(result.stdout.is_empty());
    }
}
