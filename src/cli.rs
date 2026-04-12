use clap::Parser;

#[derive(Parser)]
#[command(name = "spec-diff", version, about = "Show test outline changes on a branch")]
pub struct Cli {
    #[arg(long, help = "Base revision (default: merge-base with main)")]
    pub base: Option<String>,

    #[arg(long, help = "Head revision (default: working copy)")]
    pub head: Option<String>,

    #[arg(long, value_enum, default_value = "tree", help = "Output format")]
    pub format: OutputFormat,

    #[arg(long, help = "Only show changed specs")]
    pub changed_only: bool,

    #[arg(long, help = "Force a specific framework")]
    pub framework: Option<String>,

    #[arg(long, help = "Filter specs by name pattern")]
    pub filter: Option<String>,

    #[arg(long, help = "Disable colored output")]
    pub no_color: bool,

    #[arg(long, help = "Base directory (for testing without VCS)")]
    pub base_dir: Option<String>,

    #[arg(long, help = "Head directory (for testing without VCS)")]
    pub head_dir: Option<String>,
}

#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Tree,
    Json,
    Compact,
}
