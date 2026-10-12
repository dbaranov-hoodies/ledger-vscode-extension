//! The apps a test suite loads next to the app it tests (what Ragger calls sideloaded apps), as
//! its `conftest.py` declares them, and how to prepare them for a suite that ships no script for
//! it. The Exchange tests, for instance, need a dozen coin apps built for every device.

use crate::app::{App, join_path};
use crate::command::Command;
use crate::device::Device;
use crate::host_python::path_text;
use crate::task::{InContainer, OnHost, Step};

/// Where Ragger reads the declaration from.
pub const CONFTEST: &str = "conftest.py";
/// Where the clones and the builds of the apps of a suite are kept, inside its directory.
const DEPENDENCIES_DIR: &str = ".test_dependencies";

/// One app of `SIDELOADED_APPS`. Its binary is `<variant>_<device>.elf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideloadedApp {
    pub variant: String,
    /// The name the app has on the device.
    pub name: String,
}

/// What the `conftest.py` of a suite says about the apps its tests load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sideloaded {
    /// Where the binaries are looked for, relative to the app folder: `test/python/lib_binaries`.
    pub directory: String,
    pub apps: Vec<SideloadedApp>,
}

impl Sideloaded {
    /// The file Ragger looks for, relative to the app folder.
    pub fn binary(&self, app: &SideloadedApp, device: Device) -> String {
        join_path(&self.directory, &binary_name(&app.variant, device))
    }

    /// Whether the extension knows how to build at least one of the apps.
    pub fn is_buildable(&self) -> bool {
        self.apps.iter().any(|app| recipe(&app.variant).is_some())
    }
}

/// `APTOS_nanos2.elf`: the variant, and the directory that the device builds into.
pub fn binary_name(variant: &str, device: Device) -> String {
    format!("{variant}_{}.elf", device.spec().build_dir_name)
}

// ----- reading the conftest

/// Reads `SIDELOADED_APPS` (a dictionary of strings) and `SIDELOADED_APPS_DIR` (a string) out of
/// a `conftest.py`. `None` when it declares no apps, or says it in a way that cannot be read
/// without running Python: the extension then does not guess.
pub fn parse_conftest(source: &str) -> Option<Sideloaded> {
    let directory = assignments(source, "SIDELOADED_APPS_DIR").find_map(|value| {
        let (directory, _) = string_literal(value)?;
        Some(directory)
    })?;
    let apps: Vec<SideloadedApp> = assignments(source, "SIDELOADED_APPS")
        .find_map(dictionary)?
        .into_iter()
        .map(|(variant, name)| SideloadedApp { variant, name })
        .collect();
    if apps.is_empty() {
        return None;
    }
    Some(Sideloaded {
        directory: clean_directory(&directory),
        apps,
    })
}

fn clean_directory(directory: &str) -> String {
    let mut directory = directory.replace('\\', "/");
    while let Some(rest) = directory.strip_prefix("./") {
        directory = rest.to_owned();
    }
    directory.trim_end_matches('/').to_owned()
}

fn is_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// What follows `name =` each time the file assigns it, `name` being a whole identifier on a line
/// that is not a comment.
fn assignments<'a>(source: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    source.match_indices(name).filter_map(move |(start, _)| {
        let before = &source[..start];
        if before
            .chars()
            .next_back()
            .is_some_and(is_identifier_character)
        {
            return None;
        }
        let line = before.rsplit('\n').next().unwrap_or_default();
        if line.contains('#') {
            return None;
        }
        let after = source[start + name.len()..].trim_start_matches([' ', '\t']);
        let value = after.strip_prefix('=')?;
        (!value.starts_with('=')).then_some(value)
    })
}

/// Spaces, new lines and comments.
fn skip_trivia(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        match text.strip_prefix('#') {
            Some(comment) => text = comment.split_once('\n').map_or("", |(_, rest)| rest),
            None => return text,
        }
    }
}

/// A string between quotes at the start of the text, and what comes after it. Prefixes
/// (`r"…"`, `f"…"`) and triple quotes are not strings the extension reads.
fn string_literal(text: &str) -> Option<(String, &str)> {
    let text = skip_trivia(text);
    let quote = text.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let body = &text[1..];
    if let Some(after) = body.strip_prefix(quote) {
        return if after.starts_with(quote) {
            None
        } else {
            Some((String::new(), after))
        };
    }
    let mut value = String::new();
    let mut characters = body.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '\\' => value.push(characters.next()?.1),
            '\n' => return None,
            c if c == quote => return Some((value, &body[index + 1..])),
            c => value.push(c),
        }
    }
    None
}

