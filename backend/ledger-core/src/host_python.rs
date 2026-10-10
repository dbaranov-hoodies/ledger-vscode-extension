//! The Python environment on the host that some actions need. Each project has its own, so that
//! what one app needs does not disturb another.

use std::path::{Path, PathBuf};

use crate::command::Command;
use crate::platform::Platform;
use crate::task::Step;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPython {
    pub platform: Platform,
    /// The directory of the environment. It is created when it is missing.
    pub venv: PathBuf,
}

impl HostPython {
    /// Makes sure the environment exists and has the packages. Both steps do nothing when that
    /// is already so.
    pub(crate) fn environment(&self, packages: &[&str]) -> Vec<Step> {
        let create = Command::new(self.system_python())
            .args(["-m", "venv"])
            .arg(path_text(&self.venv));
        let install = Command::new(self.python())
            .args(["-m", "pip", "install", "--disable-pip-version-check"])
            .args(packages.iter().copied());
        vec![Step::on_host(create), Step::on_host(install)]
    }

    /// The Python of the environment.
    pub(crate) fn python(&self) -> String {
        let python = match self.platform {
            Platform::Windows => self.venv.join("Scripts").join("python.exe"),
            Platform::Linux | Platform::MacOs => self.venv.join("bin").join("python"),
        };
        path_text(&python)
    }

    /// The Python that creates the environment.
    pub(crate) fn system_python(&self) -> &'static str {
        match self.platform {
            Platform::Windows => "python",
            Platform::Linux | Platform::MacOs => "python3",
        }
    }
}

pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(platform: Platform) -> HostPython {
        HostPython {
            platform,
            venv: "/home/u/venvs/app-x".into(),
        }
    }

    fn commands(steps: Vec<Step>) -> Vec<Command> {
        steps
            .into_iter()
            .map(|step| match step {
                Step::OnHost(host) => host.command,
                Step::InContainer(_) => panic!("a step on the host"),
            })
            .collect()
    }

    #[test]
    fn the_environment_is_created_and_given_its_packages() {
        let steps = commands(host(Platform::MacOs).environment(&["ledgerblue", "GitPython"]));

        assert_eq!(
            steps,
            [
                Command::new("python3")
                    .args(["-m", "venv"])
                    .arg("/home/u/venvs/app-x"),
                Command::new("/home/u/venvs/app-x/bin/python")
                    .args(["-m", "pip", "install", "--disable-pip-version-check"])
                    .args(["ledgerblue", "GitPython"]),
            ]
        );
    }

    #[test]
    fn on_windows_python_is_called_python_and_lives_in_scripts() {
        let windows = host(Platform::Windows);

        assert_eq!(windows.system_python(), "python");
        let expected = PathBuf::from("/home/u/venvs/app-x")
            .join("Scripts")
            .join("python.exe");
        assert_eq!(windows.python(), path_text(&expected));
    }
}
