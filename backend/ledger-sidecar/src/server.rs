//! The state of the extension, and what is done with each request.
//!
//! Requests that change a choice answer with the new snapshot. What happens later, such as a
//! build running, is told with notifications, and the snapshot is sent again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ledger_core::actions::{self, ActionId, ContainerAction, DisabledReason, Run};
use ledger_core::app::App;
use ledger_core::container::Observation;
use ledger_core::detect::{self, FolderFiles, ManifestWarning};
use ledger_core::device::Device;
use ledger_core::device_ops::{DeviceSettings, Onboarding};
use ledger_core::docker::{self, ContainerUser};
use ledger_core::environment::{self, Environment};
use ledger_core::host_python::HostPython;
use ledger_core::language::Language;
use ledger_core::platform::Platform;
use ledger_core::selection::{AppSelection, SavedSelection};
use ledger_core::snapshot::{self, Snapshot};
use ledger_core::target::Target;
use ledger_core::task::{Plan, Step, Warning};
use ledger_core::update::{CheckPolicy, Freshness};
use ledger_core::workspace::{SavedWorkspace, Workspace};
use ledger_runtime::{CommandRunner, Config, ContainerManager, JobOutcome, LogSender, Outcome};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

use crate::protocol::{self, *};
use crate::wire::{self, ErrorCode, Request, RpcError};

/// The bounds a command runner needs to be used from background tasks.
pub trait Runner: CommandRunner + Clone + Send + Sync + 'static {}
impl<T: CommandRunner + Clone + Send + Sync + 'static> Runner for T {}

pub struct Server<R: Runner> {
    inner: Arc<Inner<R>>,
}

impl<R: Runner> Clone for Server<R> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<R: Runner> {
    runner: R,
    /// Every line to send to the extension.
    out: mpsc::UnboundedSender<String>,
    logs: LogSender,
    state: Mutex<State<R>>,
}

struct State<R: Runner> {
    session: Option<Session>,
    folders: Vec<FolderInput>,
    saved: SavedWorkspace,
    workspace: Workspace,
    apps: BTreeMap<PathBuf, ManagedApp<R>>,
    /// What Docker reports when there is no app to look at.
    docker_only: Observation,
    running: BTreeSet<ActionId>,
    warned: BTreeSet<PathBuf>,
}

#[derive(Clone)]
struct Session {
    platform: Platform,
    host_ids: Option<(u32, u32)>,
    display: Option<String>,
    settings: SettingsInput,
}

struct ManagedApp<R: Runner> {
    host_venv: String,
    config: Config,
    manager: Arc<ContainerManager<R>>,
    observation: Observation,
}

fn unknown_docker() -> Observation {
    Observation {
        docker_available: false,
        image_id: None,
        freshness: Freshness::Unknown,
        containers: Vec::new(),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params).map_err(|error| RpcError::invalid_params(error.to_string()))
}

fn not_initialized() -> RpcError {
    RpcError::failed("the sidecar was not initialized")
}

fn no_app() -> RpcError {
    RpcError::failed("there is no app")
}

fn to_value(value: &impl Serialize) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|error| RpcError::failed(error.to_string()))
}

