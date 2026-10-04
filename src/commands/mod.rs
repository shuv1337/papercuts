pub mod add;
pub mod doctor;
pub mod list;
pub mod related;
pub mod reopen;
pub mod resolve;
pub mod schema;

use crate::cli::{Cli, Command};
use crate::error::{AppError, AppResult};
use crate::output;
use jiff::Timestamp;

pub fn run(cli: Cli, now: Timestamp) -> AppResult<i32> {
    match cli.command {
        Command::Add(args) => add::run(args, cli.file, cli.pretty, now),
        Command::List(args) => list::run(args, cli.file, cli.pretty, now),
        Command::Related(args) => related::run(args, cli.file, cli.pretty),
        Command::Resolve(args) => resolve::run(args, cli.file, cli.pretty, now),
        Command::Reopen(args) => reopen::run(args, cli.file, cli.pretty, now),
        Command::Schema { target } => {
            output::write_success(schema::contract(target), cli.pretty, output::Meta::new())
                .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
            Ok(0)
        }
        Command::Doctor => doctor::run(cli.file, cli.pretty),
    }
}
