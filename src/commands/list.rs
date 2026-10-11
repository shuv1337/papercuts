use crate::cli::{FileSpec, ListArgs, OutputFormat, StatusFilter};
use crate::error::{AppError, AppResult};
use crate::output::{self, Meta};
use crate::store;
use crate::{ItemStatus, ListItem, Severity, parse_since};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Debug, Serialize, Deserialize)]
pub struct ListData {
    pub items: Vec<ListItem>,
    pub count: usize,
    pub total: usize,
    pub truncated: bool,
}

pub fn run(args: ListArgs, files: Vec<FileSpec>, pretty: bool, now: Timestamp) -> AppResult<i32> {
    let plan = store::discover_read(files)?;
    let folded = store::fold_plan(
        &plan,
        "Pass an existing --file PATH or run `papercuts add` to create a discovered default file.",
    )?;
    let mut warnings = folded.warnings;
    let since = args
        .since
        .as_deref()
        .map(|value| parse_since(value, now))
        .transpose()?;
    let mut items: Vec<_> = folded
        .items
        .into_iter()
        .filter(|item| match args.status {
            StatusFilter::Open => item.status == ItemStatus::Open,
            StatusFilter::Resolved => item.status == ItemStatus::Resolved,
            StatusFilter::All => true,
        })
        .filter(|item| {
            args.agent
                .as_ref()
                .is_none_or(|agent| &item.cut.agent == agent)
        })
        .filter(|item| {
            args.tag
                .as_ref()
                .is_none_or(|tag| item.cut.tags.contains(tag))
        })
        .filter(|item| {
            args.severity
                .is_none_or(|severity| item.cut.severity == severity)
        })
        .filter(|item| {
            since.is_none_or(|threshold| {
                item.cut
                    .ts
                    .parse::<Timestamp>()
                    .is_ok_and(|timestamp| timestamp >= threshold)
            })
        })
        .collect();
    let total = items.len();
    items.truncate(args.limit);
    let data = ListData {
        count: items.len(),
        total,
        truncated: total > items.len(),
        items,
    };
    if data.items.is_empty() {
        warnings.push("no papercuts matched; try --status all or broader filters".into());
    }
    if args.format == OutputFormat::Md {
        write_markdown(&data.items, &warnings)?;
    } else {
        let mut meta = Meta::new();
        plan.apply_meta(&mut meta);
        meta.warnings = warnings;
        output::write_success(data, pretty, meta)
            .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
    }
    Ok(0)
}

fn write_markdown(items: &[ListItem], warnings: &[String]) -> AppResult<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    for severity in [Severity::Blocker, Severity::Major, Severity::Minor] {
        let matching: Vec<_> = items
            .iter()
            .filter(|item| item.cut.severity == severity)
            .collect();
        if matching.is_empty() {
            continue;
        }
        writeln!(
            output,
            "## {}",
            match severity {
                Severity::Blocker => "Blocker",
                Severity::Major => "Major",
                Severity::Minor => "Minor",
            }
        )
        .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
        for item in matching {
            let id = if item.status == ItemStatus::Resolved {
                format!("~~{}~~", item.cut.id)
            } else {
                item.cut.id.clone()
            };
            let tags = if item.cut.tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", item.cut.tags.join(","))
            };
            let source = item
                .source
                .as_ref()
                .map_or_else(String::new, |label| format!("@{label}"));
            writeln!(
                output,
                "- [{id}] {} — {}{source}, {}{tags}",
                item.cut.text, item.cut.agent, item.cut.ts
            )
            .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
        }
    }
    for warning in warnings {
        writeln!(output, "> note: {warning}")
            .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
    }
    Ok(())
}
