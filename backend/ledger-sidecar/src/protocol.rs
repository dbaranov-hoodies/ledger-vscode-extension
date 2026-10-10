//! The messages between the extension and the sidecar. JSON-RPC 2.0 with one JSON value per
//! line. The extension sends requests; the sidecar answers them and sends notifications.

use std::collections::BTreeMap;

use ledger_core::actions::ActionId;
use ledger_core::platform::Platform;
use ledger_runtime::{LogEvent, Source, Stream};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const INITIALIZE: &str = "initialize";
pub const REFRESH: &str = "refresh";
pub const UPDATE_SETTINGS: &str = "updateSettings";
pub const SELECT_APP: &str = "selectApp";
pub const SET_DEVICES: &str = "setDevices";
pub const SET_BUILD_USE_CASE: &str = "setBuildUseCase";
pub const SET_VARIANT: &str = "setVariant";
pub const SET_ENFORCER_CHECK: &str = "setEnforcerCheck";
pub const COLLECT_TESTS: &str = "collectTests";
pub const TOGGLE_TEST: &str = "toggleTest";
pub const SET_TESTS: &str = "setTests";
pub const SELECT_FAILED_TESTS: &str = "selectFailedTests";
pub const LOCATE_TEST: &str = "locateTest";
pub const SET_TEST_SUITE: &str = "setTestSuite";
pub const SET_AI: &str = "setAi";
pub const RUN_ACTION: &str = "runAction";
pub const CANCEL_ACTION: &str = "cancelAction";

pub const SNAPSHOT: &str = "snapshot";
pub const SAVED: &str = "saved";
pub const LOG: &str = "log";
pub const ACTION_FINISHED: &str = "actionFinished";
pub const MESSAGE: &str = "message";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum PlatformName {
    Linux,
    MacOs,
    Windows,
}

impl PlatformName {
    pub fn platform(self) -> Platform {
        match self {
            Self::Linux => Platform::Linux,
            Self::MacOs => Platform::MacOs,
            Self::Windows => Platform::Windows,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct HostIds {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InitializeParams {
    pub platform: PlatformName,
    pub host_ids: Option<HostIds>,
    /// The `DISPLAY` of the host.
    pub display: Option<String>,
    pub settings: SettingsInput,
    /// What the sidecar sent last as `saved`.
    #[ts(type = "unknown")]
    pub saved: serde_json::Value,
    pub folders: Vec<FolderInput>,
}

/// The settings of the extension that the sidecar depends on.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingsInput {
    pub docker_image: String,
    /// Extra arguments for `docker run`, as one string.
    pub docker_run_args: String,
    pub auto_update: bool,
    pub max_retries: u32,
    pub open_as_root: bool,
    pub onboarding_pin: String,
    pub onboarding_seed: String,
    /// The key of the host to open the secure channel with, when the user wants it.
    pub scp_private_key: Option<String>,
    #[serde(rename = "allowNanoX")]
    #[ts(rename = "allowNanoX")]
    pub allow_nano_x: bool,
    /// The display name of a device, as in the settings: `Nano S Plus`.
    pub default_device: Option<String>,
    pub verbose_tests: bool,
    /// Commands that install what the tests of an app need, by the name of its folder.
    pub test_requirements: BTreeMap<String, String>,
}

/// A workspace folder with the files detection reads (see `FolderFiles::NAMES`).
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderInput {
    /// The path on the host.
    pub path: String,
    /// The path of the URI of the folder, which Docker mounts.
    pub uri_path: String,
    /// The Python environment of this project on the host.
    pub host_venv: String,
    /// The `appSettings` that the former extension saved for this folder, if any.
    #[serde(default)]
    #[ts(type = "unknown")]
    pub legacy_settings: serde_json::Value,
    pub files: Vec<FileInput>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct FileInput {
    /// Relative to the folder.
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RefreshParams {
    /// The folders again, when their files changed. Without them, only Docker is looked at.
    pub folders: Option<Vec<FolderInput>>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SelectAppParams {
    pub folder: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SetDevicesParams {
    /// Device ids, such as `nanosp`.
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SetBuildUseCaseParams {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SetVariantParams {
    pub value: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SetEnforcerCheckParams {
    /// `All` or the name of a check.
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct ToggleTestParams {
    /// The pytest id of a directory, file, class or test: all the tests below it change together.
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SetTestsParams {
    /// The pytest ids of the tests to run. `null` runs every test.
    pub ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct LocateTestParams {
    /// The pytest id of a file, class or test. A directory has no source to open.
    pub id: String,
}

/// Where a test is written, for the extension to open.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct TestLocation {
    /// The path of the file on the host.
    pub path: String,
    /// The line where the test starts, counting from 1.
    pub line: u32,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SetTestSuiteParams {
    pub name: String,
}

/// The language models that the extension can use for the AI review. Only VS Code knows them.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SetAiParams {
    pub models: Vec<String>,
    pub selected: Option<String>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct RunActionParams {
    pub id: ActionId,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct CancelActionParams {
    pub id: ActionId,
}

/// What to keep between sessions: the extension stores it and gives it back at `initialize`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SavedNotification {
    #[ts(type = "unknown")]
    pub saved: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ActionFinishedNotification {
    pub id: ActionId,
    pub success: bool,
    /// The user stopped it, which is not a failure to report.
    pub cancelled: bool,
    /// Why it failed.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum MessageLevel {
    Info,
    Warning,
    Error,
}

/// Something to show the user.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MessageNotification {
    pub level: MessageLevel,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum StreamName {
    Stdout,
    Stderr,
}

/// The output of what runs, one source per container (`stax`) and one for the image (`image`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum LogNotification {
    Started {
        source: String,
        command: String,
    },
    Line {
        source: String,
        stream: StreamName,
        text: String,
    },
    Finished {
        source: String,
        success: bool,
    },
    Note {
        source: String,
        text: String,
    },
}

fn source_name(source: &Source) -> String {
    match source {
        Source::Image => "image".to_owned(),
        Source::Container(device) => device.spec().name.to_owned(),
    }
}

impl From<LogEvent> for LogNotification {
    fn from(event: LogEvent) -> Self {
        match event {
            LogEvent::Started { source, command } => Self::Started {
                source: source_name(&source),
                command,
            },
            LogEvent::Line {
                source,
                stream,
                text,
            } => Self::Line {
                source: source_name(&source),
                stream: match stream {
                    Stream::Stdout => StreamName::Stdout,
                    Stream::Stderr => StreamName::Stderr,
                },
                text,
            },
            LogEvent::Finished { source, success } => Self::Finished {
                source: source_name(&source),
                success,
            },
            LogEvent::Note { source, text } => Self::Note {
                source: source_name(&source),
                text,
            },
        }
    }
}
