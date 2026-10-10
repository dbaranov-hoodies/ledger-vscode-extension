//! The udev rules that let a Linux host talk to a Ledger device. Fetching the rules and running
//! the commands needs the network and the user's password, so the caller does both.

use std::path::Path;

use crate::command::Command;

pub const RULES_URL: &str =
    "https://raw.githubusercontent.com/LedgerHQ/udev-rules/master/20-hw1.rules";
const RULES_DIR: &str = "/etc/udev/rules.d/";

/// Where the rules are installed, to read what is there.
pub fn installed_rules_path() -> String {
    let name = RULES_URL.rsplit('/').next().unwrap_or_default();
    format!("{RULES_DIR}{name}")
}

/// Whether the rules are missing or differ from the latest ones.
pub fn needs_install(installed: Option<&str>, latest: &str) -> bool {
    installed != Some(latest)
}

/// Installs the rules that were downloaded to `downloaded`. It needs `sudo`, so it has to run
/// where the user can type a password: one command after the other, stopping at the first failure.
pub fn install_commands(downloaded: &Path) -> Vec<Command> {
    let sudo = |args: &[&str]| Command::new("sudo").args(args.iter().copied());
    vec![
        sudo(&["mv", &downloaded.to_string_lossy(), RULES_DIR]),
        sudo(&["udevadm", "control", "--reload-rules"]),
        sudo(&["udevadm", "trigger"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_are_installed_under_their_own_name() {
        assert_eq!(installed_rules_path(), "/etc/udev/rules.d/20-hw1.rules");
    }

    #[test]
    fn missing_or_different_rules_need_installing() {
        assert!(needs_install(None, "rules"));
        assert!(needs_install(Some("old rules"), "rules"));
        assert!(!needs_install(Some("rules"), "rules"));
    }

    #[test]
    fn installing_moves_the_file_and_reloads_the_rules() {
        let lines: Vec<String> = install_commands(Path::new("/tmp/20-hw1.rules"))
            .iter()
            .map(Command::shell_line)
            .collect();

        assert_eq!(
            lines,
            [
                "sudo mv /tmp/20-hw1.rules /etc/udev/rules.d/",
                "sudo udevadm control --reload-rules",
                "sudo udevadm trigger",
            ]
        );
    }
}
