//! Where a test is written: the file it is in, and the line where it starts.

use crate::test_tree::path_from_app;

/// The files that the node with this pytest id may be in, relative to the app folder, the most
/// likely first. pytest ids are relative to its root, which may be the suite or the app.
pub fn file_candidates(suite_directory: &str, id: &str) -> Vec<String> {
    let path = id.split("::").next().unwrap_or(id);
    let from_app = path_from_app(suite_directory, path);
    if from_app == path {
        vec![from_app]
    } else {
        vec![from_app, path.to_owned()]
    }
}

/// The line, counting from 1, where the node with this pytest id starts in the source of its
/// file: the `class` or `def` line of its last name, or 1 for a file or when it is not found.
pub fn line_of(source: &str, id: &str) -> usize {
    let names: Vec<&str> = id.split("::").skip(1).collect();
    let Some(last) = names.last() else { return 1 };
    let lines: Vec<&str> = source.lines().collect();
    let found = find_in_scope(&lines, &names).or_else(|| {
        lines
            .iter()
            .position(|line| definition(line).is_some_and(|(_, name)| name == *last))
    });
    found.map_or(1, |at| at + 1)
}

/// The index of the line that defines the last name, looking for each name inside the block of
/// the one before: a class, then a method of it.
fn find_in_scope(lines: &[&str], names: &[&str]) -> Option<usize> {
    let mut from = 0;
    let mut parent_indent: Option<usize> = None;
    let mut found = None;
    for name in names {
        let (at, indent) = find_definition(lines, from, parent_indent, name)?;
        found = Some(at);
        from = at + 1;
        parent_indent = Some(indent);
    }
    found
}

/// The first definition of `name` from this line on, inside the block that is indented deeper
/// than `parent_indent` (at the top of the file when there is none).
fn find_definition(
    lines: &[&str],
    from: usize,
    parent_indent: Option<usize>,
    name: &str,
) -> Option<(usize, usize)> {
    for (at, line) in lines.iter().enumerate().skip(from) {
        let text = line.trim_start();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let indent = line.len() - text.len();
        let inside = parent_indent.map_or(indent == 0, |parent| indent > parent);
        if inside && definition(line).is_some_and(|(_, defined)| defined == name) {
            return Some((at, indent));
        }
        if parent_indent.is_some_and(|parent| indent <= parent) {
            return None;
        }
    }
    None
}

/// The indentation and name of a `def`, `async def` or `class` line.
fn definition(line: &str) -> Option<(usize, &str)> {
    let text = line.trim_start();
    let rest = ["async def ", "def ", "class "]
        .iter()
        .find_map(|keyword| text.strip_prefix(keyword))?
        .trim_start();
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| (line.len() - text.len(), &rest[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "\
import pytest


def helper():
    pass


@pytest.mark.parametrize(\"n\", [1, 2])
def test_one(n):
    pass


class TestFoo:
    def test_two(self):
        pass

    # def test_commented(self):
    async def test_three(self):
        pass

    class TestInner:
        def test_two(self):
            pass


class TestBar:
    def test_two(self):
        pass


def test_one_more():
    pass
";

    fn line(id: &str) -> usize {
        line_of(SOURCE, id)
    }

    #[test]
    fn a_function_is_found_at_its_def_line_not_at_its_decorator() {
        assert_eq!(line("t.py::test_one"), 9);
        assert_eq!(line("t.py::test_one_more"), 31);
    }

    #[test]
    fn a_method_is_found_in_its_own_class() {
        assert_eq!(line("t.py::TestFoo::test_two"), 14);
        assert_eq!(line("t.py::TestBar::test_two"), 27);
        assert_eq!(line("t.py::TestFoo::TestInner::test_two"), 22);
    }

    #[test]
    fn async_tests_and_classes_are_found_and_comments_are_not() {
        assert_eq!(line("t.py::TestFoo::test_three"), 18);
        assert_eq!(line("t.py::TestFoo"), 13);
        assert_eq!(line("t.py::TestBar"), 26);
        assert_eq!(
            line("t.py::TestFoo::test_commented"),
            1,
            "only a comment has it"
        );
    }

    #[test]
    fn a_file_starts_at_the_first_line_and_so_does_what_is_not_found() {
        assert_eq!(line("t.py"), 1);
        assert_eq!(line("t.py::test_missing"), 1);
        assert_eq!(
            line("t.py::TestMissing::test_two"),
            14,
            "the first of that name, at least"
        );
    }

    #[test]
    fn a_class_cut_short_by_an_unindented_line_does_not_hide_its_tests() {
        let source = "class TestFoo:\n    def test_a(self):\n        x = \"\"\"\nat the margin\n\"\"\"\n\n    def test_b(self):\n        pass\n";

        assert_eq!(line_of(source, "t.py::TestFoo::test_b"), 7);
    }

    #[test]
    fn the_file_is_looked_for_in_the_app_and_where_pytest_ran() {
        assert_eq!(
            file_candidates("./tests/standalone/", "test_a.py::test_x"),
            ["tests/standalone/test_a.py", "test_a.py"]
        );
        assert_eq!(
            file_candidates(
                "./tests/standalone/",
                "tests/standalone/test_a.py::T::test_x"
            ),
            ["tests/standalone/test_a.py"]
        );
        assert_eq!(
            file_candidates("./tests/standalone/", "sub/test_b.py"),
            ["tests/standalone/sub/test_b.py", "sub/test_b.py"]
        );
        assert_eq!(file_candidates("./", "test_a.py::test_x"), ["test_a.py"]);
    }
}
