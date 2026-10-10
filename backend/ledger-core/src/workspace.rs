//! The apps of the workspace folders, identified by the path of their folder, and what the user
//! chose for each.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::c_app::MakefileInfo;
use crate::device::Device;
use crate::selection::{AppSelection, SavedSelection};

#[derive(Debug, Default)]
pub struct Workspace {
    entries: Vec<Entry>,
    current: Option<PathBuf>,
}

#[derive(Debug)]
struct Entry {
    app: App,
    selection: AppSelection,
}

/// The workspace choices as they are stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    /// The folder of the selected app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<PathBuf>,
    #[serde(default)]
    pub apps: BTreeMap<PathBuf, SavedSelection>,
}

impl Workspace {
    /// Takes the apps just detected. Known apps keep their choices and Makefile info, which the
    /// caller should ask again. The selected app stays while it exists, else the saved one, else the first.
    pub fn refresh(
        &mut self,
        detected: Vec<App>,
        saved: &SavedWorkspace,
        default_device: Option<Device>,
    ) {
        let mut known: BTreeMap<PathBuf, Entry> = std::mem::take(&mut self.entries)
            .into_iter()
            .map(|entry| (entry.app.folder.clone(), entry))
            .collect();
        let nothing_saved = SavedSelection::default();

        self.entries = detected
            .into_iter()
            .filter_map(|mut app| match known.remove(&app.folder) {
                Some(before) => {
                    app.makefile = app.makefile.or(before.app.makefile);
                    let mut selection = before.selection;
                    selection.reconcile(&app);
                    Some(Entry { app, selection })
                }
                None => {
                    let saved = saved.apps.get(&app.folder).unwrap_or(&nothing_saved);
                    AppSelection::new(&app, saved, default_device)
                        .map(|selection| Entry { app, selection })
                }
            })
            .collect();

        let exists =
            |folder: &PathBuf| self.entries.iter().any(|entry| &entry.app.folder == folder);
        let first = self.entries.first().map(|entry| entry.app.folder.clone());
        self.current = self
            .current
            .take()
            .filter(exists)
            .or_else(|| saved.app.clone().filter(exists))
            .or(first);
    }

    pub fn apps(&self) -> impl Iterator<Item = (&App, &AppSelection)> {
        self.entries
            .iter()
            .map(|entry| (&entry.app, &entry.selection))
    }

    /// The selected app. `None` when the workspace has no app.
    pub fn current(&self) -> Option<(&App, &AppSelection)> {
        let entry = &self.entries[self.current_index()?];
        Some((&entry.app, &entry.selection))
    }

    /// The selected app, with its choices to change.
    pub fn current_mut(&mut self) -> Option<(&App, &mut AppSelection)> {
        let index = self.current_index()?;
        let entry = &mut self.entries[index];
        Some((&entry.app, &mut entry.selection))
    }