impl<R: Runner> Server<R> {
    /// The server, and the receiver of the lines it sends. Call it inside a Tokio runtime.
    pub fn new(runner: R) -> (Self, mpsc::UnboundedReceiver<String>) {
        let (out, lines) = mpsc::unbounded_channel();
        let (logs, mut events) = ledger_runtime::log::channel();
        let forward = out.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let line = wire::notification(protocol::LOG, &LogNotification::from(event));
                if forward.send(line).is_err() {
                    break;
                }
            }
        });
        let state = State {
            session: None,
            folders: Vec::new(),
            saved: SavedWorkspace::default(),
            workspace: Workspace::default(),
            apps: BTreeMap::new(),
            docker_only: unknown_docker(),
            running: BTreeSet::new(),
            warned: BTreeSet::new(),
        };
        let inner = Inner {
            runner,
            out,
            logs,
            state: Mutex::new(state),
        };
        (
            Self {
                inner: Arc::new(inner),
            },
            lines,
        )
    }

    /// Handles one line from the extension: answers a request, ignores a notification.
    pub async fn handle(&self, line: &str) {
        let request: Request = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(error) => {
                let error = RpcError::new(ErrorCode::ParseError, error.to_string());
                self.send(wire::response(&Value::Null, &Err(error)));
                return;
            }
        };
        let result = self.dispatch(&request.method, request.params).await;
        if let Some(id) = request.id {
            self.send(wire::response(&id, &result));
        }
    }

    fn send(&self, line: String) {
        // Nobody is listening once the extension is gone, and then nothing matters.
        let _ = self.inner.out.send(line);
    }

    fn notify(&self, method: &str, params: &impl Serialize) {
        self.send(wire::notification(method, params));
    }

    fn message(&self, level: MessageLevel, text: impl Into<String>) {
        let text = text.into();
        self.notify(protocol::MESSAGE, &MessageNotification { level, text });
    }

    async fn dispatch(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            protocol::INITIALIZE => self.initialize(parse(params)?).await,
            protocol::REFRESH => self.refresh(parse(params)?).await,
            protocol::UPDATE_SETTINGS => self.update_settings(parse(params)?).await,
            protocol::SELECT_APP => self.select_app(parse(params)?).await,
            protocol::SET_DEVICES => self.set_devices(parse(params)?).await,
            protocol::SET_BUILD_USE_CASE => {
                let SetBuildUseCaseParams { name } = parse(params)?;
                self.choose("build use case", move |app, selection| {
                    selection.set_build_use_case(&name, app)
                })
                .await
            }
            protocol::SET_VARIANT => {
                let SetVariantParams { value } = parse(params)?;
                self.choose("variant", move |app, selection| {
                    selection.set_variant(&value, app)
                })
                .await
            }
            protocol::SET_ENFORCER_CHECK => {
                let SetEnforcerCheckParams { name } = parse(params)?;
                self.choose("check", move |_, selection| {
                    selection.set_enforcer_check(&name)
                })
                .await
            }
            protocol::RUN_ACTION => self.run_action(parse::<RunActionParams>(params)?.id).await,
            other => Err(RpcError::new(
                ErrorCode::MethodNotFound,
                format!("unknown method `{other}`"),
            )),
        }
    }

    // ----- requests

    async fn initialize(&self, params: InitializeParams) -> Result<Value, RpcError> {
        let session = Session {
            platform: params.platform.platform(),
            host_ids: params.host_ids.map(|ids| (ids.uid, ids.gid)),
            display: params.display,
            settings: params.settings,
        };
        {
            let mut state = self.inner.state.lock().await;
            state.session = Some(session);
            state.saved = serde_json::from_value(params.saved).unwrap_or_default();
        }
        self.apply_folders(params.folders).await;
        self.settled(true).await
    }

    async fn refresh(&self, params: RefreshParams) -> Result<Value, RpcError> {
        let files_changed = params.folders.is_some();
        if let Some(folders) = params.folders {
            self.apply_folders(folders).await;
        }
        self.settled(files_changed).await
    }

    async fn update_settings(&self, settings: SettingsInput) -> Result<Value, RpcError> {
        let folders = {
            let mut state = self.inner.state.lock().await;
            state.session.as_mut().ok_or_else(not_initialized)?.settings = settings;
            state.folders.clone()
        };
        self.apply_folders(folders).await;
        self.settled(false).await
    }

    async fn select_app(&self, params: SelectAppParams) -> Result<Value, RpcError> {
        {
            let mut state = self.inner.state.lock().await;
            if !state.workspace.select(Path::new(&params.folder)) {
                return Err(RpcError::failed(format!("no app in {}", params.folder)));
            }
        }
        self.settled(false).await
    }

    async fn set_devices(&self, params: SetDevicesParams) -> Result<Value, RpcError> {
        let devices = params
            .devices
            .iter()
            .map(|id| {
                Device::from_name(id)
                    .ok_or_else(|| RpcError::invalid_params(format!("unknown device `{id}`")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let target = Target::new(devices)
            .ok_or_else(|| RpcError::invalid_params("at least one device is needed"))?;
        self.choose("device", move |app, selection| {
            selection.set_target(&target, app).is_some()
        })
        .await
    }

    /// Changes a choice of the selected app and answers with the new snapshot.
    async fn choose(
        &self,
        what: &str,
        change: impl FnOnce(&App, &mut AppSelection) -> bool,
    ) -> Result<Value, RpcError> {
        {
            let mut state = self.inner.state.lock().await;
            let (app, selection) = state.workspace.current_mut().ok_or_else(no_app)?;
            if !change(app, selection) {
                return Err(RpcError::invalid_params(format!(
                    "the app does not offer this {what}"
                )));
            }
        }
        self.push_saved().await;
        let state = self.inner.state.lock().await;
        to_value(&state.snapshot()?)
    }

    /// After the files or the choices changed: looks at Docker, tells what to keep, and answers.
    async fn settled(&self, ask_makefile_again: bool) -> Result<Value, RpcError> {
        self.observe_current().await;
        self.push_saved().await;
        self.query_makefile_info(ask_makefile_again);
        let state = self.inner.state.lock().await;
        to_value(&state.snapshot()?)
    }

    // ----- detection and containers

    async fn apply_folders(&self, folders: Vec<FolderInput>) {
        let mut apps = Vec::new();
        let mut notices = Vec::new();
        for folder in &folders {
            let files = FolderFiles::new(
                folder
                    .files
                    .iter()
                    .map(|file| (file.path.clone(), file.content.clone())),
            );
            match detect::detect_app(Path::new(&folder.path), &files) {
                Ok(Some(detected)) => {
                    if let Some(warning) = detected.warning {
                        notices.push((detected.app.folder.clone(), warning));
                    }
                    apps.push(detected.app);
                }
                Ok(None) => {}
                Err(error) => {
                    let text = format!("App detection failed in {}. {error}", folder.path);
                    self.message(MessageLevel::Error, text);
                }
            }
        }

        let mut state = self.inner.state.lock().await;
        let Some(session) = state.session.clone() else {
            return;
        };
        for (folder, warning) in notices {
            if state.warned.insert(folder.clone()) {
                self.message(MessageLevel::Warning, manifest_warning(&folder, warning));
            }
        }
        let default_device = session
            .settings
            .default_device
            .as_deref()
            .and_then(Device::from_display_name);
        // What the former extension saved is only used for an app nothing was saved for yet.
        for folder in &folders {
            let path = PathBuf::from(&folder.path);
            if !folder.legacy_settings.is_null() && !state.saved.apps.contains_key(&path) {
                let legacy = SavedSelection::from_legacy_settings(&folder.legacy_settings);
                state.saved.apps.insert(path, legacy);
            }
        }
        let saved = state.saved.clone();
        state.workspace.refresh(apps, &saved, default_device);
        state.folders = folders;
        self.update_managers(&mut state, &session);
    }

    /// Gives every app a manager that fits the settings, and drops those of apps that are gone.
    fn update_managers(&self, state: &mut State<R>, session: &Session) {
        let wanted: Vec<(App, Option<FolderInput>)> = state
            .workspace
            .apps()
            .map(|(app, _)| {
                let folder = state
                    .folders
                    .iter()
                    .find(|folder| Path::new(&folder.path) == app.folder);
                (app.clone(), folder.cloned())
            })
            .collect();
        let keep: BTreeSet<PathBuf> = wanted.iter().map(|(app, _)| app.folder.clone()).collect();
        state.apps.retain(|folder, _| keep.contains(folder));

        for (app, folder) in wanted {
            let Some(folder) = folder else { continue };
            let config = self.config_for(session, &folder, &app);
            let unchanged = state
                .apps
                .get(&app.folder)
                .is_some_and(|managed| managed.config == config);
            if unchanged {
                continue;
            }
            let manager = ContainerManager::new(self.inner.runner.clone(), config.clone())
                .with_logs(self.inner.logs.clone());
            let observation = state
                .apps
                .remove(&app.folder)
                .map_or_else(unknown_docker, |managed| managed.observation);
            state.apps.insert(
                app.folder.clone(),
                ManagedApp {
                    host_venv: folder.host_venv.clone(),
                    config,
                    manager: Arc::new(manager),
                    observation,
                },
            );
        }
    }

    fn config_for(&self, session: &Session, folder: &FolderInput, app: &App) -> Config {
        let settings = &session.settings;
        let extra_args = shell_words::split(&settings.docker_run_args).unwrap_or_else(|error| {
            let text = format!("The docker run arguments of the settings are not valid: {error}");
            self.message(MessageLevel::Warning, text);
            Vec::new()
        });
        Config {
            image: settings.docker_image.clone(),
            app_folder: app.folder_name(),
            app_dir: docker::host_mount_path(&folder.uri_path, session.platform),
            auto_update: settings.auto_update,
            max_retries: settings.max_retries,
            user: user_of(session),
            extra_args,
            platform: session.platform,
            display: session.display.clone(),
        }
    }

    /// Asks Docker about the containers of the selected app, or about Docker alone.
    async fn observe_current(&self) {
        let target = {
            let state = self.inner.state.lock().await;
            state.workspace.current().and_then(|(app, _)| {
                let managed = state.apps.get(&app.folder)?;
                Some((
                    app.folder.clone(),
                    app.devices.clone(),
                    Arc::clone(&managed.manager),
                ))
            })
        };
        match target {
            Some((folder, devices, manager)) => {
                let observation = manager.observe(&devices).await;
                let mut state = self.inner.state.lock().await;
                if let Some(managed) = state.apps.get_mut(&folder) {
                    managed.observation = observation;
                }
            }
            None => {
                let command = docker::info();
                let output = self.inner.runner.run(&command, None).await;
                let mut state = self.inner.state.lock().await;
                state.docker_only = Observation {
                    docker_available: output.is_ok_and(|output| output.success),
                    ..unknown_docker()
                };
            }
        }
    }

    /// Asks the Makefile of the selected C app for its name and variants, in the background.
    /// The image has to be there already, so this is tried again after every action.
    fn query_makefile_info(&self, force: bool) {
        let server = self.clone();
        tokio::spawn(async move {
            let job = {
                let state = server.inner.state.lock().await;
                state.workspace.current().and_then(|(app, _)| {
                    let managed = state.apps.get(&app.folder)?;
                    let wanted = app.language == Language::C && (force || app.makefile.is_none());
                    (wanted && managed.observation.image_present()).then(|| {
                        (
                            app.folder.clone(),
                            app.build_directory.clone(),
                            Arc::clone(&managed.manager),
                        )
                    })
                })
            };
            let Some((folder, build_directory, manager)) = job else {
                return;
            };
            // Not an error to show: the image or the Makefile may not be ready yet.
            let Ok(info) = manager.makefile_info(&build_directory).await else {
                return;
            };
            let known = {
                let mut state = server.inner.state.lock().await;
                state.workspace.set_makefile_info(&folder, info)
            };
            if known {
                server.push_saved().await;
                server.push_snapshot().await;
            }
        });
    }

    // ----- notifications

    async fn push_snapshot(&self) {
        let state = self.inner.state.lock().await;
        if let Ok(snapshot) = state.snapshot() {
            self.notify(protocol::SNAPSHOT, &snapshot);
        }
    }

    async fn push_saved(&self) {
        let mut state = self.inner.state.lock().await;
        let saved = state.workspace.saved();
        state.saved = saved.clone();
        if let Ok(saved) = serde_json::to_value(&saved) {
            self.notify(protocol::SAVED, &SavedNotification { saved });
        }
    }

    // ----- actions

    async fn run_action(&self, id: ActionId) -> Result<Value, RpcError> {
        let prepared = self.prepare(id).await?;
        let server = self.clone();
        tokio::spawn(async move { server.execute(id, prepared).await });
        self.push_snapshot().await;
        Ok(Value::Null)
    }

    /// Checks that the action can start, and marks it as running.
    async fn prepare(&self, id: ActionId) -> Result<Prepared<R>, RpcError> {
        let mut state = self.inner.state.lock().await;
        let session = state.session.as_ref().ok_or_else(not_initialized)?;
        let (app, selection) = state.workspace.current().ok_or_else(no_app)?;
        let env = state.environment(session, app);

        let groups = actions::groups(app, selection, &env);
        let view = groups
            .iter()
            .flat_map(|group| group.main.iter().chain(&group.options))
            .find(|view| view.id == id)
            .ok_or_else(|| RpcError::failed("this action is not offered for the app"))?;
        if let Some(reason) = view.disabled_reason {
            return Err(RpcError::failed(disabled_text(reason)));
        }
        if state.running.contains(&id) {
            return Err(RpcError::failed("this action is already running"));
        }
        let managed = state.apps.get(&app.folder).ok_or_else(no_app)?;
        let manager = Arc::clone(&managed.manager);
        let auto_update = managed.config.auto_update;
        let selected: Vec<Device> = selection.target().devices().iter().copied().collect();

        let prepared = match actions::run(id) {
            Run::Task(_) => {
                let plan = actions::plan(id, app, selection, &env)
                    .ok_or_else(|| RpcError::failed("this action is not a task"))?
                    .map_err(|error| RpcError::failed(error.to_string()))?;
                if is_interactive(&plan) {
                    return Err(RpcError::failed(
                        "this action opens an interactive terminal, which is not supported yet",
                    ));
                }
                Prepared::Plan {
                    plan,
                    manager,
                    auto_update,
                }
            }
            Run::Containers(action) => Prepared::Containers {
                action,
                devices: selected,
                manager,
            },
            Run::Shell => {
                return Err(RpcError::failed("this action is run by the extension"));
            }
        };
        state.running.insert(id);
        Ok(prepared)
    }

    /// Runs the action to its end, then tells how it went.
    async fn execute(&self, id: ActionId, prepared: Prepared<R>) {
        let result = match prepared {
            Prepared::Plan {
                plan,
                manager,
                auto_update,
            } => self.run_plan(&plan, &manager, auto_update).await,
            Prepared::Containers {
                action,
                devices,
                manager,
            } => manager.run_container_action(action, &devices, now()).await,
        };
        {
            let mut state = self.inner.state.lock().await;
            state.running.remove(&id);
        }
        self.observe_current().await;
        let (success, message) = match result {
            Ok(()) => (true, None),
            Err(message) => (false, Some(message)),
        };
        self.notify(
            protocol::ACTION_FINISHED,
            &ActionFinishedNotification {
                id,
                success,
                message,
            },
        );
        self.push_snapshot().await;
        self.query_makefile_info(false);
    }

    async fn run_plan(
        &self,
        plan: &Plan,
        manager: &ContainerManager<R>,
        auto_update: bool,
    ) -> Result<(), String> {
        for warning in &plan.warnings {
            match warning {
                Warning::DefaultOnboardingSeed => self.message(
                    MessageLevel::Warning,
                    "Do not use default onboarding seed with real funds !",
                ),
            }
        }
        let mut devices: Vec<Device> = plan
            .jobs
            .iter()
            .filter(|job| job.uses_container())
            .map(|job| job.device)
            .collect();
        devices.dedup();
        if !devices.is_empty() {
            if auto_update {
                manager
                    .check_for_update(now(), &CheckPolicy::default())
                    .await;
            }
            match manager.ensure_ready(&devices, now()).await {
                Outcome::Settled(_) => {}
                Outcome::DockerUnavailable => return Err("Docker is not available".to_owned()),
                Outcome::Failed(reason) => return Err(reason),
            }
            // The image is there now, so the Makefile can be asked.
            self.observe_current().await;
            self.query_makefile_info(false);
        }
        let results = manager.run_plan(plan).await;
        let failures: Vec<String> = results
            .iter()
            .filter_map(|result| match &result.outcome {
                JobOutcome::Succeeded => None,
                JobOutcome::Failed { step, reason } => Some(format!(
                    "{}: step {} failed: {reason}",
                    result.device.spec().display_name,
                    step + 1
                )),
            })
            .collect();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("\n"))
        }
    }
}

enum Prepared<R: Runner> {
    Plan {
        plan: Plan,
        manager: Arc<ContainerManager<R>>,
        auto_update: bool,
    },
    Containers {
        action: ContainerAction,
        devices: Vec<Device>,
        manager: Arc<ContainerManager<R>>,
    },
}

fn is_interactive(plan: &Plan) -> bool {
    plan.jobs
        .iter()
        .flat_map(|job| &job.steps)
        .any(|step| match step {
            Step::InContainer(step) => step.tty,
            Step::OnHost(step) => step.tty,
        })
}

fn user_of(session: &Session) -> ContainerUser {
    ContainerUser::resolve(
        session.settings.open_as_root,
        session.platform,
        session.host_ids,
    )
}

fn disabled_text(reason: DisabledReason) -> &'static str {
    match reason {
        DisabledReason::SeveralDevices => "this action needs a single device",
        DisabledReason::DeviceActionsOff => "actions on a Nano X are turned off in the settings",
        DisabledReason::NoComposeFile => "the app has no docker-compose.yml",
        DisabledReason::NoTestSuite => "the app has no test suite",
        DisabledReason::NoAiModel => "no AI model is available",
    }
}

