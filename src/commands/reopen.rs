use crate::cli::ReopenArgs;
use crate::commands::resolve::{ResolveData, ResolveManyData, match_id, normalize_prefix};
use crate::error::{AppError, AppResult};
use crate::output::{self, Meta};
use crate::store;
use crate::{ItemStatus, ListItem, ReopenRecord, Resolution, format_timestamp, resolve_agent};
use jiff::Timestamp;
use std::path::PathBuf;

pub fn run(
    args: ReopenArgs,
    file: Option<PathBuf>,
    pretty: bool,
    now: Timestamp,
) -> AppResult<i32> {
    let prefixes: Vec<_> = args
        .ids
        .iter()
        .map(|id| normalize_prefix(id))
        .collect::<AppResult<_>>()?;
    let resolved = store::discover(file)?;
    if args
        .agent
        .as_deref()
        .is_some_and(|agent| agent.trim().is_empty())
    {
        return Err(AppError::invalid_input(
            "agent name cannot be empty or whitespace-only",
            "Pass a non-empty --agent NAME or omit the flag.",
        ));
    }
    let (agent, source) = resolve_agent(args.agent);
    let ts = format_timestamp(now);
    let reopening = Resolution {
        ts: ts.clone(),
        agent: agent.clone(),
        note: args.note.clone(),
    };
    let many = args.ids.len() > 1;
    let action = |log: &mut std::fs::File| -> AppResult<(bool, Vec<String>, Vec<ListItem>)> {
        let bytes = store::read_bytes(log, &resolved.path)?;
        let folded = store::fold_bytes(&bytes);
        let mut ids = prefixes
            .iter()
            .map(|prefix| match_id(prefix, &folded.items))
            .collect::<AppResult<Vec<_>>>()?;
        ids.sort();
        ids.dedup();
        let mut items = ids
            .iter()
            .map(|id| {
                folded
                    .items
                    .iter()
                    .find(|item| item.cut.id == *id)
                    .cloned()
                    .ok_or_else(|| AppError::internal("matched papercut disappeared during reopen"))
            })
            .collect::<AppResult<Vec<_>>>()?;
        let already_open_ids: Vec<_> = ids
            .iter()
            .zip(&items)
            .filter(|(_, item)| item.status == ItemStatus::Open)
            .map(|(id, _)| id.clone())
            .collect();
        let mut events = Vec::new();
        for (id, item) in ids.iter().zip(items.iter_mut()) {
            if item.status == ItemStatus::Open {
                continue;
            }
            item.status = ItemStatus::Open;
            item.resolution = None;
            item.reopened = Some(reopening.clone());
            events.push(ReopenRecord {
                kind: "reopen".into(),
                id: id.clone(),
                ts: ts.clone(),
                agent: agent.clone(),
                note: args.note.clone(),
            });
        }
        let changed = !args.dry_run && !events.is_empty();
        if changed {
            store::append_json_batch(log, &resolved.path, &bytes, &events)?;
        }
        Ok((changed, already_open_ids, items))
    };
    let (changed, already_open_ids, records) = if args.dry_run {
        store::with_shared(&resolved.path, action)
    } else {
        store::with_exclusive(&resolved.path, false, action)
    }?;
    let mut meta = Meta::new();
    meta.file = Some(resolved.path.to_string_lossy().into_owned());
    meta.agent_source = Some(source.into());
    if already_open_ids.len() == records.len() {
        meta.warnings.push("already open".into());
    } else if !already_open_ids.is_empty() {
        let noun = if already_open_ids.len() == 1 {
            "ID"
        } else {
            "IDs"
        };
        meta.warnings.push(format!(
            "already open: {} {noun} ({})",
            already_open_ids.len(),
            already_open_ids.join(", ")
        ));
    } else if args.dry_run {
        meta.warnings
            .push("dry run; no reopen event appended".into());
    }
    if !many {
        output::write_success(
            ResolveData {
                changed,
                record: records.into_iter().next().expect("one record"),
            },
            pretty,
            meta,
        )
        .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
    } else {
        output::write_success(ResolveManyData { changed, records }, pretty, meta)
            .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
    }
    Ok(0)
}
