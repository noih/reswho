use crate::model::{Diagnostic, Process, ProcessCommand};
use std::{io, mem::size_of, path::Path, ptr};

const MAX_BYTES: usize = 1_048_576 + size_of::<i32>();
const MAX_ARGS: usize = 4096;
const MAX_ARG_BYTES: usize = 16_384;
const FILTERED: &str = "[filtered]";

pub(super) fn unavailable(diagnostic: Diagnostic) -> ProcessCommand {
    ProcessCommand {
        unavailable: vec![diagnostic],
        ..ProcessCommand::default()
    }
}

pub(super) fn discard_if_changed(process: &mut Process, requested: Option<u32>, consistent: bool) {
    if requested == Some(process.pid) && !consistent {
        process.command = Some(unavailable(Diagnostic::new(
            "identity_changed",
            "command",
            "command: process identity could not be confirmed across observation",
        )));
    }
}

pub(super) fn collect(pid: i32, executable: Option<&Path>, pointer_width: usize) -> ProcessCommand {
    match read(pid).and_then(|buffer| parse(&buffer, pointer_width)) {
        Ok(argv) => sanitize(argv, executable),
        Err(error) => unavailable(error),
    }
}

fn read(pid: i32) -> Result<Vec<u8>, Diagnostic> {
    let mut argmax = 0i32;
    let mut size = size_of::<i32>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut i32).cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(Diagnostic::io("command.limit", &io::Error::last_os_error()));
    }
    if size != size_of::<i32>() || argmax <= 0 || argmax as usize > MAX_BYTES - size_of::<i32>() {
        return Err(problem(
            "limit_exceeded",
            "command: OS argument buffer exceeds collection limit",
        ));
    }
    let mut buffer = vec![0u8; argmax as usize + size_of::<i32>()];
    size = buffer.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(Diagnostic::io("command.read", &io::Error::last_os_error()));
    }
    if size >= buffer.len() {
        return Err(problem(
            "limit_exceeded",
            "command: argument buffer reached capacity; completeness unknown",
        ));
    }
    buffer.truncate(size);
    Ok(buffer)
}

fn problem(code: &'static str, message: &'static str) -> Diagnostic {
    Diagnostic::new(code, "command", message)
}

fn parse(buffer: &[u8], pointer_width: usize) -> Result<Vec<String>, Diagnostic> {
    let malformed = || {
        problem(
            "invalid_data",
            "command: incomplete or invalid argument data",
        )
    };
    if buffer.len() > MAX_BYTES {
        return Err(problem(
            "limit_exceeded",
            "command: argument buffer exceeds collection limit",
        ));
    }
    let argc = i32::from_ne_bytes(
        buffer
            .get(..4)
            .ok_or_else(malformed)?
            .try_into()
            .map_err(|_| malformed())?,
    );
    if argc < 0 || argc as usize > MAX_ARGS {
        return Err(problem(
            "limit_exceeded",
            "command: argument count exceeds collection limit",
        ));
    }
    let path_end = buffer
        .get(4..)
        .ok_or_else(malformed)?
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(malformed)?
        + 4;
    if path_end == 4 {
        return Err(malformed());
    }
    if !matches!(pointer_width, 4 | 8) {
        return Err(malformed());
    }
    let args_start =
        (path_end + 1 - size_of::<i32>()).next_multiple_of(pointer_width) + size_of::<i32>();
    let padding = buffer.get(path_end + 1..args_start).ok_or_else(malformed)?;
    if padding.iter().any(|byte| *byte != 0) {
        return Err(malformed());
    }
    let mut rest = buffer.get(args_start..).ok_or_else(malformed)?;
    let mut args = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(malformed)?;
        if end > MAX_ARG_BYTES {
            return Err(problem(
                "limit_exceeded",
                "command: argument exceeds collection limit",
            ));
        }
        let arg = std::str::from_utf8(&rest[..end]).map_err(|_| malformed())?;
        args.push(arg.to_owned());
        rest = &rest[end + 1..];
    }
    Ok(args)
}

