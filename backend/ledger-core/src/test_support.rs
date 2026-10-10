//! Fixtures for the tests of the modules that look at the whole state of an app.

use std::collections::BTreeSet;

use crate::app::App;
use crate::c_app::{MakefileInfo, Variants};
use crate::container::{ContainerState, Observation, ObservedContainer};
use crate::device::Device;
use crate::device_ops::{DeviceSettings, Onboarding};
use crate::docker::ContainerUser;
use crate::environment::{Environment, Settings};
use crate::failures::Failures;
use crate::host_python::HostPython;
use crate::manifest::{self, ParsedManifest};
use crate::platform::Platform;
use crate::rust_app::RustAppPackage;
use crate::selection::{AppSelection, SavedSelection};
use crate::test_tree::TestTree;
use crate::update::Freshness;

const BOILERPLATE: &str = include_str!("../tests/fixtures/boilerplate_c.toml");
const RUST_MANIFEST: &str = "[app]\nsdk = \"rust\"\nbuild_directory = \"./\"\n\
    devices = [\"stax\", \"flex\"]\n[pytest.standalone]\ndirectory = \"./tests/\"\n";

fn from_manifest(folder: &str, source: &str, rust: Option<RustAppPackage>) -> App {
    let ParsedManifest::Current(manifest) = manifest::parse(source).unwrap() else {
        panic!("a current manifest");
    };
    App::from_manifest(folder, &manifest, rust)
}

/// The boilerplate app: five devices, two test suites, a compose file, and its Makefile asked.
pub fn c_app() -> App {
    let mut app = from_manifest("/work/app-boilerplate", BOILERPLATE, None);
    app.makefile = Some(MakefileInfo {
        name: "Boilerplate".to_owned(),
        variants: Some(Variants {
            name: "COIN".to_owned(),
            values: vec!["BTC".to_owned(), "LTC".to_owned()],
        }),
    });
    app.compose_file = Some("docker-compose.yml".to_owned());
    app
}

pub fn rust_app() -> App {
    let package = RustAppPackage {
        package_name: "app-rust".to_owned(),
        app_name: "Rusty".to_owned(),
    };
    from_manifest("/work/app-rust", RUST_MANIFEST, Some(package))
}

/// With Stax selected, as it is neither a Nano X nor the first device of the boilerplate.
pub fn selection(app: &App) -> AppSelection {
    AppSelection::new(app, &SavedSelection::default(), Some(Device::Stax)).unwrap()
}

/// Everything in place: Docker runs the containers of the app, the tests were collected, the
/// compose service is known and the suite ships its scripts.
pub fn environment(app: &App) -> Environment {
    let containers = app
        .devices
        .iter()
        .map(|&device| ObservedContainer {
            device,
            state: ContainerState::Running,
            image_id: Some("sha256:a".to_owned()),
            pip_cache_ready: true,
            busy: false,
        })
        .collect();
    Environment {
        settings: Settings {
            verbose_tests: false,
            test_requirements: String::new(),
            device: DeviceSettings {
                scp_private_key: None,
                onboarding: Onboarding {
                    pin: "1234".to_owned(),
                    seed: "one two three".to_owned(),
                },
                allow_nano_x: false,
            },
        },
        containers: Observation {
            docker_available: true,
            image_id: Some("sha256:a".to_owned()),
            freshness: Freshness::UpToDate,
            containers,
        },
        user: ContainerUser::Host {
            uid: 1000,
            gid: 1000,
        },
        host: HostPython {
            platform: Platform::Linux,
            venv: "/home/u/venvs/app".into(),
        },
        compose_service: Some("builder".to_owned()),
        test_tree: Some(TestTree::from_collect_output(
            "test_a.py::test_x\ntest_a.py::test_y\nsub/test_b.py::test_z\n",
        )),
        collecting_tests: false,
        failures: Failures::default(),
        test_helpers: true,
        running: BTreeSet::new(),
        ai_models: vec!["gpt".to_owned()],
        ai_model: Some("gpt".to_owned()),
    }
}