/// `{"key": "value", …}` at the start of the text.
fn dictionary(text: &str) -> Option<Vec<(String, String)>> {
    let mut rest = skip_trivia(text).strip_prefix('{')?;
    let mut entries = Vec::new();
    loop {
        rest = skip_trivia(rest);
        if rest.starts_with('}') {
            return Some(entries);
        }
        let (key, after) = string_literal(rest)?;
        let after = skip_trivia(after).strip_prefix(':')?;
        let (value, after) = string_literal(after)?;
        entries.push((key, value));
        rest = skip_trivia(after);
        rest = rest.strip_prefix(',').unwrap_or(rest);
    }
}

// ----- how to build them

/// What it takes to build an app that a suite loads. The Exchange suite is built in its CI from
/// these repositories and branches (`reusable_swap_functional_tests.yml`), the keys of which are
/// the variants of its `conftest.py`, apart from the entries that were pointed elsewhere since.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipe {
    /// `Owner/name` on GitHub.
    pub repo: &'static str,
    pub git_ref: &'static str,
}

impl Recipe {
    const fn develop(repo: &'static str) -> Self {
        Self {
            repo,
            git_ref: "develop",
        }
    }

    fn url(&self) -> String {
        format!("https://github.com/{}.git", self.repo)
    }

    /// The name of the clone: the name of the repository.
    fn directory_name(&self) -> &'static str {
        self.repo.rsplit('/').next().unwrap_or(self.repo)
    }
}

/// `None` for an app the extension does not know where to get.
pub fn recipe(variant: &str) -> Option<Recipe> {
    let repo = match variant {
        "stellar" => "LedgerHQ/app-stellar-dev",
        "tezos" => "LedgerHQ/app-tezos-dev",
        "ethereum" | "ethereum_classic" => "LedgerHQ/app-ethereum-dev",
        "litecoin" => "LedgerHQ/app-bitcoin-legacy-dev",
        "bitcoin" => "LedgerHQ/app-bitcoin-dev",
        "DOT" => "LedgerHQ/app-polkadot-dev",
        "tron" => "LedgerHQ/app-tron-dev",
        "ton" => "LedgerHQ/app-ton-dev",
        "near" => "LedgerHQ/app-near-dev",
        "ATOM" => "LedgerHQ/app-cosmos-dev",
        "APTOS" => "LedgerHQ/app-aptos",
        "kaspa" => "LedgerHQ/app-kaspa-dev",
        "celo" => "LedgerHQ/app-celo-spender-dev",
        _ => return None,
    };
    Some(Recipe::develop(repo))
}

/// The variant is given as `COIN` and `CHAIN`, the two names the apps use for it, and the keys of
/// every test are the ones that the tests were recorded with. An app ignores a variable that it
/// does not know.
fn make_flags(variant: &str) -> Vec<String> {
    [
        format!("COIN={variant}"),
        format!("CHAIN={variant}"),
        "CAL_TEST_KEY=1".to_owned(),
        "DOMAIN_NAME_TEST_KEY=1".to_owned(),
        "SET_PLUGIN_TEST_KEY=1".to_owned(),
        "NFT_TEST_KEY=1".to_owned(),
        "TRUSTED_NAME_TEST_KEY=1".to_owned(),
    ]
    .to_vec()
}

/// Builds one app for the device in the container, from its clone, unless its binary is already
/// there and `skip` is 1. Arguments: the clone, the directory of the binaries, the name of the
/// binary, `skip`, and the arguments of `make`. The manifest of the app says where its Makefile
/// is, and a Rust app is not built here.
const BUILD_SCRIPT: &str = r##"
src="$1"; out="$2"; elf="$3"; skip="$4"; shift 4
if [ "$skip" = 1 ] && [ -f "$out/$elf" ]; then
  echo "$elf is already there."
  exit 0
fi
info=$(python3 -c '
import sys, tomllib
try:
    with open(sys.argv[1], "rb") as manifest:
        app = tomllib.load(manifest).get("app", {})
except (OSError, tomllib.TOMLDecodeError):
    app = {}
print(app.get("sdk", "C"))
print(app.get("build_directory", "./"))
' "$src/ledger_app.toml") || exit 1
sdk=$(printf '%s\n' "$info" | sed -n 1p)
dir=$(printf '%s\n' "$info" | sed -n 2p)
case "$sdk" in
  C|c) ;;
  *) echo "$elf: the app in $src is a $sdk app. Build it by hand and put the binary in $out."; exit 1 ;;
esac
build="$src/$dir"
if [ ! -f "$build/Makefile" ]; then
  echo "$elf: there is no Makefile in $build. Was the repository fetched?"
  exit 1
fi
make -C "$build" clean || exit 1
make -C "$build" -j "$@" || exit 1
mkdir -p "$out" || exit 1
cp "$build/bin/app.elf" "$out/$elf"
"##;

/// Keeps what the preparation puts in the suite out of the Git status of the app, without
/// touching a file of the app: a directory with a `.gitignore` that ignores everything.
const IGNORE_SCRIPT: &str = r##"mkdir -p "$1" && printf '*\n' > "$1/.gitignore""##;