use crate::redaction::{SENSITIVE_NAMES, normalized_option, sensitive_option};

fn named_option(value: &str) -> bool {
    let normalized = normalized_option(value);
    SENSITIVE_NAMES.contains(&normalized.as_str())
        || matches!(
            normalized.as_str(),
            "key"
                | "pass"
                | "user"
                | "env"
                | "eval"
                | "print"
                | "command"
                | "clientsecret"
                | "accesstoken"
                | "refreshtoken"
                | "credentials"
                | "cookies"
                | "headers"
        )
}

fn inline_code_option(value: &str) -> bool {
    matches!(
        value,
        "-c" | "-e" | "-p" | "--eval" | "--print" | "--command"
    ) || (value.starts_with('-')
        && !value.starts_with("--")
        && value[1..].chars().all(|c| "ilxec".contains(c))
        && value.contains(['c', 'e']))
}

pub(super) fn sanitize(mut argv: Vec<String>, executable: Option<&Path>) -> ProcessCommand {
    let python_flags_end = if executable
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .is_some_and(is_python)
    {
        1 + argv
            .iter()
            .skip(1)
            .take_while(|arg| python_flag(arg))
            .count()
    } else {
        0
    };
    let mut redacted_indices = Vec::new();
    let mut redact_next = false;
    for (index, arg) in argv.iter_mut().enumerate() {
        let option = arg.split_once('=').map_or(arg.as_str(), |(key, _)| key);
        let attached = ["-p", "-u", "-H", "-e", "-c"].into_iter().find(|flag| {
            arg.starts_with(flag)
                && arg.len() > flag.len()
                && !arg.starts_with("--")
                && !named_option(option)
        });
        let next = !redact_next
            && index > 0
            && index >= python_flags_end
            && arg.starts_with('-')
            && attached.is_none()
            && (sensitive_option(option) || inline_code_option(option))
            && !arg.contains('=');
        let opaque = arg.contains("://")
            || arg.starts_with(['{', '['])
            || arg.chars().any(char::is_control)
            || arg.to_ascii_lowercase().starts_with("bearer ");
        let replacement = if redact_next {
            Some(FILTERED.into())
        } else if let Some(flag) = attached {
            Some(format!("{flag}{FILTERED}"))
        } else if let Some((key, _)) = arg.split_once('=') {
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            {
                Some(format!("{key}={FILTERED}"))
            } else {
                Some(FILTERED.into())
            }
        } else if opaque {
            Some(FILTERED.into())
        } else {
            None
        };
        if let Some(replacement) = replacement {
            *arg = replacement;
            redacted_indices.push(index);
        }
        redact_next = next;
    }
    let entrypoint = entrypoint(&argv, &redacted_indices, executable);
    ProcessCommand {
        argv: Some(argv),
        redacted_indices,
        entrypoint,
        unavailable: Vec::new(),
    }
}

fn is_python(bin: &str) -> bool {
    bin.strip_prefix("python").is_some_and(|suffix| {
        suffix.is_empty() || suffix.chars().all(|c| c.is_ascii_digit() || c == '.')
    })
}

fn python_flag(arg: &str) -> bool {
    matches!(arg, "-u" | "-B" | "-E" | "-I" | "-s" | "-S")
}