fn manifest_warning(folder: &Path, warning: ManifestWarning) -> String {
    let (what, fix) = match warning {
        ManifestWarning::Missing => ("does not have a manifest", "add a manifest to your app"),
        ManifestWarning::Deprecated => (
            "has a deprecated manifest",
            "update your manifest to the new format",
        ),
    };
    format!(
        "App found in {} {what}. Please refer to https://github.com/LedgerHQ/ledgered/blob/master/doc/manifest.md to {fix}.",
        folder.display()
    )
}

impl<R: Runner> State<R> {
    fn environment(&self, session: &Session, app: &App) -> Environment {
        let settings = &session.settings;
        let managed = self.apps.get(&app.folder);
        Environment {
            settings: environment::Settings {
                verbose_tests: settings.verbose_tests,
                test_requirements: settings
                    .test_requirements
                    .get(&app.folder_name())
                    .cloned()
                    .unwrap_or_default(),
                device: DeviceSettings {
                    scp_private_key: settings.scp_private_key.clone(),
                    onboarding: Onboarding {
                        pin: settings.onboarding_pin.clone(),
                        seed: settings.onboarding_seed.clone(),
                    },
                    allow_nano_x: settings.allow_nano_x,
                },
            },
            containers: managed.map_or_else(
                || self.docker_only.clone(),
                |managed| managed.observation.clone(),
            ),
            user: user_of(session),
            host: HostPython {
                platform: session.platform,
                venv: managed
                    .map(|managed| managed.host_venv.clone())
                    .unwrap_or_default()
                    .into(),
            },
            compose_service: None,
            test_tree: None,
            collecting_tests: false,
            test_helpers: false,
            running: self.running.clone(),
            ai_models: Vec::new(),
            ai_model: None,
        }
    }