    /// An app of the workspace, with its choices to change.
    pub fn app_mut(&mut self, folder: &Path) -> Option<(&App, &mut AppSelection)> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.app.folder == folder)?;
        Some((&entry.app, &mut entry.selection))
    }

    fn current_index(&self) -> Option<usize> {
        let current = self.current.as_ref()?;
        self.entries
            .iter()
            .position(|entry| &entry.app.folder == current)
    }

    /// Returns whether the workspace has an app in this folder.
    pub fn select(&mut self, folder: &Path) -> bool {
        let known = self.entries.iter().any(|entry| entry.app.folder == folder);
        if known {
            self.current = Some(folder.to_path_buf());
        }
        known
    }

    /// Records what the Makefile of the C app in `folder` printed, and checks the choices
    /// against it. Returns whether the workspace has an app in this folder.
    pub fn set_makefile_info(&mut self, folder: &Path, info: MakefileInfo) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.app.folder == folder)
        else {
            return false;
        };
        entry.app.makefile = Some(info);
        entry.selection.reconcile(&entry.app);
        true
    }

    pub fn saved(&self) -> SavedWorkspace {
        SavedWorkspace {
            app: self.current.clone(),
            apps: self
                .entries
                .iter()
                .map(|entry| (entry.app.folder.clone(), entry.selection.saved(&entry.app)))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c_app::Variants;
    use crate::manifest::{self, ParsedManifest};
    use crate::selection::SavedTarget;
    use crate::target::Target;

    fn app(folder: &str, devices: &str) -> App {
        let source = format!(
            "[app]\nsdk = \"c\"\nbuild_directory = \"./\"\ndevices = [{devices}]\n[use_cases]\ndebug = \"DEBUG=1\"\n"
        );
        let ParsedManifest::Current(manifest) = manifest::parse(&source).unwrap() else {
            panic!("a current manifest");
        };
        App::from_manifest(folder, &manifest, None)
    }

    fn workspace(apps: Vec<App>) -> Workspace {
        let mut workspace = Workspace::default();
        workspace.refresh(apps, &SavedWorkspace::default(), None);
        workspace
    }

    fn two_apps() -> Vec<App> {
        vec![
            app("/work/app-a", "\"stax\", \"flex\""),
            app("/work/app-b", "\"nanox\", \"stax\""),
        ]
    }

    fn current_folder(workspace: &Workspace) -> Option<&Path> {
        workspace.current().map(|(app, _)| app.folder.as_path())
    }

    #[test]
    fn the_first_app_is_selected_at_first() {
        let workspace = workspace(two_apps());

        assert_eq!(current_folder(&workspace), Some(Path::new("/work/app-a")));
        assert_eq!(workspace.apps().count(), 2);
    }

    #[test]
    fn a_workspace_without_apps_has_no_current_app() {
        let mut workspace = workspace(Vec::new());

        assert_eq!(current_folder(&workspace), None);
        assert!(workspace.current_mut().is_none());
        assert!(!workspace.select(Path::new("/work/app-a")));
    }

    #[test]
    fn detecting_the_apps_again_keeps_the_selected_app() {
        let mut workspace = workspace(two_apps());
        assert!(workspace.select(Path::new("/work/app-b")));

        workspace.refresh(two_apps(), &SavedWorkspace::default(), None);

        assert_eq!(current_folder(&workspace), Some(Path::new("/work/app-b")));
    }

    #[test]
    fn the_selected_app_stays_selected_when_its_manifest_changed() {
        let mut workspace = workspace(two_apps());
        workspace.select(Path::new("/work/app-b"));
        let changed = vec![
            app("/work/app-a", "\"stax\", \"flex\""),
            app("/work/app-b", "\"nanox\", \"stax\", \"flex\""),
        ];

        workspace.refresh(changed, &SavedWorkspace::default(), None);

        let (app, _) = workspace.current().unwrap();
        assert_eq!(app.folder, Path::new("/work/app-b"));
        assert_eq!(app.devices.len(), 3);
    }

    #[test]
    fn the_choices_survive_detecting_the_apps_again() {
        let mut workspace = workspace(two_apps());
        workspace.select(Path::new("/work/app-b"));
        let (app, selection) = workspace.current_mut().unwrap();
        selection.set_target(&Target::one(Device::Stax), app);
        selection.set_build_use_case("debug", app);

        workspace.refresh(two_apps(), &SavedWorkspace::default(), None);

        let (_, selection) = workspace.current().unwrap();
        assert_eq!(selection.target(), &Target::one(Device::Stax));
        assert_eq!(selection.build_use_case(), Some("debug"));
    }

    #[test]
    fn a_choice_the_app_no_longer_offers_gives_way_to_the_default() {
        let mut workspace = workspace(two_apps());
        let (app, selection) = workspace.current_mut().unwrap();
        selection.set_target(&Target::one(Device::Flex), app);

        workspace.refresh(vec![app_without_flex()], &SavedWorkspace::default(), None);

        let (_, selection) = workspace.current().unwrap();
        assert_eq!(selection.target(), &Target::one(Device::Stax));
    }

    fn app_without_flex() -> App {
        app("/work/app-a", "\"stax\"")
    }

    #[test]
    fn closing_the_selected_folder_selects_the_first_app() {
        let mut workspace = workspace(two_apps());
        workspace.select(Path::new("/work/app-b"));

        workspace.refresh(
            vec![app("/work/app-a", "\"stax\"")],
            &SavedWorkspace::default(),
            None,
        );

        assert_eq!(current_folder(&workspace), Some(Path::new("/work/app-a")));
    }

    #[test]
    fn losing_every_app_leaves_none_selected() {
        let mut workspace = workspace(two_apps());

        workspace.refresh(Vec::new(), &SavedWorkspace::default(), None);

        assert_eq!(current_folder(&workspace), None);
    }

    #[test]
    fn folders_with_the_same_name_are_told_apart_by_their_path() {
        let mut workspace = workspace(vec![
            app("/one/app-x", "\"stax\""),
            app("/two/app-x", "\"flex\""),
        ]);

        assert!(workspace.select(Path::new("/two/app-x")));

        let (app, selection) = workspace.current().unwrap();
        assert_eq!(app.folder, Path::new("/two/app-x"));
        assert_eq!(selection.target(), &Target::one(Device::Flex));
    }

    #[test]
    fn an_unknown_folder_cannot_be_selected() {
        let mut workspace = workspace(two_apps());

        assert!(!workspace.select(Path::new("/work/app-c")));

        assert_eq!(current_folder(&workspace), Some(Path::new("/work/app-a")));
    }

    #[test]
    fn a_new_app_starts_from_what_was_saved() {
        let saved = SavedWorkspace {
            app: Some("/work/app-b".into()),
            apps: BTreeMap::from([(
                PathBuf::from("/work/app-b"),
                SavedSelection {
                    target: Some(SavedTarget::All),
                    build_use_case: Some("debug".to_owned()),
                    ..SavedSelection::default()
                },
            )]),
        };
        let mut workspace = Workspace::default();

        workspace.refresh(two_apps(), &saved, None);

        let (app, selection) = workspace.current().unwrap();
        assert_eq!(app.folder, Path::new("/work/app-b"));
        assert_eq!(selection.target(), &Target::all(&app.devices).unwrap());
        assert_eq!(selection.build_use_case(), Some("debug"));
    }

    #[test]
    fn a_saved_app_that_does_not_exist_any_more_is_not_selected() {
        let saved = SavedWorkspace {
            app: Some("/work/gone".into()),
            apps: BTreeMap::new(),
        };
        let mut workspace = Workspace::default();

        workspace.refresh(two_apps(), &saved, None);

        assert_eq!(current_folder(&workspace), Some(Path::new("/work/app-a")));
    }

    #[test]
    fn the_default_device_applies_to_apps_that_support_it_and_the_others_take_their_first() {
        let mut workspace = Workspace::default();

        workspace.refresh(two_apps(), &SavedWorkspace::default(), Some(Device::Flex));

        let targets: Vec<_> = workspace
            .apps()
            .map(|(_, selection)| selection.target().clone())
            .collect();
        assert_eq!(
            targets,
            [Target::one(Device::Flex), Target::one(Device::NanoX)]
        );
    }

    fn info(variants: &[&str]) -> MakefileInfo {
        MakefileInfo {
            name: "A".to_owned(),
            variants: Some(Variants {
                name: "COIN".to_owned(),
                values: variants.iter().map(|value| (*value).to_owned()).collect(),
            }),
        }
    }

    #[test]
    fn the_makefile_info_chooses_the_default_variant() {
        let mut workspace = workspace(two_apps());

        assert!(workspace.set_makefile_info(Path::new("/work/app-a"), info(&["BTC", "LTC"])));

        let (app, selection) = workspace.current().unwrap();
        assert_eq!(app.name(), Some("A"));
        assert_eq!(selection.variant(), Some("BTC"));
    }

    #[test]
    fn the_makefile_info_of_an_unknown_folder_is_refused() {
        let mut workspace = workspace(two_apps());

        assert!(!workspace.set_makefile_info(Path::new("/work/app-c"), info(&["BTC"])));
    }

    #[test]
    fn detecting_the_apps_again_keeps_the_makefile_info_until_it_is_asked_again() {
        let mut workspace = workspace(two_apps());
        workspace.set_makefile_info(Path::new("/work/app-a"), info(&["BTC", "LTC"]));
        let (app, selection) = workspace.current_mut().unwrap();
        selection.set_variant("LTC", app);

        workspace.refresh(two_apps(), &SavedWorkspace::default(), None);

        let (app, selection) = workspace.current().unwrap();
        assert_eq!(app.name(), Some("A"));
        assert_eq!(selection.variant(), Some("LTC"));
    }

    #[test]
    fn the_saved_workspace_restores_the_same_choices() {
        let mut first = workspace(two_apps());
        first.select(Path::new("/work/app-b"));
        let (app, selection) = first.current_mut().unwrap();
        selection.set_target(&Target::all(&app.devices).unwrap(), app);
        selection.set_build_use_case("debug", app);

        let json = serde_json::to_string(&first.saved()).unwrap();
        let saved: SavedWorkspace = serde_json::from_str(&json).unwrap();
        let mut second = Workspace::default();
        second.refresh(two_apps(), &saved, None);

        assert_eq!(second.saved(), first.saved());
        assert_eq!(current_folder(&second), Some(Path::new("/work/app-b")));
    }
}