fn entrypoint(argv: &[String], redacted: &[usize], executable: Option<&Path>) -> Option<String> {
    let bin = executable?.file_name()?.to_str()?;
    let python = is_python(bin);
    let index = if bin == "java" {
        (argv.get(1)?.as_str() == "-jar").then_some(2)?
    } else if python
        || matches!(
            bin,
            "node" | "nodejs" | "ruby" | "perl" | "sh" | "bash" | "zsh"
        )
    {
        let mut index = 1;
        if python {
            while argv.get(index).is_some_and(|arg| python_flag(arg)) {
                index += 1;
            }
        }
        if argv.get(index).is_some_and(|arg| arg == "--") {
            index += 1;
        }
        index
    } else {
        return None;
    };
    let value = argv.get(index)?;
    if value.is_empty() || value.starts_with('-') || redacted.contains(&index) {
        return None;
    }
    Some(value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(args: &[&[u8]], tail: &[u8]) -> Vec<u8> {
        let mut bytes = (args.len() as i32).to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"/fixture/bin/tool\0");
        bytes.resize((bytes.len() - 4).next_multiple_of(8) + 4, 0);
        for arg in args {
            bytes.extend_from_slice(arg);
            bytes.push(0);
        }
        bytes.extend_from_slice(tail);
        bytes
    }

    #[test]
    fn parses_only_argc_arguments_and_preserves_empty_and_spaced_values() {
        let raw = buffer(
            &[b"tool", b"", b"a b", b"last"],
            b"ENV_SECRET=never-output\0",
        );
        assert_eq!(parse(&raw, 8).unwrap(), ["tool", "", "a b", "last"]);
        assert!(
            parse(&buffer(&[], b"ENV_SECRET=never-output\0"), 8)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn respects_target_alignment_without_skipping_leading_empty_arguments() {
        for width in [4, 8] {
            for path_length in 1..=32 {
                let mut raw = 3i32.to_ne_bytes().to_vec();
                raw.extend(vec![b'x'; path_length]);
                raw.push(0);
                raw.resize((raw.len() - 4).next_multiple_of(width) + 4, 0);
                raw.extend_from_slice(b"\0\0tool\0ENV_SECRET=hidden\0");
                assert_eq!(parse(&raw, width).unwrap(), ["", "", "tool"]);
            }
        }
    }

    #[test]
    fn rejects_truncated_non_utf8_and_oversized_input() {
        for raw in [vec![], buffer(&[b"tool", &[255]], b"")] {
            assert!(parse(&raw, 8).is_err());
        }
        let mut truncated = buffer(&[b"tool"], b"");
        truncated.pop();
        assert!(parse(&truncated, 8).is_err());
        assert_eq!(
            parse(&vec![0; MAX_BYTES + 1], 8).unwrap_err().code,
            "limit_exceeded"
        );
        assert!(parse(&buffer(&[&vec![b'x'; MAX_ARG_BYTES + 1]], b""), 8).is_err());
        let mut many = buffer(&[b"tool"], b"");
        many[..4].copy_from_slice(&((MAX_ARGS + 1) as i32).to_ne_bytes());
        assert!(parse(&many, 8).is_err());
    }

    fn sanitized(bin: &str, args: &[&str]) -> ProcessCommand {
        sanitize(
            args.iter().map(|s| (*s).into()).collect(),
            Some(Path::new(bin)),
        )
    }

    #[test]
    fn redacts_sensitive_values_urls_assignments_headers_and_inline_code() {
        let command = sanitized(
            "/bin/tool",
            &[
                "tool",
                "--access-token",
                "secret-a",
                "--password=secret-b",
                "-psecret-c",
                "https://user:secret-d@host/",
                "CONFIG=secret-e",
                "-H",
                "Authorization: secret-f",
                "-lc",
                "echo secret-g",
                "{\"secret\":\"secret-h\"}",
                "--port",
                "8080",
            ],
        );
        let json = serde_json::to_string(&command).unwrap();
        for secret in [
            "secret-a", "secret-b", "secret-c", "secret-d", "secret-e", "secret-f", "secret-g",
            "secret-h",
        ] {
            assert!(!json.contains(secret));
        }
        assert_eq!(command.argv.as_ref().unwrap().last().unwrap(), "8080");
        assert_eq!(command.redacted_indices, [2, 3, 4, 5, 6, 8, 10, 11]);
    }

    #[test]
    fn filtering_preserves_option_names_and_argument_positions() {
        assert_eq!(
            sanitized("/bin/tool", &["tool", "-pfixture=value", "normal"])
                .argv
                .unwrap(),
            ["tool", "-p[filtered]", "normal"]
        );
        let command = sanitized(
            "/bin/tool",
            &[
                "tool",
                "-token",
                "fixture-value",
                "-password",
                "fixture-value",
                "-pwd=fixture-value",
                "--auth=https://fixture-value@example.test",
                "-pfixture-value",
                "--port",
                "8080",
            ],
        );
        assert_eq!(
            command.argv.unwrap(),
            [
                "tool",
                "-token",
                "[filtered]",
                "-password",
                "[filtered]",
                "-pwd=[filtered]",
                "--auth=[filtered]",
                "-p[filtered]",
                "--port",
                "8080",
            ]
        );
        assert_eq!(command.redacted_indices, [2, 4, 5, 6, 7]);
    }

    #[test]
    fn common_sensitive_option_aliases_redact_values_in_both_forms() {
        for flag in [
            "--token",
            "--TOKEN",
            "--pwd",
            "--DB_PWD",
            "--password",
            "--auth",
            "--Authorization",
            "--api-key",
            "--api_key",
            "--apiKey",
            "--access-key",
            "--clientSecret",
            "--passkey",
            "--PASS",
        ] {
            let separated = sanitized(
                "/bin/tool",
                &["tool", flag, "fixture-sensitive-value", "--port", "8080"],
            );
            assert_eq!(
                separated.argv.unwrap(),
                ["tool", flag, FILTERED, "--port", "8080"],
                "{flag}"
            );
            assert_eq!(separated.redacted_indices, [2]);
            let assignment = format!("{flag}=fixture-sensitive-value");
            let joined = sanitized("/bin/tool", &["tool", &assignment]);
            assert_eq!(
                joined.argv.unwrap()[1],
                format!("{flag}={FILTERED}"),
                "{flag}"
            );
            assert_eq!(joined.redacted_indices, [1]);
        }
    }

    #[test]
    fn entrypoint_is_only_a_supported_unredacted_launch_target() {
        assert_eq!(
            sanitized("/bin/python3", &["python3", "-u", "-B", "worker.py"])
                .entrypoint
                .as_deref(),
            Some("worker.py")
        );
        for (bin, args, expected) in [
            (
                "/opt/bin/python3.14",
                vec!["python", "worker job.py"],
                Some("worker job.py"),
            ),
            (
                "/bin/node",
                vec!["node", "--", "src/server.js"],
                Some("src/server.js"),
            ),
            (
                "/bin/java",
                vec!["java", "-jar", "/app/server.jar"],
                Some("/app/server.jar"),
            ),
            ("/bin/python3", vec!["python", "-m", "http.server"], None),
            (
                "/bin/node",
                vec!["node", "--require", "hook.js", "app.js"],
                None,
            ),
            ("/bin/python3", vec!["python", "-c", "secret-code"], None),
            ("/bin/other", vec!["python", "app.py"], None),
        ] {
            assert_eq!(sanitized(bin, &args).entrypoint.as_deref(), expected);
        }
    }

    #[test]
    fn identity_failure_discards_command_and_unrequested_processes_stay_empty() {
        let mut process = Process {
            pid: 7,
            command: Some(sanitized("/bin/node", &["node", "app.js"])),
            ..Process::default()
        };
        discard_if_changed(&mut process, Some(7), false);
        let command = process.command.unwrap();
        assert!(command.argv.is_none());
        assert!(command.entrypoint.is_none());
        assert_eq!(command.unavailable[0].code, "identity_changed");
        let mut unrequested = Process::default();
        discard_if_changed(&mut unrequested, Some(7), false);
        assert!(unrequested.command.is_none());
        let denied = unavailable(Diagnostic::io(
            "command.read",
            &io::Error::from_raw_os_error(libc::EPERM),
        ));
        assert!(denied.argv.is_none());
        assert_eq!(denied.unavailable[0].code, "permission_denied");
    }
}
