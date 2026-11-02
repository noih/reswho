use anyhow::{Context, Result, bail};
use std::{
    io::{ErrorKind, Read},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
pub(super) fn find_program(name: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct Output {
    pub stdout: Vec<u8>,
    pub issue: Option<String>,
}

pub(super) trait Runner {
    fn run(&mut self, args: &[String]) -> Result<Output>;

    fn direct_stats(
        &mut self,
        _host: &str,
        _ids: &[String],
    ) -> Result<Option<super::stats::Batch>> {
        Ok(None)
    }
}

pub(super) struct RuntimeCommand {
    pub cancel: crate::snapshot::Cancellation,
    pub deadline: Instant,
    pub program: std::path::PathBuf,
    pub sample_interval: Duration,
    pub api_version: Option<String>,
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
        bail!("cannot configure container CLI output pipe");
    }
    Ok(())
}

fn drain(pipe: &mut impl Read, buffer: &mut Vec<u8>) -> Result<bool> {
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

impl Runner for RuntimeCommand {
    fn direct_stats(&mut self, host: &str, ids: &[String]) -> Result<Option<super::stats::Batch>> {
        let Some(socket) = host
            .strip_prefix("unix://")
            .filter(|path| std::path::Path::new(path).is_absolute())
        else {
            return Ok(None);
        };
        let client = super::http::Client {
            socket: std::path::Path::new(socket),
            cancel: &self.cancel,
            deadline: self.deadline,
        };
        super::stats::collect(
            &client,
            ids,
            self.sample_interval,
            self.api_version.as_deref(),
        )
    }

    fn run(&mut self, args: &[String]) -> Result<Output> {
        self.cancel.check()?;
        if Instant::now() >= self.deadline {
            bail!("container CLI collection deadline exceeded");
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
                ErrorKind::NotFound => anyhow::anyhow!("container CLI not found on PATH"),
                _ => anyhow::anyhow!("cannot start container CLI ({})", error.kind()),
            })?,
            reaped: false,
        };
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
            self.cancel.check()?;
            let out_done = drain(&mut stdout, &mut output)?;
            let err_done = drain(&mut stderr, &mut errors)?;
            if out_done
                && err_done
                && let Some(status) = child.process.try_wait()?
            {
                child.reaped = true;
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
                });
            }
            if Instant::now() >= self.deadline {
                return Ok(Output {
                    stdout: output,
                    issue: Some("container CLI collection deadline exceeded".into()),
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
    fn cancellation_reaps_running_owned_command() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pid");
        let cancel = crate::snapshot::Cancellation::default();
        let mut runner = RuntimeCommand {
            cancel: cancel.clone(),
            program: "/bin/sh".into(),
            sample_interval: Duration::from_millis(250),
            api_version: None,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let argument = path.to_string_lossy().into_owned();
        let handle = std::thread::spawn(move || {
            runner.run(&[
                "-c".into(),
                "echo $$ > \"$1\"; exec /bin/sleep 30".into(),
                "fixture".into(),
                argument,
            ])
        });
        let start = Instant::now();
        let pid = loop {
            if let Ok(value) = std::fs::read_to_string(&path)
                && let Ok(pid) = value.trim().parse::<i32>()
            {
                break Some(pid);
            }
            if start.elapsed() >= Duration::from_secs(2) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        cancel.cancel();
        let result = handle.join().unwrap();
        assert!(result.err().unwrap().to_string().contains("canceled"));
        let pid = pid.expect("owned command started");
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        directory.close().unwrap();
    }

    #[test]
    fn timeout_reaps_owned_child_and_preserves_partial_output() {
        let mut runner = RuntimeCommand {
            cancel: crate::snapshot::Cancellation::default(),
            program: "/bin/sh".into(),
            sample_interval: Duration::from_millis(250),
            api_version: None,
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
        let mut runner = RuntimeCommand {
            cancel: crate::snapshot::Cancellation::default(),
            program: "/bin/sh".into(),
            sample_interval: Duration::from_millis(250),
            api_version: None,
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
