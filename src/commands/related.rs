use crate::cli::{FileSpec, RelatedArgs};
use crate::error::{AppError, AppResult};
use crate::output::{self, Meta};
use crate::relevance::{self, Query, RelatedMatch};
use crate::store;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RelatedData {
    pub items: Vec<RelatedMatch>,
    pub count: usize,
    pub total: usize,
    pub truncated: bool,
}

pub fn run(args: RelatedArgs, files: Vec<FileSpec>, pretty: bool) -> AppResult<i32> {
    let plan = store::discover_read(files)?;
    let folded = store::fold_plan(
        &plan,
        "Pass an existing --file PATH or run `papercuts add` to create a discovered default file.",
    )?;
    let mut warnings = folded.warnings;

    let report = relevance::top_matches(
        Query::new(
            &args.text,
            &args.tags,
            plan.repo.as_deref().and_then(|path| path.to_str()),
        ),
        &folded.items,
        usize::MAX,
        args.min_score,
    );
    warnings.extend(report.warnings);
    let matches = report.matches;
    let total = matches.len();
    let mut items = matches;
    items.truncate(args.limit);
    if items.is_empty() {
        warnings.push("no related papercuts matched; try lower --min-score or broader text".into());
    }
    let data = RelatedData {
        count: items.len(),
        total,
        truncated: total > items.len(),
        items,
    };
    let mut meta = Meta::new();
    plan.apply_meta(&mut meta);
    meta.warnings = warnings;
    output::write_success(data, pretty, meta)
        .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
    Ok(0)
}
