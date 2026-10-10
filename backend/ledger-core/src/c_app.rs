//! What the Makefile of a C app prints when asked about the app. Only `make` can tell, so it
//! runs in a container and this module reads what it printed.

/// The target of `make` that prints `APPNAME=<name>`.
pub const LIST_INFO_TARGET: &str = "listinfo";
/// The target of `make` that prints `VARIANTS <variable> <value>...`.
pub const LIST_VARIANTS_TARGET: &str = "listvariants";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakefileInfo {
    pub name: String,
    pub variants: Option<Variants>,
}

/// The values one Makefile variable can take to build the app in different flavours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variants {
    /// The variable, passed to `make` as `<name>=<value>`.
    pub name: String,
    /// Never empty.
    pub values: Vec<String>,
}

impl Variants {
    /// The value named like the app, or else the first.
    pub fn default_value(&self, app_name: &str) -> &str {
        self.values
            .iter()
            .find(|value| value.eq_ignore_ascii_case(app_name))
            .unwrap_or(&self.values[0])
    }
}

/// Reads the output of `make listinfo`.
pub fn parse_app_name(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("APPNAME="))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// Reads the output of `make listvariants`. `None` when the app has no variants.
pub fn parse_variants(output: &str) -> Option<Variants> {
    let line = output.lines().find(|line| line.starts_with("VARIANTS"))?;
    let mut words = line.split_whitespace().skip(1);
    let name = words.next()?.to_owned();
    let values: Vec<String> = words.map(str::to_owned).collect();
    (!values.is_empty()).then_some(Variants { name, values })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variants(name: &str, values: &[&str]) -> Variants {
        Variants {
            name: name.to_owned(),
            values: values.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn the_app_name_is_read_from_the_listinfo_output() {
        let output = "make: Entering directory '/app'\nAPPNAME=Boilerplate\nAPPVERSION=1.0.0\n";

        assert_eq!(parse_app_name(output), Some("Boilerplate".to_owned()));
    }

    #[test]
    fn an_app_name_with_spaces_is_kept_whole() {
        assert_eq!(
            parse_app_name("APPNAME=Bitcoin Test\n"),
            Some("Bitcoin Test".to_owned())
        );
    }

    #[test]
    fn no_app_name_is_found_in_other_output() {
        assert_eq!(parse_app_name(""), None);
        assert_eq!(parse_app_name("APPNAME=\n"), None);
        assert_eq!(parse_app_name("make: *** No rule to make target.\n"), None);
    }

    #[test]
    fn variants_are_read_from_the_listvariants_output() {
        let output = "make: Entering directory '/app'\nVARIANTS COIN BTC LTC\n";

        assert_eq!(
            parse_variants(output),
            Some(variants("COIN", &["BTC", "LTC"]))
        );
    }

    #[test]
    fn an_app_without_variants_has_none() {
        assert_eq!(parse_variants("VARIANTS\n"), None);
        assert_eq!(parse_variants("VARIANTS COIN\n"), None);
        assert_eq!(parse_variants(""), None);
    }

    #[test]
    fn the_default_variant_is_the_one_named_like_the_app_or_the_first() {
        let variants = variants("COIN", &["BTC", "LTC"]);

        assert_eq!(variants.default_value("ltc"), "LTC");
        assert_eq!(variants.default_value("Boilerplate"), "BTC");
    }
}
