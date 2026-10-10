//! Functional tests as a tree (directories, files, classes, tests) and a selection of them.
//!
//! Every node id is a pytest node id, so any node, not only a test, can be handed to pytest.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Directory,
    File,
    Class,
    Test,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestNode {
    pub kind: NodeKind,
    /// Last segment of the id: `tests`, `test_app.py`, `TestApp` or `test_menu`.
    pub name: String,
    /// pytest node id, such as `tests/test_app.py::TestApp::test_menu`.
    pub id: String,
    pub children: Vec<TestNode>,
}

impl TestNode {
    /// The tests at or below this node.
    pub fn tests(&self) -> Vec<&TestNode> {
        if self.kind == NodeKind::Test {
            return vec![self];
        }
        self.children.iter().flat_map(TestNode::tests).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestTree {
    roots: Vec<TestNode>,
}

impl TestTree {
    /// Builds the tree from the output of `pytest --collect-only -q`, in collection order.
    /// Lines that are not test ids are ignored, and parametrization (`[nanox]`) is dropped, so
    /// a test appears once and selecting it runs all its variants.
    pub fn from_collect_output(output: &str) -> Self {
        let mut tree = Self::default();
        for steps in output.lines().filter_map(steps_of) {
            tree.insert(steps);
        }
        tree
    }

    pub fn roots(&self) -> &[TestNode] {
        &self.roots
    }

    pub fn tests(&self) -> Vec<&TestNode> {
        self.roots.iter().flat_map(TestNode::tests).collect()
    }

    fn insert(&mut self, steps: Vec<Step>) {
        let mut level = &mut self.roots;
        for step in steps {
            let index = match level
                .iter()
                .position(|node| node.kind == step.kind && node.name == step.name)
            {
                Some(index) => index,
                None => {
                    level.push(TestNode {
                        kind: step.kind,
                        name: step.name,
                        id: step.id,
                        children: Vec::new(),
                    });
                    level.len() - 1
                }
            };
            level = &mut level[index].children;
        }
    }
}

struct Step {
    kind: NodeKind,
    name: String,
    id: String,
}

/// The nodes from the top directory down to the test of one line of collection output.
fn steps_of(line: &str) -> Option<Vec<Step>> {
    let line = line.trim();
    let id = line.split_once('[').map_or(line, |(id, _params)| id);
    let mut segments = id.split("::");
    let path = segments.next()?;
    let inner: Vec<&str> = segments.collect();
    if inner.is_empty() || inner.contains(&"") {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let (file, directories) = parts.split_last()?;

    let mut steps = Vec::new();
    let mut id = String::new();
    for directory in directories {
        append(&mut id, "/", directory);
        steps.push(Step {
            kind: NodeKind::Directory,
            name: (*directory).to_owned(),
            id: id.clone(),
        });
    }
    append(&mut id, "/", file);
    steps.push(Step {
        kind: NodeKind::File,
        name: (*file).to_owned(),
        id: id.clone(),
    });
    for (position, name) in inner.iter().enumerate() {
        append(&mut id, "::", name);
        let kind = if position + 1 == inner.len() {
            NodeKind::Test
        } else {
            NodeKind::Class
        };
        steps.push(Step {
            kind,
            name: (*name).to_owned(),
            id: id.clone(),
        });
    }
    Some(steps)
}

fn append(id: &mut String, separator: &str, segment: &str) {
    if !id.is_empty() {
        id.push_str(separator);
    }
    id.push_str(segment);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Unchecked,
    /// Some of the tests below the node are selected.
    Partial,
    Checked,
}

/// The tests chosen to run. Choosing a directory, file or class chooses every test below it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestSelection {
    selected: BTreeSet<String>,
}

impl TestSelection {
    pub fn all(tree: &TestTree) -> Self {
        Self {
            selected: tree.tests().iter().map(|test| test.id.clone()).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn state(&self, node: &TestNode) -> CheckState {
        let tests = node.tests();
        let chosen = tests
            .iter()
            .filter(|test| self.selected.contains(&test.id))
            .count();
        match chosen {
            0 => CheckState::Unchecked,
            count if count == tests.len() => CheckState::Checked,
            _ => CheckState::Partial,
        }
    }

    /// Selects or deselects every test at or below the node.
    pub fn set(&mut self, node: &TestNode, selected: bool) {
        for test in node.tests() {
            if selected {
                self.selected.insert(test.id.clone());
            } else {
                self.selected.remove(&test.id);
            }
        }
    }

    /// Checks everything below the node, or unchecks it when it is already fully checked.
    pub fn toggle(&mut self, node: &TestNode) {
        let check = self.state(node) != CheckState::Checked;
        self.set(node, check);
    }

    /// Forgets the tests that a new collection no longer lists.
    pub fn retain_existing(&mut self, tree: &TestTree) {
        let existing: BTreeSet<&str> = tree.tests().iter().map(|test| test.id.as_str()).collect();
        self.selected.retain(|id| existing.contains(id.as_str()));
    }

    /// pytest arguments that select exactly these tests, each as the widest node that is fully
    /// selected (a whole directory, file or class when possible).
    pub fn pytest_args(&self, tree: &TestTree) -> Vec<String> {
        let mut args = Vec::new();
        self.collect_args(tree.roots(), &mut args);
        args
    }

    fn collect_args(&self, nodes: &[TestNode], args: &mut Vec<String>) {
        for node in nodes {
            match self.state(node) {
                CheckState::Checked => args.push(node.id.clone()),
                CheckState::Partial => self.collect_args(&node.children, args),
                CheckState::Unchecked => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\
test_app_mainmenu.py::test_app_mainmenu[nanox]
test_app_mainmenu.py::test_app_mainmenu[stax]
test_appname_cmd.py::test_app_name
sub/test_deep.py::TestFoo::test_a[nanox]
sub/test_deep.py::TestFoo::test_b
sub/test_deep.py::test_c

5 tests collected in 0.02s
";

    fn tree() -> TestTree {
        TestTree::from_collect_output(OUTPUT)
    }

    fn node<'a>(tree: &'a TestTree, id: &str) -> &'a TestNode {
        fn find<'a>(nodes: &'a [TestNode], id: &str) -> Option<&'a TestNode> {
            nodes.iter().find_map(|node| {
                (node.id == id)
                    .then_some(node)
                    .or_else(|| find(&node.children, id))
            })
        }
        find(tree.roots(), id).expect("node should exist")
    }

    fn names(nodes: &[TestNode]) -> Vec<&str> {
        nodes.iter().map(|node| node.name.as_str()).collect()
    }

    #[test]
    fn builds_directories_files_classes_and_tests_in_collection_order() {
        let tree = tree();

        assert_eq!(
            names(tree.roots()),
            ["test_app_mainmenu.py", "test_appname_cmd.py", "sub"]
        );
        let deep = node(&tree, "sub/test_deep.py");
        assert_eq!(deep.kind, NodeKind::File);
        assert_eq!(names(&deep.children), ["TestFoo", "test_c"]);
        let class = node(&tree, "sub/test_deep.py::TestFoo");
        assert_eq!(class.kind, NodeKind::Class);
        assert_eq!(names(&class.children), ["test_a", "test_b"]);
        assert_eq!(node(&tree, "sub").kind, NodeKind::Directory);
    }

    #[test]
    fn parametrized_tests_appear_once() {
        let ids: Vec<_> = tree().tests().iter().map(|test| test.id.clone()).collect();

        assert_eq!(
            ids,
            [
                "test_app_mainmenu.py::test_app_mainmenu",
                "test_appname_cmd.py::test_app_name",
                "sub/test_deep.py::TestFoo::test_a",
                "sub/test_deep.py::TestFoo::test_b",
                "sub/test_deep.py::test_c",
            ]
        );
    }

    #[test]
    fn a_parameter_may_contain_the_separator() {
        let tree = TestTree::from_collect_output("t.py::test_x[a::b]\n");

        assert_eq!(tree.tests()[0].id, "t.py::test_x");
    }

    #[test]
    fn a_leading_dot_slash_is_dropped_from_ids() {
        let tree = TestTree::from_collect_output("./tests/test_a.py::test_x\n");

        assert_eq!(tree.tests()[0].id, "tests/test_a.py::test_x");
        assert_eq!(node(&tree, "tests").kind, NodeKind::Directory);
    }

    #[test]
    fn lines_that_are_not_test_ids_are_ignored() {
        let tree = TestTree::from_collect_output("\nno tests collected\nt.py::\n::test\n");

        assert!(tree.tests().is_empty());
        assert!(tree.roots().is_empty());
    }

    #[test]
    fn a_node_is_partial_when_only_some_of_its_tests_are_selected() {
        let tree = tree();
        let mut selection = TestSelection::default();
        selection.toggle(node(&tree, "sub/test_deep.py::TestFoo::test_a"));

        assert_eq!(
            selection.state(node(&tree, "sub/test_deep.py::TestFoo")),
            CheckState::Partial
        );
        assert_eq!(selection.state(node(&tree, "sub")), CheckState::Partial);
        assert_eq!(
            selection.state(node(&tree, "test_appname_cmd.py")),
            CheckState::Unchecked
        );
    }

    #[test]
    fn toggling_a_partial_node_checks_everything_and_toggling_again_clears_it() {
        let tree = tree();
        let mut selection = TestSelection::default();
        let class = node(&tree, "sub/test_deep.py::TestFoo");
        selection.toggle(node(&tree, "sub/test_deep.py::TestFoo::test_a"));

        selection.toggle(class);
        assert_eq!(selection.state(class), CheckState::Checked);

        selection.toggle(class);
        assert_eq!(selection.state(class), CheckState::Unchecked);
        assert!(selection.is_empty());
    }

    #[test]
    fn a_fully_selected_directory_is_passed_to_pytest_as_one_argument() {
        let tree = tree();
        let mut selection = TestSelection::default();
        selection.set(node(&tree, "sub"), true);

        assert_eq!(selection.pytest_args(&tree), ["sub"]);
    }

    #[test]
    fn a_single_test_is_passed_by_its_own_id() {
        let tree = tree();
        let mut selection = TestSelection::default();
        selection.set(node(&tree, "sub/test_deep.py::TestFoo::test_a"), true);

        assert_eq!(
            selection.pytest_args(&tree),
            ["sub/test_deep.py::TestFoo::test_a"]
        );
    }

    #[test]
    fn arguments_use_the_widest_fully_selected_node() {
        let tree = tree();
        let mut selection = TestSelection::default();
        selection.set(node(&tree, "test_appname_cmd.py::test_app_name"), true);
        selection.set(node(&tree, "sub/test_deep.py::TestFoo"), true);

        assert_eq!(
            selection.pytest_args(&tree),
            ["test_appname_cmd.py", "sub/test_deep.py::TestFoo"]
        );
    }

    #[test]
    fn selecting_everything_passes_every_top_level_node() {
        let tree = tree();

        assert_eq!(
            TestSelection::all(&tree).pytest_args(&tree),
            ["test_app_mainmenu.py", "test_appname_cmd.py", "sub"]
        );
        assert!(TestSelection::default().pytest_args(&tree).is_empty());
    }

    #[test]
    fn a_new_collection_drops_tests_that_no_longer_exist() {
        let before = tree();
        let after = TestTree::from_collect_output("test_appname_cmd.py::test_app_name\n");
        let mut selection = TestSelection::all(&before);

        selection.retain_existing(&after);

        assert_eq!(selection, TestSelection::all(&after));
    }
}