    fn snapshot(&self) -> Result<Snapshot, RpcError> {
        let session = self.session.as_ref().ok_or_else(not_initialized)?;
        let env = match self.workspace.current() {
            Some((app, _)) => self.environment(session, app),
            None => Environment {
                containers: self.docker_only.clone(),
                ..self.bare_environment(session)
            },
        };
        Ok(snapshot::build(&self.workspace, &env))
    }

    /// An environment for a workspace without apps, where only Docker matters.
    fn bare_environment(&self, session: &Session) -> Environment {
        let settings = &session.settings;
        Environment {
            settings: environment::Settings {
                verbose_tests: settings.verbose_tests,
                test_requirements: String::new(),
                device: DeviceSettings {
                    scp_private_key: None,
                    onboarding: Onboarding {
                        pin: String::new(),
                        seed: String::new(),
                    },
                    allow_nano_x: settings.allow_nano_x,
                },
            },
            containers: self.docker_only.clone(),
            user: user_of(session),
            host: HostPython {
                platform: session.platform,
                venv: PathBuf::new(),
            },
            compose_service: None,
            test_tree: None,
            collecting_tests: false,
            test_helpers: false,
            running: BTreeSet::new(),
            ai_models: Vec::new(),
            ai_model: None,
        }
    }
}
