use std::fmt;
use std::time::Duration;

/// A program and its arguments, run without a shell: nothing here is ever re-parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    /// The command is given up on after this long. `None` lets it run as long as it needs.
    pub timeout: Option<Duration>,
    /// What to show instead of the whole command line, when that would be unreadable.
    pub label: Option<String>,
}

impl Command {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            timeout: None,
            label: None,
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The whole command, written the way a shell would need it.
    pub fn shell_line(&self) -> String {
        let words = std::iter::once(&self.program).chain(&self.args);
        shell_words::join(words)
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
}

/// For logs and messages: the label when there is one, else the whole command line.
impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.label {
            Some(label) => f.write_str(label),
            None => f.write_str(&self.shell_line()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_program_with_its_arguments() {
        let command = Command::new("docker").arg("run").args(["-t", "-d"]);

        assert_eq!(command.program, "docker");
        assert_eq!(command.args, ["run", "-t", "-d"]);
    }

    #[test]
    fn display_quotes_what_a_shell_would_need_quoted() {
        let command = Command::new("docker")
            .arg("run")
            .arg("my image")
            .arg("it's");

        assert_eq!(command.to_string(), r"docker run 'my image' 'it'\''s'");
    }

    #[test]
    fn a_label_replaces_the_command_line_in_messages_but_not_in_the_shell_line() {
        let command = Command::new("docker").arg("exec").label("make -j");

        assert_eq!(command.to_string(), "make -j");
        assert_eq!(command.shell_line(), "docker exec");
    }
}
