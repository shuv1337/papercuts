use crate::cli::RelatedArgs;
use crate::error::{AppError, AppResult};
use crate::output::{self, Meta};
use crate::relevance::{self, Query, RelatedMatch};
use crate::store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct RelatedData {
    pub items: Vec<RelatedMatch>,
    pub count: usize,
    pub total: usize,
    pub truncated: bool,
}

pub fn run(args: RelatedArgs, file: Option<PathBuf>, pretty: bool) -> AppResult<i32> {
    let resolved = store::discover(file)?;
    let mut warnings = Vec::new();
    let folded = match store::with_shared(&resolved.path, |log| {
        let bytes = store::read_bytes(log, &resolved.path)?;
        Ok(store::fold_bytes(&bytes))
    }) {
        Ok(folded) => folded,
        Err(error) if error.code == "not_found" && error.exit_code == 66 => {
            if resolved.explicit {
                return Err(AppError::not_found(
                    format!("papercuts file not found: {}", resolved.path.display()),
                    "Pass an existing --file PATH or run `papercuts add` to create a discovered default file.",
                ));
            }
            warnings.push("no papercuts file yet; papercuts add creates it".into());
            store::FoldResult::default()
        }
        Err(error) => return Err(error),
    };
    warnings.extend(folded.warnings);

    let report = relevance::top_matches(
        Query::new(
            &args.text,
            &args.tags,
            resolved.repo.as_deref().and_then(|path| path.to_str()),
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
    meta.file = Some(resolved.path.to_string_lossy().into_owned());
    meta.warnings = warnings;
    output::write_success(data, pretty, meta)
        .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
    Ok(0)
}
