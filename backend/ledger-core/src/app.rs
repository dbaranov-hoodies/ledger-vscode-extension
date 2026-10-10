//! The Ledger app being built, as far as running its tasks needs to know.

use std::path::PathBuf;

use crate::c_app::MakefileInfo;
use crate::container;
use crate::device::Device;
use crate::language::Language;
use crate::manifest::{AppManifest, BuildUseCase, TestSuite};
use crate::rust_app::RustAppPackage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// Host path of the app folder. It identifies the app: two folders can share a name.
    pub folder: PathBuf,
    pub language: Language,
    /// As written in the manifest, relative to the app folder: `./`, `./app`, …
    pub build_directory: String,
    /// The devices the app supports, in manifest order.
    pub devices: Vec<Device>,
    /// The names a Rust app's `Cargo.toml` declares.
    pub rust: Option<RustAppPackage>,
    pub build_use_cases: Vec<BuildUseCase>,
    pub test_suites: Vec<TestSuite>,
    /// What a C app's Makefile prints. `None` until a container has been asked.
    pub makefile: Option<MakefileInfo>,
    /// The `docker-compose.yml` of the app, relative to the app folder.
    pub compose_file: Option<String>,
}

impl App {
    pub fn from_manifest(
        folder: impl Into<PathBuf>,
        manifest: &AppManifest,
        rust: Option<RustAppPackage>,
    ) -> Self {
        Self {
            folder: folder.into(),
            language: manifest.language,
            build_directory: manifest.build_directory.clone(),
            devices: manifest.devices.clone(),
            rust,
            build_use_cases: manifest.build_use_cases.clone(),
            test_suites: manifest.test_suites.clone(),
            makefile: None,
            compose_file: None,
        }
    }

    pub fn folder_name(&self) -> String {
        self.folder
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    /// The name the app has on the device. A C app's is known once its Makefile was asked.
    pub fn name(&self) -> Option<&str> {
        match (&self.rust, &self.makefile) {
            (Some(rust), _) => Some(&rust.app_name),
            (None, Some(makefile)) => Some(&makefile.name),
            (None, None) => None,
        }
    }

    pub fn container_name(&self, device: Device) -> String {
        container::container_name(&self.folder_name(), device)
    }

    /// A path inside the build directory, relative to the app folder.
    pub fn in_build_directory(&self, path: &str) -> String {
        join_path(&self.build_directory, path)
    }
}

/// Appends a relative path to a directory, without doubling slashes or keeping a lone `.`.
pub fn join_path(directory: &str, path: &str) -> String {
    match directory.trim_end_matches('/') {
        "" | "." => path.to_owned(),
        directory => format!("{directory}/{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{self, ParsedManifest};

    #[test]
    fn paths_are_joined_without_doubled_slashes() {
        assert_eq!(join_path("./app", "build/x"), "./app/build/x");
        assert_eq!(join_path("./app/", "build/x"), "./app/build/x");
        assert_eq!(join_path("./", "build/x"), "build/x");
        assert_eq!(join_path(".", "build/x"), "build/x");
        assert_eq!(join_path("", "build/x"), "build/x");
    }

    fn manifest(source: &str) -> AppManifest {
        let ParsedManifest::Current(manifest) = manifest::parse(source).unwrap() else {
            panic!("a current manifest");
        };
        manifest
    }

    #[test]
    fn an_app_takes_its_facts_from_the_manifest() {
        let manifest = manifest(
            "[app]\nsdk = \"C\"\nbuild_directory = \"./app\"\ndevices = [\"stax\", \"flex\"]\n",
        );

        let app = App::from_manifest("/work/app-x", &manifest, None);

        assert_eq!(app.language, Language::C);
        assert_eq!(app.devices, [Device::Stax, Device::Flex]);
        assert_eq!(app.build_use_cases, manifest.build_use_cases);
        assert_eq!(app.folder_name(), "app-x");
        assert_eq!(app.container_name(Device::Flex), "app-x-container-flex");
        assert_eq!(app.in_build_directory("build/stax"), "./app/build/stax");
    }

    #[test]
    fn a_rust_app_is_named_by_its_cargo_toml_and_a_c_app_by_its_makefile() {
        let c = manifest("[app]\nsdk = \"C\"\nbuild_directory = \"./\"\ndevices = [\"stax\"]\n");
        let mut app = App::from_manifest("/work/app-x", &c, None);
        assert_eq!(app.name(), None);

        app.makefile = Some(MakefileInfo {
            name: "X".to_owned(),
            variants: None,
        });
        assert_eq!(app.name(), Some("X"));

        app.rust = Some(RustAppPackage {
            package_name: "x-rust".to_owned(),
            app_name: "XR".to_owned(),
        });
        assert_eq!(app.name(), Some("XR"));
    }

    #[test]
    fn two_folders_with_the_same_name_are_different_apps() {
        let manifest =
            manifest("[app]\nsdk = \"C\"\nbuild_directory = \"./\"\ndevices = [\"stax\"]\n");

        let first = App::from_manifest("/one/app-x", &manifest, None);
        let second = App::from_manifest("/two/app-x", &manifest, None);

        assert_eq!(first.folder_name(), second.folder_name());
        assert_ne!(first.folder, second.folder);
    }
}
