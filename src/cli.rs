use crate::output::TreeOptions;
use clap::Parser;

#[derive(Parser)]
#[command(name = "specdiff", version, about = "Show test outline changes on a branch")]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    #[arg(short, long, help = "Print diff to stdout and exit (non-interactive)")]
    pub print: bool,

    #[arg(long, help = "Base revision (default: auto-detected)")]
    pub base: Option<String>,

    #[arg(long, help = "Head revision (default: working copy)")]
    pub head: Option<String>,

    #[arg(long, value_enum, default_value = "tree", help = "Output format (use with --print)")]
    pub format: OutputFormat,

    #[arg(long, help = "Only show changed specs")]
    pub changed_only: bool,

    #[arg(long, help = "Show every unchanged spec line (no context truncation)")]
    pub full_context: bool,

    #[arg(long, help = "Force a specific framework")]
    pub framework: Option<String>,

    #[arg(long, help = "Filter specs by name pattern")]
    pub filter: Option<String>,

    #[arg(long, help = "Disable colored output")]
    pub no_color: bool,

    #[arg(long, help = "Base directory (for diffing without VCS)")]
    pub base_dir: Option<String>,

    #[arg(long, help = "Head directory (for diffing without VCS)")]
    pub head_dir: Option<String>,
}

impl Cli {
    pub fn prints(&self, stdout_is_terminal: bool) -> bool {
        self.print || !stdout_is_terminal
    }

    pub fn tree_options(&self) -> TreeOptions {
        TreeOptions {
            changed_only: self.changed_only,
            color: !self.no_color,
            full_context: self.full_context,
        }
    }
}

#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Tree,
    Json,
    Compact,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn tree_options(args: &[&str]) -> TreeOptions {
        let argv = std::iter::once("specdiff").chain(args.iter().copied());
        Cli::parse_from(argv).tree_options()
    }

    #[test]
    fn tree_options_default_to_color_with_truncated_context() {
        let opts = tree_options(&[]);
        assert!(opts.color);
        assert!(!opts.changed_only);
        assert!(!opts.full_context);
    }

    #[test]
    fn output_is_printed_when_asked_or_when_stdout_is_not_a_terminal() {
        let interactive = Cli::parse_from(["specdiff"]);
        assert!(!interactive.prints(true));
        assert!(interactive.prints(false));
        let print = Cli::parse_from(["specdiff", "--print"]);
        assert!(print.prints(true));
        assert!(print.prints(false));
    }

    #[test]
    fn tree_options_follow_their_flags() {
        let opts = tree_options(&["--no-color", "--changed-only", "--full-context"]);
        assert!(!opts.color);
        assert!(opts.changed_only);
        assert!(opts.full_context);
    }

    proptest! {
        #[test]
        fn prints_matches_its_truth_table(
            print_flag in proptest::bool::ANY,
            stdout_is_terminal in proptest::bool::ANY,
        ) {
            let cli = Cli { print: print_flag, ..cli_with_defaults() };
            prop_assert_eq!(cli.prints(stdout_is_terminal), print_flag || !stdout_is_terminal);
        }

        #[test]
        fn prints_round_trips_through_clap_parse(
            print_flag in proptest::bool::ANY,
        ) {
            let mut argv: Vec<&str> = vec!["specdiff"];
            if print_flag {
                argv.push("--print");
            }
            let parsed = Cli::parse_from(argv);
            prop_assert_eq!(parsed.print, print_flag);
            prop_assert!(parsed.prints(false));
            prop_assert_eq!(parsed.prints(true), print_flag);
        }

        #[test]
        fn tree_options_is_a_pointwise_isomorphism_of_its_three_flags(
            no_color in proptest::bool::ANY,
            changed_only in proptest::bool::ANY,
            full_context in proptest::bool::ANY,
        ) {
            let cli = Cli {
                no_color,
                changed_only,
                full_context,
                ..cli_with_defaults()
            };
            let opts = cli.tree_options();
            prop_assert_eq!(opts.color, !no_color);
            prop_assert_eq!(opts.changed_only, changed_only);
            prop_assert_eq!(opts.full_context, full_context);
        }

        #[test]
        fn tree_options_is_idempotent_and_involutive_under_flag_negation(
            no_color in proptest::bool::ANY,
            changed_only in proptest::bool::ANY,
            full_context in proptest::bool::ANY,
        ) {
            let cli = Cli {
                no_color,
                changed_only,
                full_context,
                ..cli_with_defaults()
            };
            let first = cli.tree_options();
            let second = cli.tree_options();
            prop_assert_eq!(first.color, second.color);
            prop_assert_eq!(first.changed_only, second.changed_only);
            prop_assert_eq!(first.full_context, second.full_context);
            let negated = Cli {
                no_color: !no_color,
                changed_only: !changed_only,
                full_context: !full_context,
                ..cli_with_defaults()
            };
            let flipped = negated.tree_options();
            prop_assert_eq!(flipped.color, !first.color);
            prop_assert_eq!(flipped.changed_only, !first.changed_only);
            prop_assert_eq!(flipped.full_context, !first.full_context);
        }
    }

    fn cli_with_defaults() -> Cli {
        Cli::parse_from(["specdiff"])
    }
}