/// Lists what is still missing and fails if anything is. Arguments: the number of repositories,
/// then the name and the clone of each (a clone that holds nothing but `.git` was not fetched),
/// then the binaries that the suite asks for.
const CHECK_SCRIPT: &str = r##"
n="$1"; shift
no_source=0; absent=0
while [ "$n" -gt 0 ]; do
  label="$1"; dir="$2"; shift 2; n=$((n - 1))
  if ! ls -A "$dir" 2>/dev/null | grep -qv '^\.git$'; then
    echo "Not fetched: $label"
    no_source=$((no_source + 1))
  fi
done
for file in "$@"; do
  if [ ! -f "$file" ]; then
    echo "Missing: $file"
    absent=$((absent + 1))
  fi
done
if [ "$no_source" = 0 ] && [ "$absent" = 0 ]; then
  echo "Every sideloaded app is in place."
  exit 0
fi
echo "Repositories not fetched: $no_source. Binaries missing: $absent."
exit 1
"##;

/// Clones what the suite needs on the host, where the Git credentials are, then builds it in the
/// container, which has the SDKs, and checks that every binary the suite asks for is there.
/// A repository that cannot be fetched, or an app that cannot be built, does not stop the others:
/// the check at the end lists what is left, and fails.
pub(crate) fn steps(
    app: &App,
    suite_directory: &str,
    sideloaded: &Sideloaded,
    devices: &[Device],
    clean: bool,
) -> Vec<Step> {
    let dependencies = join_path(suite_directory, DEPENDENCIES_DIR);
    let mut steps: Vec<Step> = Vec::new();
    if clean {
        steps.push(InContainer::new(["rm", "-rf", &dependencies]).into());
    }
    steps.push(InContainer::new(["bash", "-c", IGNORE_SCRIPT, "_", &dependencies]).into());

    let buildable: Vec<(&SideloadedApp, Recipe)> = sideloaded
        .apps
        .iter()
        .filter_map(|sideloaded| Some((sideloaded, recipe(&sideloaded.variant)?)))
        .collect();

    // A repository can hold several of the apps, with another variant each: it is cloned once.
    let mut cloned: Vec<Recipe> = Vec::new();
    for (_, recipe) in &buildable {
        if !cloned.contains(recipe) {
            cloned.push(*recipe);
            steps.extend(clone(app, &dependencies, recipe));
        }
    }

    for (sideloaded_app, recipe) in &buildable {
        let source = join_path(&dependencies, recipe.directory_name());
        for device in devices {
            let mut argv = vec![
                "bash".to_owned(),
                "-c".to_owned(),
                BUILD_SCRIPT.to_owned(),
                "_".to_owned(),
                source.clone(),
                sideloaded.directory.clone(),
                binary_name(&sideloaded_app.variant, *device),
                if clean { "0" } else { "1" }.to_owned(),
            ];
            argv.extend(make_flags(&sideloaded_app.variant));
            steps.push(InContainer::new(argv).sdk(*device).keep_going().into());
        }
    }

    for unknown in sideloaded
        .apps
        .iter()
        .filter(|sideloaded| recipe(&sideloaded.variant).is_none())
    {
        let message = format!(
            "The extension does not know where to get {} ({}). Put its binaries in {}.",
            unknown.name, unknown.variant, sideloaded.directory
        );
        steps.push(InContainer::new(["printf", "%s\n", &message]).into());
    }

    let mut check = vec![
        "bash".to_owned(),
        "-c".to_owned(),
        CHECK_SCRIPT.to_owned(),
        "_".to_owned(),
        cloned.len().to_string(),
    ];
    for recipe in &cloned {
        check.push(recipe.repo.to_owned());
        check.push(join_path(&dependencies, recipe.directory_name()));
    }
    for sideloaded_app in &sideloaded.apps {
        for device in devices {
            check.push(sideloaded.binary(sideloaded_app, *device));
        }
    }
    steps.push(InContainer::new(check).into());
    steps
}

