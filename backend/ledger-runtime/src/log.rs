//! The output of the commands, as separate streams: one per source, so that a terminal can show
//! the containers of an app side by side.

use ledger_core::{Command, Device};
use tokio::sync::mpsc;

/// Where output comes from. Each source is its own stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    /// The image, shared by the containers of an app.
    Image,
    /// The container of a device, and what runs on the host for that device.
    Container(Device),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    Started {
        source: Source,
        command: String,
    },
    Line {
        source: Source,
        stream: Stream,
        text: String,
    },
    Finished {
        source: Source,
        success: bool,
    },
    /// Something the runtime itself says about a source, such as an update put off.
    Note {
        source: Source,
        text: String,
    },
}

pub type LogSender = mpsc::UnboundedSender<LogEvent>;
pub type LogReceiver = mpsc::UnboundedReceiver<LogEvent>;

pub fn channel() -> (LogSender, LogReceiver) {
    mpsc::unbounded_channel()
}

/// Sends the output of the commands of one source to the log, line by line, as it is produced.
#[derive(Debug, Clone)]
pub struct Log {
    source: Source,
    sender: LogSender,
}

impl Log {
    pub fn new(source: Source, sender: LogSender) -> Self {
        Self { source, sender }
    }

    pub fn started(&self, command: &Command) {
        self.send(LogEvent::Started {
            source: self.source.clone(),
            command: command.to_string(),
        });
    }

    pub fn line(&self, stream: Stream, text: &str) {
        self.send(LogEvent::Line {
            source: self.source.clone(),
            stream,
            text: text.to_owned(),
        });
    }

    pub fn finished(&self, success: bool) {
        self.send(LogEvent::Finished {
            source: self.source.clone(),
            success,
        });
    }

    pub fn note(&self, text: &str) {
        self.send(LogEvent::Note {
            source: self.source.clone(),
            text: text.to_owned(),
        });
    }

    /// Nobody listening is not an error: the output is then simply not shown.
    fn send(&self, event: LogEvent) {
        let _ = self.sender.send(event);
    }
}
