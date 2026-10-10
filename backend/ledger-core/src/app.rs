//! The Ledger app being built, as far as running its tasks needs to know.

use crate::container;
use crate::device::Device;
use crate::language::Language;
use crate::manifest::Manifest;
use crate::rust_app::RustAppPackage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub folder_name: String,
    pub language: Language,
    /// As written in the manifest, relative to the app folder: `./`, `./app`, …
    pub build_directory: String,
    /// The devices the app supports, in manifest order.
    pub devices: Vec<Device>,
    /// The names a Rust app's `Cargo.toml` declares.
    pub rust: Option<RustAppPackage>,
}

impl App {
    pub fn from_manifest(
        folder_name: impl Into<String>,
        manifest: &Manifest,
        rust: Option<RustAppPackage>,
    ) -> Self {
        Self {
            folder_name: folder_name.into(),
            language: manifest.language,
            build_directory: manifest.build_directory.clone(),
            devices: manifest.devices.clone(),
            rust,
        }
    }

    pub fn container_name(&self, device: Device) -> String {
        container::container_name(&self.folder_name, device)
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

    #[test]
    fn an_app_takes_its_facts_from_the_manifest() {
        let source =
            "[app]\nsdk = \"C\"\nbuild_directory = \"./app\"\ndevices = [\"stax\", \"flex\"]\n";
        let ParsedManifest::Current(manifest) = manifest::parse(source).unwrap() else {
            panic!("a current manifest");
        };

        let app = App::from_manifest("app-x", &manifest, None);

        assert_eq!(app.language, Language::C);
        assert_eq!(app.devices, [Device::Stax, Device::Flex]);
        assert_eq!(app.container_name(Device::Flex), "app-x-container-flex");
        assert_eq!(app.in_build_directory("build/stax"), "./app/build/stax");
    }
}
