use std::io;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ledger_core::Command;
use ledger_core::docker::RunId;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use crate::log::{Log, Stream};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a command to completion. Production uses [`SystemRunner`], tests use a fake Docker.
pub trait CommandRunner {
    /// Besides returning the whole output, sends it to `log` line by line while it is produced.
    fn run(
        &self,
        command: &Command,
        log: Option<&Log>,
    ) -> impl Future<Output = io::Result<Output>> + Send;
}

impl<T: CommandRunner + Sync> CommandRunner for &T {
    fn run(
        &self,
        command: &Command,
        log: Option<&Log>,
    ) -> impl Future<Output = io::Result<Output>> + Send {
        (**self).run(command, log)
    }
}

impl<T: CommandRunner + Send + Sync> CommandRunner for std::sync::Arc<T> {
    fn run(
        &self,
        command: &Command,
        log: Option<&Log>,
    ) -> impl Future<Output = io::Result<Output>> + Send {
        (**self).run(command, log)
    }
}

/// Starts a real process. Dropping the future kills the process, and so does the command's
/// timeout.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    async fn run(&self, command: &Command, log: Option<&Log>) -> io::Result<Output> {
        let Some(limit) = command.timeout else {
            return execute(command, log).await;
        };
        match tokio::time::timeout(limit, execute(command, log)).await {
            Ok(result) => result,
            Err(_elapsed) => {
                let message = format!("timed out after {:?}", limit);
                if let Some(log) = log {
                    log.line(Stream::Stderr, &message);
                    log.finished(false);
                }
                Err(io::Error::new(io::ErrorKind::TimedOut, message))
            }
        }
    }
}

async fn execute(command: &Command, log: Option<&Log>) -> io::Result<Output> {
    if let Some(log) = log {
        log.started(command);
    }
    let spawned = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            if let Some(log) = log {
                log.line(Stream::Stderr, &error.to_string());
                log.finished(false);
            }
            return Err(error);
        }
    };

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (stdout, stderr) = tokio::join!(
        collect(stdout, Stream::Stdout, log),
        collect(stderr, Stream::Stderr, log)
    );
    let status = child.wait().await?;

    if let Some(log) = log {
        log.finished(status.success());
    }
    Ok(Output {
        success: status.success(),
        stdout: stdout?,
        stderr: stderr?,
    })
}

/// Reads a pipe to its end, sending each line to the log as soon as it is complete.
async fn collect(
    reader: impl AsyncRead + Unpin,
    stream: Stream,
    log: Option<&Log>,
) -> io::Result<String> {
    let mut reader = BufReader::new(reader);
    let mut all = String::new();
    let mut raw = Vec::new();
    while reader.read_until(b'\n', &mut raw).await? > 0 {
        let text = String::from_utf8_lossy(&raw);
        if let Some(log) = log {
            log.line(stream, text.trim_end_matches(['\r', '\n']));
        }
        all.push_str(&text);
        raw.clear();
    }
    Ok(all)
}

/// An id that no other run of this machine has, to tag the processes of one run.
pub fn new_run_id() -> RunId {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    RunId::new(format!("{:x}-{nanos:x}-{count:x}", std::process::id()))
        .expect("hex digits and dashes are valid")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::Duration;

    use ledger_core::Device;

    use super::*;
    use crate::log::{self, LogEvent, LogReceiver, Source};

    #[test]
    fn run_ids_are_unique() {
        let ids: HashSet<_> = (0..100).map(|_| new_run_id().as_str().to_owned()).collect();

        assert_eq!(ids.len(), 100);
    }

    fn drain(receiver: &mut LogReceiver) -> Vec<LogEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    fn lines(events: &[LogEvent], wanted: Stream) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| match event {
                LogEvent::Line { stream, text, .. } if *stream == wanted => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    const SOURCE: Source = Source::Container(Device::Stax);

    #[cfg(unix)]
    #[tokio::test]
    async fn a_real_process_is_streamed_line_by_line_and_collected() {
        let (sender, mut receiver) = log::channel();
        let log = Log::new(SOURCE, sender);
        let command = Command::new("sh").args(["-c", "printf 'a\\nb\\n'; printf 'e\\n' >&2"]);

        let output = SystemRunner.run(&command, Some(&log)).await.unwrap();

        assert!(output.success);
        assert_eq!(output.stdout, "a\nb\n");
        assert_eq!(output.stderr, "e\n");
        let events = drain(&mut receiver);
        assert!(matches!(&events[0], LogEvent::Started { source, .. } if *source == SOURCE));
        assert_eq!(lines(&events, Stream::Stdout), ["a", "b"]);
        assert_eq!(lines(&events, Stream::Stderr), ["e"]);
        assert_eq!(
            events.last(),
            Some(&LogEvent::Finished {
                source: SOURCE,
                success: true
            })
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_process_is_reported_as_failed() {
        let (sender, mut receiver) = log::channel();
        let log = Log::new(SOURCE, sender);
        let command = Command::new("sh").args(["-c", "echo oops >&2; exit 3"]);

        let output = SystemRunner.run(&command, Some(&log)).await.unwrap();

        assert!(!output.success);
        let events = drain(&mut receiver);
        assert_eq!(lines(&events, Stream::Stderr), ["oops"]);
        assert_eq!(
            events.last(),
            Some(&LogEvent::Finished {
                source: SOURCE,
                success: false
            })
        );
    }

    #[tokio::test]
    async fn a_program_that_cannot_start_is_reported_in_the_log_and_as_an_error() {
        let (sender, mut receiver) = log::channel();
        let log = Log::new(SOURCE, sender);
        let command = Command::new("ledger-no-such-program");

        let result = SystemRunner.run(&command, Some(&log)).await;

        assert!(result.is_err());
        let events = drain(&mut receiver);
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], LogEvent::Started { .. }));
        assert!(matches!(
            &events[1],
            LogEvent::Line {
                stream: Stream::Stderr,
                ..
            }
        ));
        assert!(matches!(
            &events[2],
            LogEvent::Finished { success: false, .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_that_takes_too_long_is_given_up_on() {
        let (sender, mut receiver) = log::channel();
        let log = Log::new(SOURCE, sender);
        let command = Command::new("sleep")
            .arg("30")
            .timeout(Duration::from_millis(100));

        let started = std::time::Instant::now();
        let error = SystemRunner.run(&command, Some(&log)).await.unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
        let events = drain(&mut receiver);
        assert_eq!(lines(&events, Stream::Stderr).len(), 1);
        assert!(matches!(
            events.last(),
            Some(LogEvent::Finished { success: false, .. })
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nothing_is_sent_without_a_log() {
        let command = Command::new("sh").args(["-c", "echo quiet"]);

        let output = SystemRunner.run(&command, None).await.unwrap();

        assert_eq!(output.stdout, "quiet\n");
    }
}
