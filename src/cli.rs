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
}
