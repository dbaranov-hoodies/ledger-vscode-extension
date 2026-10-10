//! The `ledger_app.toml` manifest, in its current and legacy formats.

use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::device::Device;
use crate::error::{Error, Result};
use crate::language::Language;

/// A manifest of either format, told apart by its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedManifest {
    Current(Manifest),
    Legacy(LegacyManifest),
}

/// The current manifest format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub language: Language,
    pub build_directory: String,
    pub devices: Vec<Device>,
    /// Every `[pytest.<name>]` suite, plus the legacy `[tests]` table as the `standalone` suite.
    pub test_suites: Vec<TestSuite>,
    pub build_use_cases: Vec<BuildUseCase>,
}

/// The deprecated format, recognised by its `[rust-app]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyManifest {
    /// `rust-app.manifest-path`: path of the app's `Cargo.toml`.
    pub cargo_manifest_path: String,
}

impl LegacyManifest {
    pub const LANGUAGE: Language = Language::Rust;
    pub const DEVICES: [Device; 3] = [Device::NanoS, Device::NanoSPlus, Device::NanoX];
}

/// A group of functional tests, with the apps it needs built first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSuite {
    pub name: String,
    /// Directory of the tests, relative to the app.
    pub directory: Option<String>,
    pub use_cases: Vec<TestUseCase>,
}

/// A named set of apps to build before running the functional tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestUseCase {
    pub name: String,
    pub dependencies: Vec<TestDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestDependency {
    pub git_repo_url: String,
    pub git_repo_ref: String,
    /// Build use case of the dependency to build.
    pub use_case: String,
}

/// A named build configuration. Every use case is also a Makefile target (`make <name>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildUseCase {
    pub name: String,
    pub origin: UseCaseOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseCaseOrigin {
    /// Added by the tools because the manifest does not define it (`release`, `debug`).
    BuiltIn,
    Manifest,
}

pub fn parse(source: &str) -> Result<ParsedManifest> {
    let raw: RawManifest = toml::from_str(source)?;
    if let Some(rust_app) = raw.rust_app {
        let cargo_manifest_path = rust_app
            .manifest_path
            .ok_or(Error::MissingProperty("rust-app.manifest-path"))?;
        return Ok(ParsedManifest::Legacy(LegacyManifest {
            cargo_manifest_path,
        }));
    }
    Manifest::from_raw(raw).map(ParsedManifest::Current)
}

impl Manifest {
    /// The suite that the legacy `[tests]` table describes.
    pub const STANDALONE_SUITE: &'static str = "standalone";

    pub fn test_suite(&self, name: &str) -> Option<&TestSuite> {
        self.test_suites.iter().find(|suite| suite.name == name)
    }

