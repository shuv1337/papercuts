use crate::Severity;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// One `--file` value: a log path with an optional `LABEL=` prefix naming its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSpec {
    pub label: Option<String>,
    pub path: PathBuf,
}

pub const MAX_LABEL_LEN: usize = 64;

pub fn is_valid_label(label: &str) -> bool {
    let mut bytes = label.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && label.len() <= MAX_LABEL_LEN
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Splits `LABEL=PATH` when the text before the first `=` is a valid label; otherwise the
/// whole value is a path (so `./a=b.jsonl` stays a path).
pub fn parse_file_spec(value: &str) -> Result<FileSpec, String> {
    if let Some((label, path)) = value.split_once('=')
        && is_valid_label(label)
    {
        if path.is_empty() {
            return Err(format!("--file label '{label}' has an empty path"));
        }
        return Ok(FileSpec {
            label: Some(label.to_owned()),
            path: PathBuf::from(path),
        });
    }
    if value.is_empty() {
        return Err("--file path is empty".into());
    }
    Ok(FileSpec {
        label: None,
        path: PathBuf::from(value),
    })
}

#[derive(Debug, Parser)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    version,
    about,
    long_about = None,
    arg_required_else_help = true,
    subcommand_required = true,
    color = clap::ColorChoice::Never,
    rename_all = "kebab-case"
)]
pub struct Cli {
    #[command(flatten)]
    pub files: FileArgs,

    #[arg(long, global = true)]
    pub pretty: bool,

    #[command(subcommand)]
    pub command: Command,
}

// `--file` is declared on the root and on every subcommand (not as a clap global, which
// keeps only the last side's values), so `papercuts --file a list --file b` reads both.
#[derive(Debug, Clone, Default, Args)]
pub struct FileArgs {
    #[arg(
        long = "file",
        value_name = "[LABEL=]PATH",
        value_parser = parse_file_spec,
        action = ArgAction::Append,
        help = "Log file; repeat (optionally as LABEL=PATH) on list, related, and doctor to read several logs with each cut labeled by source"
    )]
    pub file: Vec<FileSpec>,
}

impl Cli {
    /// All `--file` values in argv order: root occurrences, then the subcommand's.
    pub fn files(&self) -> Vec<FileSpec> {
        let sub = match &self.command {
            Command::Add(args) => &args.files,
            Command::List(args) => &args.files,
            Command::Related(args) => &args.files,
            Command::Resolve(args) => &args.files,
            Command::Reopen(args) => &args.files,
            Command::Schema { files, .. } => files,
            Command::Doctor(args) => &args.files,
        };
        self.files.file.iter().chain(&sub.file).cloned().collect()
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(alias = "log")]
    Add(AddArgs),
    List(ListArgs),
    Related(RelatedArgs),
    Resolve(ResolveArgs),
    Reopen(ReopenArgs),
    Schema {
        #[arg(value_enum, default_value_t = SchemaTarget::All)]
        target: SchemaTarget,
        #[command(flatten)]
        files: FileArgs,
    },
    Doctor(DoctorArgs),
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[command(flatten)]
    pub files: FileArgs,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    #[command(flatten)]
    pub files: FileArgs,
    #[arg(value_name = "TEXT")]
    pub text: Option<String>,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long, value_enum, default_value_t = Severity::Minor)]
    pub severity: Severity,
    #[arg(
        long,
        allow_hyphen_values = true,
        value_name = "TEXT",
        help = "Command that failed"
    )]
    pub cmd: Option<String>,
    #[arg(long = "exit", value_name = "N", help = "Command exit status")]
    pub exit_code: Option<i32>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Read regular UTF-8 PATH (<=1 MiB); best-effort redaction; store sanitized value <=4096 bytes"
    )]
    pub stderr_file: Option<PathBuf>,
    #[arg(
        long,
        allow_hyphen_values = true,
        value_name = "TEXT",
        help = "Additional evidence or filing note"
    )]
    pub evidence: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(
        long,
        help = "Skip related-match advisory computation; exact duplicate detection still runs"
    )]
    pub no_check: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub files: FileArgs,
    #[arg(long, value_enum, default_value_t = StatusFilter::Open)]
    pub status: StatusFilter,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long, value_enum)]
    pub severity: Option<Severity>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct RelatedArgs {
    #[command(flatten)]
    pub files: FileArgs,
    #[arg(value_name = "TEXT")]
    pub text: String,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long, default_value_t = 5)]
    pub limit: usize,
    #[arg(long, default_value_t = 0.0)]
    pub min_score: f64,
}