/// Fetches one commit instead of the history, and what it needs of its submodules. It does the
/// same again on a clone that exists, which brings it to the latest commit of the branch. None of
/// the steps stops the job, a repository the user cannot reach included: the others are still
/// fetched, and the check at the end says which was not.
fn clone(app: &App, dependencies: &str, recipe: &Recipe) -> Vec<Step> {
    let directory = join_path(dependencies, recipe.directory_name());
    let path = path_text(&app.folder.join(directory.trim_start_matches("./")));
    let on_host = |command: Command| Step::OnHost(OnHost::new(command).keep_going());
    let git = |arguments: &[&str]| {
        on_host(
            Command::new("git")
                .args(["-C", path.as_str()])
                .args(arguments.iter().copied()),
        )
    };
    vec![
        on_host(Command::new("git").args(["init", "--quiet", path.as_str()])),
        git(&["fetch", "--depth", "1", &recipe.url(), recipe.git_ref]),
        git(&["checkout", "--quiet", "--force", "FETCH_HEAD"]),
        git(&[
            "submodule",
            "update",
            "--init",
            "--recursive",
            "--depth",
            "1",
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    /// The part of the Exchange `conftest.py` that is about the apps.
    const EXCHANGE_CONFTEST: &str = r#"
configuration.OPTIONAL.ALLOWED_SETUPS = ["default", "prod_build"]

# --8<-- [start:sideloaded_applications]
def pytest_configure(config):
    current_setup = config.getoption("--setup")
    # We don't need any lib dependency for the prod_build test
    if current_setup == "default":
        # List of sideloaded applications under the format <VARIANT_VALUES>:<APPNAME>
        configuration.OPTIONAL.SIDELOADED_APPS = {
            "APTOS": "Aptos",
            "bitcoin": "Bitcoin",
            "ethereum": "Ethereum",
            "ethereum_classic": "Ethereum Classic",
            "tezos": "Tezos Wallet",
            "litecoin": "Litecoin",
            "stellar": "Stellar",
            "DOT": "Polkadot",
            "tron": "Tron",
            "ton": "TON",
            "ATOM": "Cosmos",
            "near": "NEAR",
            "kaspa": "Kaspa",
            "celo": "Celo",
        }

        configuration.OPTIONAL.SIDELOADED_APPS_DIR = "test/python/lib_binaries/"
# --8<-- [end:sideloaded_applications]
"#;

    fn app() -> App {
        App {
            folder: "/work/app-exchange".into(),
            language: Language::C,
            build_directory: "./".to_owned(),
            devices: vec![Device::NanoSPlus, Device::Flex],
            rust: None,
            build_use_cases: Vec::new(),
            test_suites: Vec::new(),
            makefile: None,
            compose_file: None,
        }
    }

    fn sideloaded(variants: &[&str]) -> Sideloaded {
        Sideloaded {
            directory: "test/python/lib_binaries".to_owned(),
            apps: variants
                .iter()
                .map(|variant| SideloadedApp {
                    variant: (*variant).to_owned(),
                    name: format!("{variant} app"),
                })
                .collect(),
        }
    }

    fn lines(steps: &[Step]) -> Vec<String> {
        steps
            .iter()
            .map(|step| match step {
                Step::OnHost(host) => format!("host: {}", host.command.shell_line()),
                Step::InContainer(container) => {
                    let sdk = container
                        .sdk
                        .map_or(String::new(), |device| format!(" [{}]", device.spec().name));
                    let argv: Vec<&str> = container
                        .argv
                        .iter()
                        .map(|argument| {
                            // The scripts are long: only say that it is one.
                            if [BUILD_SCRIPT, CHECK_SCRIPT, IGNORE_SCRIPT]
                                .contains(&argument.as_str())
                            {
                                "<script>"
                            } else {
                                argument.as_str()
                            }
                        })
                        .collect();
                    format!("container{sdk}: {}", argv.join(" "))
                }
            })
            .collect()
    }

    // ----- reading the conftest

    #[test]
    fn the_apps_and_their_directory_are_read_from_the_exchange_conftest() {
        let sideloaded = parse_conftest(EXCHANGE_CONFTEST).unwrap();

        assert_eq!(sideloaded.directory, "test/python/lib_binaries");
        let variants: Vec<&str> = sideloaded
            .apps
            .iter()
            .map(|app| app.variant.as_str())
            .collect();
        assert_eq!(
            variants,
            [
                "APTOS",
                "bitcoin",
                "ethereum",
                "ethereum_classic",
                "tezos",
                "litecoin",
                "stellar",
                "DOT",
                "tron",
                "ton",
                "ATOM",
                "near",
                "kaspa",
                "celo"
            ]
        );
        assert_eq!(sideloaded.apps[4].name, "Tezos Wallet");
    }

    #[test]
    fn a_comment_a_trailing_comma_and_single_quotes_are_fine() {
        let source = "SIDELOADED_APPS = {\n  # the first\n  'a': 'A', # why not\n  'b': 'B',\n}\n\
                      SIDELOADED_APPS_DIR = './libs/'\n";

        let sideloaded = parse_conftest(source).unwrap();

        assert_eq!(sideloaded.directory, "libs");
        assert_eq!(sideloaded.apps.len(), 2);
        assert_eq!(sideloaded.apps[1].variant, "b");
    }

    #[test]
    fn what_cannot_be_read_without_running_python_is_not_guessed() {
        let apps = "SIDELOADED_APPS = {\"a\": \"A\"}\n";
        // The directory is computed.
        assert_eq!(
            parse_conftest(&format!(
                "{apps}SIDELOADED_APPS_DIR = os.path.join(here, \"libs\")\n"
            )),
            None
        );
        // No directory at all.
        assert_eq!(parse_conftest(apps), None);
        // The apps are computed.
        assert_eq!(
            parse_conftest("SIDELOADED_APPS = make_apps()\nSIDELOADED_APPS_DIR = \"libs\"\n"),
            None
        );
        // An f-string is not a string the extension reads.
        assert_eq!(
            parse_conftest("SIDELOADED_APPS = {f\"a\": \"A\"}\nSIDELOADED_APPS_DIR = \"libs\"\n"),
            None
        );
        // Nothing to load.
        assert_eq!(
            parse_conftest("SIDELOADED_APPS = {}\nSIDELOADED_APPS_DIR = \"libs\"\n"),
            None
        );
        assert_eq!(parse_conftest(""), None);
    }

    #[test]
    fn a_commented_out_assignment_and_a_longer_name_are_not_the_declaration() {
        let source = "# SIDELOADED_APPS = {\"old\": \"Old\"}\n\
                      MY_SIDELOADED_APPS = {\"mine\": \"Mine\"}\n\
                      SIDELOADED_APPS = {\"new\": \"New\"}\n\
                      if SIDELOADED_APPS == {}: pass\n\
                      SIDELOADED_APPS_DIR = \"libs\"\n";

        let sideloaded = parse_conftest(source).unwrap();

        assert_eq!(sideloaded.apps.len(), 1);
        assert_eq!(sideloaded.apps[0].variant, "new");
    }

    #[test]
    fn the_binary_is_named_after_the_directory_that_the_device_builds_into() {
        assert_eq!(binary_name("APTOS", Device::NanoSPlus), "APTOS_nanos2.elf");
        assert_eq!(binary_name("celo", Device::ApexP), "celo_apex_p.elf");
        let sideloaded = sideloaded(&["tezos"]);
        assert_eq!(
            sideloaded.binary(&sideloaded.apps[0], Device::Flex),
            "test/python/lib_binaries/tezos_flex.elf"
        );
    }

    // ----- the recipes

    #[test]
    fn every_app_of_the_exchange_conftest_has_a_recipe() {
        let sideloaded = parse_conftest(EXCHANGE_CONFTEST).unwrap();

        for app in &sideloaded.apps {
            let recipe =
                recipe(&app.variant).unwrap_or_else(|| panic!("no recipe for {}", app.variant));
            assert!(recipe.repo.starts_with("LedgerHQ/app-"), "{recipe:?}");
            assert_eq!(recipe.git_ref, "develop");
        }
        assert!(sideloaded.is_buildable());
        assert_eq!(recipe("unheard_of"), None);
        assert!(!self::sideloaded(&["unheard_of"]).is_buildable());
    }

    #[test]
    fn two_variants_of_a_repository_share_it() {
        assert_eq!(recipe("ethereum"), recipe("ethereum_classic"));
        assert_eq!(
            recipe("celo").unwrap().url(),
            "https://github.com/LedgerHQ/app-celo-spender-dev.git"
        );
        assert_eq!(
            recipe("celo").unwrap().directory_name(),
            "app-celo-spender-dev"
        );
    }

    #[test]
    fn the_flags_are_the_ones_of_the_ci() {
        assert_eq!(
            make_flags("DOT").join(" "),
            "COIN=DOT CHAIN=DOT CAL_TEST_KEY=1 DOMAIN_NAME_TEST_KEY=1 SET_PLUGIN_TEST_KEY=1 \
             NFT_TEST_KEY=1 TRUSTED_NAME_TEST_KEY=1"
        );
    }

    // ----- the steps

    #[test]
    fn the_apps_are_cloned_once_per_repository_then_built_for_every_device() {
        let steps = steps(
            &app(),
            "./test/python/",
            &sideloaded(&["ethereum", "ethereum_classic", "tezos"]),
            &[Device::NanoSPlus, Device::Flex],
            false,
        );

        let lines = lines(&steps);
        let deps = "/work/app-exchange/test/python/.test_dependencies";
        let flags = "CAL_TEST_KEY=1 DOMAIN_NAME_TEST_KEY=1 SET_PLUGIN_TEST_KEY=1 NFT_TEST_KEY=1 \
                     TRUSTED_NAME_TEST_KEY=1";
        let expected: Vec<String> = vec![
            "container: bash -c <script> _ ./test/python/.test_dependencies".to_owned(),
            format!("host: git init --quiet {deps}/app-ethereum-dev"),
            format!(
                "host: git -C {deps}/app-ethereum-dev fetch --depth 1 https://github.com/LedgerHQ/app-ethereum-dev.git develop"
            ),
            format!("host: git -C {deps}/app-ethereum-dev checkout --quiet --force FETCH_HEAD"),
            format!(
                "host: git -C {deps}/app-ethereum-dev submodule update --init --recursive --depth 1"
            ),
            format!("host: git init --quiet {deps}/app-tezos-dev"),
            format!(
                "host: git -C {deps}/app-tezos-dev fetch --depth 1 https://github.com/LedgerHQ/app-tezos-dev.git develop"
            ),
            format!("host: git -C {deps}/app-tezos-dev checkout --quiet --force FETCH_HEAD"),
            format!(
                "host: git -C {deps}/app-tezos-dev submodule update --init --recursive --depth 1"
            ),
            format!(
                "container [nanosp]: bash -c <script> _ ./test/python/.test_dependencies/app-ethereum-dev test/python/lib_binaries ethereum_nanos2.elf 1 COIN=ethereum CHAIN=ethereum {flags}"
            ),
        ];
        assert_eq!(lines[..10].to_vec(), expected);
        // Six builds, a build for each of the two devices of each of the three apps.
        let builds = lines
            .iter()
            .filter(|line| line.starts_with("container ["))
            .count();
        assert_eq!(builds, 6);
        assert!(lines.contains(&"container [flex]: bash -c <script> _ ./test/python/.test_dependencies/app-ethereum-dev test/python/lib_binaries ethereum_classic_flex.elf 1 COIN=ethereum_classic CHAIN=ethereum_classic CAL_TEST_KEY=1 DOMAIN_NAME_TEST_KEY=1 SET_PLUGIN_TEST_KEY=1 NFT_TEST_KEY=1 TRUSTED_NAME_TEST_KEY=1".to_owned()));
    }

    #[test]
    fn a_build_that_fails_does_not_stop_the_others_and_the_check_ends_the_job() {
        let steps = steps(
            &app(),
            "./test/python/",
            &sideloaded(&["tezos", "stellar"]),
            &[Device::Flex],
            false,
        );

        for step in &steps {
            match step {
                Step::InContainer(container) => {
                    let building = container.sdk.is_some();
                    assert_eq!(container.continue_on_error, building, "{container:?}");
                }
                Step::OnHost(host) => assert!(
                    host.continue_on_error,
                    "a repository that cannot be reached must not stop the others: {host:?}"
                ),
            }
        }
        let Some(Step::InContainer(check)) = steps.last() else {
            panic!("{steps:?}")
        };
        assert_eq!(check.argv[1], "-c");
        let dependencies = "./test/python/.test_dependencies";
        let expected: Vec<String> = [
            "2",
            "LedgerHQ/app-tezos-dev",
            &format!("{dependencies}/app-tezos-dev"),
            "LedgerHQ/app-stellar-dev",
            &format!("{dependencies}/app-stellar-dev"),
            "test/python/lib_binaries/tezos_flex.elf",
            "test/python/lib_binaries/stellar_flex.elf",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(check.argv[4..], expected);
    }

    #[test]
    fn an_app_without_a_recipe_is_said_and_still_checked() {
        let steps = steps(
            &app(),
            "./test/python/",
            &sideloaded(&["unheard_of"]),
            &[Device::Flex],
            false,
        );

        let lines = lines(&steps);
        assert!(
            !lines.iter().any(|line| line.starts_with("host:")),
            "nothing to clone: {lines:?}"
        );
        assert_eq!(
            lines[1],
            "container: printf %s\n The extension does not know where to get unheard_of app \
             (unheard_of). Put its binaries in test/python/lib_binaries."
        );
        assert!(lines[2].ends_with("test/python/lib_binaries/unheard_of_flex.elf"));
    }

    #[test]
    fn a_rebuild_starts_by_removing_what_was_prepared_and_builds_everything_again() {
        let steps = steps(
            &app(),
            "./test/python/",
            &sideloaded(&["tezos"]),
            &[Device::Flex],
            true,
        );

        let lines = lines(&steps);
        assert_eq!(
            lines[0],
            "container: rm -rf ./test/python/.test_dependencies"
        );
        let build = lines
            .iter()
            .find(|line| line.contains("tezos_flex"))
            .unwrap();
        assert!(build.contains("tezos_flex.elf 0 COIN=tezos"), "{build}");
    }

    #[test]
    fn a_suite_at_the_root_of_the_app_keeps_its_dependencies_there() {
        let steps = steps(
            &app(),
            "./",
            &sideloaded(&["tezos"]),
            &[Device::Flex],
            false,
        );

        let lines = lines(&steps);
        assert_eq!(lines[0], "container: bash -c <script> _ .test_dependencies");
        assert_eq!(
            lines[1],
            "host: git init --quiet /work/app-exchange/.test_dependencies/app-tezos-dev"
        );
    }

    // ----- the scripts, run for real

    #[cfg(unix)]
    mod scripts {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::process::Command as Process;

        use super::*;

        /// The manifests are read with `tomllib`, from Python 3.11.
        fn python_reads_toml() -> bool {
            Process::new("python3")
                .args(["-c", "import tomllib"])
                .output()
                .is_ok_and(|output| output.status.success())
        }

        struct Sandbox {
            root: PathBuf,
        }

        impl Sandbox {
            fn new(name: &str) -> Self {
                let root = std::env::temp_dir()
                    .join(format!("ledger-sideloaded-{}-{name}", std::process::id()));
                let _ = fs::remove_dir_all(&root);
                fs::create_dir_all(root.join("bin")).unwrap();
                // A `make` that records what it is asked, and builds like the SDK does.
                let make = root.join("bin/make");
                fs::write(
                    &make,
                    "#!/bin/sh\necho \"make $*\" >> \"$ROOT/make.log\"\n\
                     case \"$*\" in *clean*) ;; *) mkdir -p \"$2/bin\" && echo built > \"$2/bin/app.elf\" ;; esac\n",
                )
                .unwrap();
                fs::set_permissions(&make, fs::Permissions::from_mode(0o755)).unwrap();
                Self { root }
            }

            fn source(&self, manifest: &str) -> PathBuf {
                let source = self.root.join("src");
                fs::create_dir_all(&source).unwrap();
                fs::write(source.join("ledger_app.toml"), manifest).unwrap();
                source
            }

            /// A Makefile in a directory of the source, as a fetched repository has.
            fn makefile(&self, relative: &str) {
                let directory = self.root.join("src").join(relative);
                fs::create_dir_all(&directory).unwrap();
                fs::write(directory.join("Makefile"), "").unwrap();
            }

            /// Runs a script of the module the way a step does: `bash -c <script> _ <arguments>`.
            fn run(&self, script: &str, arguments: &[&str]) -> (bool, String) {
                let path = format!(
                    "{}:{}",
                    self.root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                );
                let output = Process::new("bash")
                    .args(["-c", script, "_"])
                    .args(arguments)
                    .env("PATH", path)
                    .env("ROOT", &self.root)
                    .output()
                    .unwrap();
                let text = String::from_utf8_lossy(&output.stdout).into_owned()
                    + &String::from_utf8_lossy(&output.stderr);
                (output.status.success(), text)
            }

            fn log(&self) -> String {
                fs::read_to_string(self.root.join("make.log")).unwrap_or_default()
            }
        }

        impl Drop for Sandbox {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.root);
            }
        }

        fn text(path: &Path) -> &str {
            path.to_str().unwrap()
        }

        #[test]
        fn an_app_is_built_where_its_manifest_says_and_its_binary_is_put_in_the_directory() {
            if !python_reads_toml() {
                return;
            }
            let sandbox = Sandbox::new("build");
            let source = sandbox.source("[app]\nsdk = \"C\"\nbuild_directory = \"./app\"\n");
            sandbox.makefile("app");
            let out = sandbox.root.join("libs");

            let (success, output) = sandbox.run(
                BUILD_SCRIPT,
                &[
                    text(&source),
                    text(&out),
                    "tezos_flex.elf",
                    "1",
                    "COIN=tezos",
                    "CAL_TEST_KEY=1",
                ],
            );

            assert!(success, "{output}");
            let build = source.join("./app");
            assert_eq!(
                sandbox.log(),
                format!(
                    "make -C {0} clean\nmake -C {0} -j COIN=tezos CAL_TEST_KEY=1\n",
                    build.display()
                )
            );
            assert_eq!(
                fs::read_to_string(out.join("tezos_flex.elf")).unwrap(),
                "built\n"
            );
        }

        #[test]
        fn without_a_manifest_the_makefile_is_at_the_root() {
            if !python_reads_toml() {
                return;
            }
            let sandbox = Sandbox::new("no-manifest");
            let source = sandbox.root.join("src");
            fs::create_dir_all(&source).unwrap();
            sandbox.makefile("");
            let out = sandbox.root.join("libs");

            let (success, output) = sandbox.run(
                BUILD_SCRIPT,
                &[text(&source), text(&out), "x_flex.elf", "1"],
            );

            assert!(success, "{output}");
            assert!(
                sandbox
                    .log()
                    .contains(&format!("make -C {}/./ clean", source.display()))
            );
            assert!(out.join("x_flex.elf").exists());
        }

        #[test]
        fn a_binary_that_is_there_is_kept_unless_everything_is_rebuilt() {
            if !python_reads_toml() {
                return;
            }
            let sandbox = Sandbox::new("skip");
            let source = sandbox.source("[app]\nsdk = \"C\"\nbuild_directory = \"./\"\n");
            sandbox.makefile("");
            let out = sandbox.root.join("libs");
            fs::create_dir_all(&out).unwrap();
            fs::write(out.join("a_flex.elf"), "old").unwrap();

            let (success, output) = sandbox.run(
                BUILD_SCRIPT,
                &[text(&source), text(&out), "a_flex.elf", "1"],
            );
            assert!(success, "{output}");
            assert!(output.contains("a_flex.elf is already there."), "{output}");
            assert_eq!(sandbox.log(), "");
            assert_eq!(fs::read_to_string(out.join("a_flex.elf")).unwrap(), "old");

            let (success, output) = sandbox.run(
                BUILD_SCRIPT,
                &[text(&source), text(&out), "a_flex.elf", "0"],
            );
            assert!(success, "{output}");
            assert_eq!(
                fs::read_to_string(out.join("a_flex.elf")).unwrap(),
                "built\n"
            );
        }

        #[test]
        fn a_rust_app_is_not_built_and_says_so() {
            if !python_reads_toml() {
                return;
            }
            let sandbox = Sandbox::new("rust");
            let source = sandbox.source("[app]\nsdk = \"rust\"\nbuild_directory = \"./\"\n");
            let out = sandbox.root.join("libs");

            let (success, output) = sandbox.run(
                BUILD_SCRIPT,
                &[text(&source), text(&out), "r_flex.elf", "1"],
            );

            assert!(!success);
            assert!(
                output.contains("is a rust app. Build it by hand"),
                "{output}"
            );
            assert_eq!(sandbox.log(), "");
        }

        #[test]
        fn the_check_lists_what_is_missing_and_fails_only_then() {
            let sandbox = Sandbox::new("check");
            let present = sandbox.root.join("present.elf");
            fs::write(&present, "").unwrap();
            let absent = sandbox.root.join("absent.elf");

            let (success, output) = sandbox.run(CHECK_SCRIPT, &["0", text(&present)]);
            assert!(success, "{output}");
            assert!(output.contains("Every sideloaded app is in place."));

            let (success, output) =
                sandbox.run(CHECK_SCRIPT, &["0", text(&present), text(&absent)]);
            assert!(!success);
            assert!(
                output.contains(&format!("Missing: {}", absent.display())),
                "{output}"
            );
            assert!(output.contains("Repositories not fetched: 0. Binaries missing: 1."));
            assert!(!output.contains("in place"));
        }

        #[test]
        fn the_check_lists_every_repository_that_was_not_fetched_and_every_missing_binary() {
            let sandbox = Sandbox::new("unreachable");
            let fetched = sandbox.root.join("fetched");
            fs::create_dir_all(fetched.join(".git")).unwrap();
            fs::write(fetched.join("Makefile"), "").unwrap();
            // `git init` made the directory, then the fetch failed: there is only `.git`.
            let refused = sandbox.root.join("refused");
            fs::create_dir_all(refused.join(".git")).unwrap();
            let never_created = sandbox.root.join("never-created");
            let present = sandbox.root.join("present.elf");
            fs::write(&present, "").unwrap();
            let absent = sandbox.root.join("absent.elf");

            let (success, output) = sandbox.run(
                CHECK_SCRIPT,
                &[
                    "3",
                    "LedgerHQ/fetched",
                    text(&fetched),
                    "LedgerHQ/app-bitcoin-dev",
                    text(&refused),
                    "LedgerHQ/never-created",
                    text(&never_created),
                    text(&present),
                    text(&absent),
                ],
            );

            assert!(!success, "the job fails when anything is left");
            assert!(
                !output.contains("Not fetched: LedgerHQ/fetched"),
                "{output}"
            );
            assert!(
                output.contains("Not fetched: LedgerHQ/app-bitcoin-dev"),
                "{output}"
            );
            assert!(
                output.contains("Not fetched: LedgerHQ/never-created"),
                "{output}"
            );
            assert!(output.contains(&format!("Missing: {}", absent.display())));
            assert!(!output.contains(&format!("Missing: {}", present.display())));
            assert!(
                output.contains("Repositories not fetched: 2. Binaries missing: 1."),
                "everything is listed at the end: {output}"
            );
        }

        #[test]
        fn an_app_whose_repository_was_not_fetched_says_so_and_builds_nothing() {
            if !python_reads_toml() {
                return;
            }
            let sandbox = Sandbox::new("no-makefile");
            // What a failed fetch leaves: a directory with no Makefile, and no manifest.
            let source = sandbox.root.join("src");
            fs::create_dir_all(source.join(".git")).unwrap();
            let out = sandbox.root.join("libs");

            let (success, output) = sandbox.run(
                BUILD_SCRIPT,
                &[text(&source), text(&out), "bitcoin_flex.elf", "1"],
            );

            assert!(!success);
            assert!(
                output.contains("bitcoin_flex.elf: there is no Makefile in"),
                "{output}"
            );
            assert!(output.contains("Was the repository fetched?"), "{output}");
            assert_eq!(sandbox.log(), "", "make was not called");
            assert!(!out.join("bitcoin_flex.elf").exists());
        }

        #[test]
        fn what_is_prepared_stays_out_of_the_git_status() {
            let sandbox = Sandbox::new("ignore");
            let dependencies = sandbox.root.join("deps");

            let (success, output) = sandbox.run(IGNORE_SCRIPT, &[text(&dependencies)]);

            assert!(success, "{output}");
            assert_eq!(
                fs::read_to_string(dependencies.join(".gitignore")).unwrap(),
                "*\n"
            );
        }
    }
}