    fn from_raw(raw: RawManifest) -> Result<Self> {
        let app = raw.app.unwrap_or_default();
        let sdk = app.sdk.ok_or(Error::MissingProperty("app.sdk"))?;
        let language = Language::from_manifest(&sdk)?;
        let build_directory = app
            .build_directory
            .ok_or(Error::MissingProperty("app.build_directory"))?;
        let devices = parse_devices(app.devices.ok_or(Error::MissingProperty("app.devices"))?)?;

        let legacy_tests = raw.tests.unwrap_or_default();
        let mut test_suites = raw
            .pytest
            .into_iter()
            .map(|(name, suite)| -> Result<TestSuite> {
                Ok(TestSuite {
                    name,
                    directory: non_empty(suite.directory),
                    use_cases: parse_test_use_cases(suite.dependencies)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        fold_legacy_tests(
            &mut test_suites,
            non_empty(legacy_tests.pytest_directory),
            parse_test_use_cases(legacy_tests.dependencies)?,
        );

        Ok(Self {
            language,
            build_directory,
            devices,
            test_suites,
            build_use_cases: build_use_cases(raw.use_cases.into_keys().collect()),
        })
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

/// The legacy `[tests]` table describes the `standalone` suite. Where both formats set the
/// directory or name the same use case, the current format wins.
fn fold_legacy_tests(
    suites: &mut Vec<TestSuite>,
    directory: Option<String>,
    use_cases: Vec<TestUseCase>,
) {
    if directory.is_none() && use_cases.is_empty() {
        return;
    }
    let index = suites
        .iter()
        .position(|suite| suite.name == Manifest::STANDALONE_SUITE)
        .unwrap_or_else(|| {
            suites.insert(
                0,
                TestSuite {
                    name: Manifest::STANDALONE_SUITE.to_owned(),
                    directory: None,
                    use_cases: Vec::new(),
                },
            );
            0
        });
    let suite = &mut suites[index];
    suite.directory = suite.directory.take().or(directory);
    for use_case in use_cases {
        if !suite
            .use_cases
            .iter()
            .any(|known| known.name == use_case.name)
        {
            suite.use_cases.push(use_case);
        }
    }
}

fn parse_devices(raw: RawDevices) -> Result<Vec<Device>> {
    let ids = match raw {
        RawDevices::List(ids) => ids,
        RawDevices::Csv(csv) => csv.split(',').map(|id| id.trim().to_owned()).collect(),
    };
    let devices = ids
        .iter()
        .map(|id| Device::from_name(id).ok_or_else(|| Error::InvalidDevice(id.clone())))
        .collect::<Result<Vec<_>>>()?;
    if devices.is_empty() {
        return Err(Error::NoDevices);
    }
    Ok(devices)
}

fn parse_test_use_cases(raw: IndexMap<String, Vec<RawDependency>>) -> Result<Vec<TestUseCase>> {
    raw.into_iter()
        .map(|(name, dependencies)| -> Result<TestUseCase> {
            let dependencies = dependencies
                .into_iter()
                .map(TestDependency::from_raw)
                .collect::<Result<Vec<_>>>()?;
            Ok(TestUseCase { name, dependencies })
        })
        .collect()
}

/// `release` comes first, then `debug` unless the manifest defines it, then the manifest's own
/// use cases in file order.
fn build_use_cases(defined: Vec<String>) -> Vec<BuildUseCase> {
    let origin_of = |name: &str| {
        if defined.iter().any(|defined| defined == name) {
            UseCaseOrigin::Manifest
        } else {
            UseCaseOrigin::BuiltIn
        }
    };

    let mut use_cases = vec![BuildUseCase {
        name: "release".to_owned(),
        origin: origin_of("release"),
    }];
    if origin_of("debug") == UseCaseOrigin::BuiltIn {
        use_cases.push(BuildUseCase {
            name: "debug".to_owned(),
            origin: UseCaseOrigin::BuiltIn,
        });
    }
    use_cases.extend(
        defined
            .iter()
            .filter(|name| name.as_str() != "release")
            .map(|name| BuildUseCase {
                name: name.clone(),
                origin: UseCaseOrigin::Manifest,
            }),
    );
    use_cases
}

impl TestDependency {
    fn from_raw(raw: RawDependency) -> Result<Self> {
        Ok(Self {
            git_repo_url: raw.url.ok_or(Error::MissingProperty("url"))?,
            git_repo_ref: raw.git_ref.ok_or(Error::MissingProperty("ref"))?,
            use_case: raw.use_case.ok_or(Error::MissingProperty("use_case"))?,
        })
    }
}

#[derive(Deserialize)]
struct RawManifest {
    app: Option<RawApp>,
    #[serde(rename = "rust-app")]
    rust_app: Option<RawRustApp>,
    #[serde(default)]
    pytest: IndexMap<String, RawPytestSuite>,
    tests: Option<RawTests>,
    #[serde(default)]
    use_cases: IndexMap<String, IgnoredAny>,
}

#[derive(Deserialize, Default)]
struct RawApp {
    sdk: Option<String>,
    build_directory: Option<String>,
    devices: Option<RawDevices>,
}

/// Devices are a TOML array, or, in older manifests, a comma-separated string.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawDevices {
    List(Vec<String>),
    Csv(String),
}

#[derive(Deserialize)]
struct RawRustApp {
    #[serde(rename = "manifest-path")]
    manifest_path: Option<String>,
}

#[derive(Deserialize)]
struct RawPytestSuite {
    directory: Option<String>,
    #[serde(default)]
    dependencies: IndexMap<String, Vec<RawDependency>>,
}

#[derive(Deserialize, Default)]
struct RawTests {
    pytest_directory: Option<String>,
    #[serde(default)]
    dependencies: IndexMap<String, Vec<RawDependency>>,
}

#[derive(Deserialize)]
struct RawDependency {
    url: Option<String>,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    use_case: Option<String>,
}
