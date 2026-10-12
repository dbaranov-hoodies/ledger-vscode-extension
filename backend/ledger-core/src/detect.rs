//! Finds out whether a folder holds a Ledger app, from the files the caller read in it.

use std::collections::BTreeMap;
use std::path::Path;

use crate::app::{App, join_path};
use crate::device::Device;
use crate::error::{Error, Result};
use crate::language::Language;
use crate::manifest::{self, AppManifest, LegacyManifest, ParsedManifest, TestSuite};
use crate::rust_app::{self, RustAppPackage};
use crate::sideloaded;

const MANIFEST_FILE: &str = "ledger_app.toml";
const MAKEFILE: &str = "Makefile";
const CARGO_TOML: &str = "Cargo.toml";
const PYTEST_CONFIG: &str = "conftest.py";
const COMPOSE_FILE: &str = "docker-compose.yml";
/// What a Makefile includes when it builds a Ledger C app.
const C_APP_MARKER: &str = "include $(BOLOS_SDK)/Makefile.defines";

/// An app without a manifest does not say which devices it supports.
const DEFAULT_DEVICES: [Device; 5] = [
    Device::NanoSPlus,
    Device::NanoX,
    Device::Stax,
    Device::Flex,
    Device::ApexP,
];

/// The files of an app folder that detection reads, with their content.
#[derive(Debug, Clone, Default)]
pub struct FolderFiles {
    files: BTreeMap<String, String>,
}

impl FolderFiles {
    /// The names of the files to read, wherever they are in the folder up to two levels deep.
    /// The compose file only needs to be found, its content can be empty. So can the content of
    /// `conftest.py`, but then the apps that a suite loads are not known.
    pub const NAMES: [&str; 5] = [
        MANIFEST_FILE,
        MAKEFILE,
        CARGO_TOML,
        PYTEST_CONFIG,
        COMPOSE_FILE,
    ];

    /// Takes paths relative to the app folder, with `/` or `\` as separator.
    pub fn new(files: impl IntoIterator<Item = (String, String)>) -> Self {
        let files = files
            .into_iter()
            .map(|(path, content)| (normalize(&path), content))
            .collect();
        Self { files }
    }

    fn get(&self, path: &str) -> Option<&str> {
        self.files.get(&normalize(path)).map(String::as_str)
    }

    /// The files with this name, the ones closest to the app folder first.
    fn named(&self, name: &str) -> Vec<(&str, &str)> {
        let mut found: Vec<_> = self
            .files
            .iter()
            .filter(|(path, _)| path.rsplit('/').next() == Some(name))
            .map(|(path, content)| (path.as_str(), content.as_str()))
            .collect();
        found.sort_by_key(|(path, _)| path.matches('/').count());
        found
    }
}

fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut path = path.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    path.to_owned()
}

/// The directory of a file, written like the manifest does: `./` or `./app`.
fn directory_of(path: &str) -> String {
    match normalize(path).rsplit_once('/') {
        Some((directory, _)) => format!("./{directory}"),
        None => "./".to_owned(),
    }
}

/// What a detected app lacks that its owner should be told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestWarning {
    Missing,
    /// The `[rust-app]` manifest.
    Deprecated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub app: App,
    pub warning: Option<ManifestWarning>,
}

/// `None` when the folder holds no app. A C app comes back without its name and variants,
/// which only its Makefile can tell.
pub fn detect_app(folder: &Path, files: &FolderFiles) -> Result<Option<Detected>> {
    let detected = find_app(folder, files)?;
    Ok(detected.map(|mut detected| {
        let compose = files.named(COMPOSE_FILE).first().map(|(path, _)| *path);
        detected.app.compose_file = compose.map(str::to_owned);
        attach_sideloaded(&mut detected.app, files);
        detected
    }))
}

/// Reads the apps that each suite loads from its `conftest.py`, when the caller gave the content.
fn attach_sideloaded(app: &mut App, files: &FolderFiles) {
    for suite in &mut app.test_suites {
        let Some(directory) = &suite.directory else {
            continue;
        };
        suite.sideloaded = files
            .get(&join_path(directory, sideloaded::CONFTEST))
            .and_then(sideloaded::parse_conftest);
    }
}

fn find_app(folder: &Path, files: &FolderFiles) -> Result<Option<Detected>> {
    if let Some((_, source)) = files.named(MANIFEST_FILE).first() {
        let detected = match manifest::parse(source)? {
            ParsedManifest::Current(manifest) => from_manifest(folder, &manifest, files)?,
            ParsedManifest::Legacy(legacy) => from_legacy_manifest(folder, &legacy, files)?,
        };
        return Ok(Some(detected));
    }
    let makefile = files
        .named(MAKEFILE)
        .into_iter()
        .find(|(_, source)| source.contains(C_APP_MARKER));
    Ok(makefile.map(|(path, _)| from_makefile(folder, path, files)))
}

fn from_manifest(folder: &Path, manifest: &AppManifest, files: &FolderFiles) -> Result<Detected> {
    let rust = match manifest.language {
        Language::Rust => {
            let path = join_path(&manifest.build_directory, CARGO_TOML);
            Some(read_cargo_toml(files, &path)?)
        }
        Language::C => None,
    };
    Ok(Detected {
        app: App::from_manifest(folder, manifest, rust),
        warning: None,
    })
}

fn from_legacy_manifest(
    folder: &Path,
    legacy: &LegacyManifest,
    files: &FolderFiles,
) -> Result<Detected> {
    let app = App {
        build_directory: directory_of(&legacy.cargo_manifest_path),
        devices: LegacyManifest::DEVICES.to_vec(),
        rust: Some(read_cargo_toml(files, &legacy.cargo_manifest_path)?),
        ..unmanifested(folder, LegacyManifest::LANGUAGE, files)
    };
    Ok(Detected {
        app,
        warning: Some(ManifestWarning::Deprecated),
    })
}

fn from_makefile(folder: &Path, makefile: &str, files: &FolderFiles) -> Detected {
    let app = App {
        build_directory: directory_of(makefile),
        ..unmanifested(folder, Language::C, files)
    };
    Detected {
        app,
        warning: Some(ManifestWarning::Missing),
    }
}

/// An app that has no current manifest: its tests are wherever `conftest.py` is, in the folder or
/// the one below. The caller also sends the deeper ones, for the manifest to point at.
fn unmanifested(folder: &Path, language: Language, files: &FolderFiles) -> App {
    let test_suites = files
        .named(PYTEST_CONFIG)
        .into_iter()
        .find(|(path, _)| path.matches('/').count() <= 1)
        .map(|(path, _)| TestSuite {
            name: AppManifest::STANDALONE_SUITE.to_owned(),
            directory: Some(directory_of(path)),
            use_cases: Vec::new(),
            sideloaded: None,
        })
        .into_iter()
        .collect();
    App {
        folder: folder.to_path_buf(),
        language,
        build_directory: "./".to_owned(),
        devices: DEFAULT_DEVICES.to_vec(),
        rust: None,
        build_use_cases: Vec::new(),
        test_suites,
        makefile: None,
        compose_file: None,
    }
}

fn read_cargo_toml(files: &FolderFiles, path: &str) -> Result<RustAppPackage> {
    let source = files
        .get(path)
        .ok_or_else(|| Error::MissingFile(path.to_owned()))?;
    rust_app::parse_cargo_toml(source)
}