#[derive(Debug, Args)]
pub struct ResolveArgs {
    #[command(flatten)]
    pub files: FileArgs,
    #[arg(
        value_name = "ID",
        num_args = 1..,
        required = true,
        help = "One or more IDs or unique prefixes"
    )]
    pub ids: Vec<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ReopenArgs {
    #[command(flatten)]
    pub files: FileArgs,
    #[arg(
        value_name = "ID",
        num_args = 1..,
        required = true,
        help = "One or more IDs or unique prefixes"
    )]
    pub ids: Vec<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StatusFilter {
    Open,
    Resolved,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Json,
    Md,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SchemaTarget {
    All,
    Record,
    Error,
    ExitCodes,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_covers_defaults_aliases_and_globals() {
        let cli =
            Cli::try_parse_from(["papercuts", "--file", "x", "log", "ouch", "--pretty"]).unwrap();
        assert!(cli.pretty);
        assert_eq!(
            cli.files(),
            vec![FileSpec {
                label: None,
                path: PathBuf::from("x")
            }]
        );
        let Command::Add(args) = cli.command else {
            panic!("expected add")
        };
        assert_eq!(args.text.as_deref(), Some("ouch"));
        assert_eq!(args.severity, Severity::Minor);

        let cli = Cli::try_parse_from(["papercuts", "list"]).unwrap();
        let Command::List(args) = cli.command else {
            panic!("expected list")
        };
        assert_eq!(args.status, StatusFilter::Open);
        assert_eq!(args.limit, 50);
        assert_eq!(args.format, OutputFormat::Json);
    }

    #[test]
    fn file_specs_split_labels_only_when_the_prefix_is_a_valid_label() {
        let spec = |label: Option<&str>, path: &str| FileSpec {
            label: label.map(str::to_owned),
            path: PathBuf::from(path),
        };
        for (input, expected) in [
            ("log.jsonl", spec(None, "log.jsonl")),
            ("shuvbot=/a/b.jsonl", spec(Some("shuvbot"), "/a/b.jsonl")),
            ("host-1.lan_x=a=b", spec(Some("host-1.lan_x"), "a=b")),
            ("./a=b.jsonl", spec(None, "./a=b.jsonl")),
            ("/x/a=b.jsonl", spec(None, "/x/a=b.jsonl")),
            ("=b.jsonl", spec(None, "=b.jsonl")),
            ("-x=b.jsonl", spec(None, "-x=b.jsonl")),
        ] {
            assert_eq!(parse_file_spec(input), Ok(expected), "{input}");
        }
        assert!(parse_file_spec("shuvbot=").is_err());
        assert!(parse_file_spec("").is_err());
        assert!(!is_valid_label(&"a".repeat(MAX_LABEL_LEN + 1)));

        let cli = Cli::try_parse_from([
            "papercuts",
            "--file",
            "a.jsonl",
            "list",
            "--file",
            "b=b.jsonl",
        ])
        .unwrap();
        assert_eq!(
            cli.files(),
            vec![spec(None, "a.jsonl"), spec(Some("b"), "b.jsonl")]
        );
        let cli =
            Cli::try_parse_from(["papercuts", "doctor", "--file", "a", "--file", "b"]).unwrap();
        assert_eq!(cli.files(), vec![spec(None, "a"), spec(None, "b")]);
    }

    #[test]
    fn parser_rejects_bad_values_and_missing_required_id() {
        assert!(Cli::try_parse_from(["papercuts", "list", "--format", "jsonl"]).is_err());
        assert!(Cli::try_parse_from(["papercuts", "add", "x", "--severity", "critical"]).is_err());
        assert!(Cli::try_parse_from(["papercuts", "resolve"]).is_err());
        assert!(Cli::try_parse_from(["papercuts", "reopen"]).is_err());
        assert!(Cli::try_parse_from(["papercuts"]).is_err());
    }

    #[test]
    fn parser_accepts_every_command_and_stdin_marker() {
        for args in [
            vec!["papercuts", "add", "-"],
            vec!["papercuts", "list", "--status", "all"],
            vec![
                "papercuts",
                "related",
                "ouch",
                "--tag",
                "tooling",
                "--limit",
                "3",
            ],
            vec!["papercuts", "resolve", "abcd"],
            vec!["papercuts", "reopen", "abcd", "--note", "handed off"],
            vec!["papercuts", "schema", "record"],
            vec!["papercuts", "doctor"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn parser_accepts_leading_hyphen_text_values_without_swallowing_following_options() {
        let cli = Cli::try_parse_from([
            "papercuts",
            "add",
            "text",
            "--cmd",
            "-tool arg",
            "--evidence",
            "--detail note",
            "--agent",
            "tester",
        ])
        .unwrap();
        let Command::Add(args) = cli.command else {
            panic!("expected add")
        };
        assert_eq!(args.cmd.as_deref(), Some("-tool arg"));
        assert_eq!(args.evidence.as_deref(), Some("--detail note"));
        assert_eq!(args.agent.as_deref(), Some("tester"));

        let cli = Cli::try_parse_from([
            "papercuts",
            "resolve",
            "abcd1234",
            "--note",
            "--retry after timeout",
            "--agent",
            "fixer",
        ])
        .unwrap();
        let Command::Resolve(args) = cli.command else {
            panic!("expected resolve")
        };
        assert_eq!(args.note.as_deref(), Some("--retry after timeout"));
        assert_eq!(args.agent.as_deref(), Some("fixer"));
    }
}
