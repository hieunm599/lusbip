use std::io;
use std::process::{ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const USBIP_COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output>;
    fn run_interactive(&self, program: &str, args: &[&str]) -> std::io::Result<ExitStatus>;
    fn run_foreground(&self, program: &str, args: &[&str]) -> std::io::Result<ExitStatus>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StdCommandRunner;

impl CommandRunner for StdCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let started = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                return child.wait_with_output();
            }
            if started.elapsed() >= USBIP_COMMAND_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "{program} timed out after {} seconds",
                        USBIP_COMMAND_TIMEOUT.as_secs()
                    ),
                ));
            }
            thread::sleep(CHILD_POLL_INTERVAL);
        }
    }

    fn run_interactive(&self, program: &str, args: &[&str]) -> std::io::Result<ExitStatus> {
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            if started.elapsed() >= USBIP_COMMAND_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "{program} timed out after {} seconds",
                        USBIP_COMMAND_TIMEOUT.as_secs()
                    ),
                ));
            }
            thread::sleep(CHILD_POLL_INTERVAL);
        }
    }

    fn run_foreground(&self, program: &str, args: &[&str]) -> std::io::Result<ExitStatus> {
        std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
    }
}
