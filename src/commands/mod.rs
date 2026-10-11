pub mod add;
pub mod doctor;
pub mod list;
pub mod related;
pub mod reopen;
pub mod resolve;
pub mod schema;

use crate::cli::{Cli, Command, FileSpec};
use crate::error::{AppError, AppResult};
use crate::output;
use jiff::Timestamp;
use std::path::PathBuf;

pub fn run(cli: Cli, now: Timestamp) -> AppResult<i32> {
    let files = cli.files();
    match cli.command {
        Command::Add(args) => add::run(args, single_file(files, "add")?, cli.pretty, now),
        Command::List(args) => list::run(args, files, cli.pretty, now),
        Command::Related(args) => related::run(args, files, cli.pretty),
        Command::Resolve(args) => {
            resolve::run(args, single_file(files, "resolve")?, cli.pretty, now)
        }
        Command::Reopen(args) => reopen::run(args, single_file(files, "reopen")?, cli.pretty, now),
        Command::Schema { target, .. } => {
            output::write_success(schema::contract(target), cli.pretty, output::Meta::new())
                .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
            Ok(0)
        }
        Command::Doctor(_) => doctor::run(files, cli.pretty),
    }
}

/// Mutations append to exactly one log, so they accept one unlabeled `--file` at most.
fn single_file(files: Vec<FileSpec>, command: &str) -> AppResult<Option<PathBuf>> {
    if files.len() > 1 {
        return Err(AppError::invalid_argument(
            format!(
                "`{command}` writes to one log but got {} --file values",
                files.len()
            ),
            "Pass a single --file PATH; repeat --file only on list, related, or doctor.",
        ));
    }
    match files.into_iter().next() {
        Some(FileSpec {
            label: Some(label), ..
        }) => Err(AppError::invalid_argument(
            format!("`{command}` does not take a source label ('{label}=')"),
            "Pass the plain --file PATH; LABEL=PATH only applies to list, related, and doctor. Use ./PATH if the file name itself contains '='.",
        )),
        Some(spec) => Ok(Some(spec.path)),
        None => Ok(None),
    }
}
