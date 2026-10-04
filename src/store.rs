use crate::error::{AppError, AppResult};
use crate::{CutRecord, ItemStatus, ListItem, Resolution, ResolveRecord};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::Duration;

const LOCK_ATTEMPTS: usize = 50;
const LOCK_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct ResolvedFile {
    pub path: PathBuf,
    pub explicit: bool,
    pub repo: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct FoldResult {
    pub items: Vec<ListItem>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct WarningCounts {
    torn: usize,
    malformed: usize,
    unknown: usize,
    duplicate_cuts: usize,
    duplicate_resolves: usize,
    redundant_reopens: usize,
    orphans: usize,
    orphan_reopens: usize,
}

/// Status of one ID after replaying its resolve/reopen events in file order.
#[derive(Default)]
struct StatusState {
    resolution: Option<Resolution>,
    reopened: Option<Resolution>,
    resolved_seen: bool,
    reopen_seen: bool,
    redundant_reopens: usize,
}

fn as_resolution(event: &ResolveRecord) -> Resolution {
    Resolution {
        ts: event.ts.clone(),
        agent: event.agent.clone(),
        note: event.note.clone(),
    }
}

pub fn discover(flag: Option<PathBuf>) -> AppResult<ResolvedFile> {
    let cwd = std::env::current_dir().map_err(|error| AppError::from_io(error, Path::new(".")))?;
    let repo = find_repo_root(&cwd);
    if let Some(path) = flag {
        return Ok(ResolvedFile {
            path: absolute(&cwd, path),
            explicit: true,
            repo,
        });
    }
    if let Some(path) = std::env::var_os("PAPERCUTS_FILE")
        && !path.is_empty()
    {
        return Ok(ResolvedFile {
            path: absolute(&cwd, PathBuf::from(path)),
            explicit: true,
            repo,
        });
    }
    if let Some(root) = repo.clone() {
        return Ok(ResolvedFile {
            path: root.join(".papercuts.jsonl"),
            explicit: false,
            repo: Some(root),
        });
    }
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            AppError::config(
                "cannot resolve the home directory for the default papercuts file",
                "Set HOME or pass --file PATH.",
            )
        })?;
    Ok(ResolvedFile {
        path: absolute(&cwd, home).join(".papercuts/log.jsonl"),
        explicit: false,
        repo: None,
    })
}

pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .map(Path::to_path_buf)
}

fn absolute(cwd: &Path, path: PathBuf) -> PathBuf {
    let joined = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub fn with_shared<T>(path: &Path, action: impl FnOnce(&mut File) -> AppResult<T>) -> AppResult<T> {
    let mut file = File::open(path).map_err(|error| AppError::from_log_open(error, path))?;
    lock(&file, path, false)?;
    let result = action(&mut file);
    let unlock = file
        .unlock()
        .map_err(|error| AppError::from_io(error, path));
    match (result, unlock) {
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

pub fn with_exclusive<T>(
    path: &Path,
    create: bool,
    action: impl FnOnce(&mut File) -> AppResult<T>,
) -> AppResult<T> {
    if create && let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| AppError::from_io(error, parent))?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(create)
        .open(path)
        .map_err(|error| AppError::from_log_open(error, path))?;
    lock(&file, path, true)?;
    let result = action(&mut file);
    let unlock = file
        .unlock()
        .map_err(|error| AppError::from_io(error, path));
    match (result, unlock) {
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

fn lock(file: &File, path: &Path, exclusive: bool) -> AppResult<()> {
    for attempt in 0..LOCK_ATTEMPTS {
        let result = if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                let error: std::io::Error = error.into();
                if error.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(AppError::from_io(error, path));
                }
                if attempt + 1 < LOCK_ATTEMPTS {
                    thread::sleep(LOCK_DELAY);
                }
            }
        }
    }
    Err(AppError::lock_timeout(path))
}

pub fn read_bytes(file: &mut File, path: &Path) -> AppResult<Vec<u8>> {
    file.seek(SeekFrom::Start(0))
        .and_then(|_| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map(|_| bytes)
        })
        .map_err(|error| AppError::from_io(error, path))
}

pub fn append_json<T: serde::Serialize>(
    file: &mut File,
    path: &Path,
    prior: &[u8],
    record: &T,
) -> AppResult<()> {
    let mut record_bytes = Vec::new();
    serde_json::to_writer(&mut record_bytes, record)
        .map_err(|error| AppError::internal(error.to_string()))?;
    record_bytes.push(b'\n');
    append_bytes(file, path, prior, &record_bytes)
}

pub fn append_json_batch<T: serde::Serialize>(
    file: &mut File,
    path: &Path,
    prior: &[u8],
    records: &[T],
) -> AppResult<()> {
    let mut record_bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut record_bytes, record)
            .map_err(|error| AppError::internal(error.to_string()))?;
        record_bytes.push(b'\n');
    }
    append_bytes(file, path, prior, &record_bytes)
}

fn append_bytes(file: &mut File, path: &Path, prior: &[u8], record_bytes: &[u8]) -> AppResult<()> {
    append_bytes_with(file, path, prior, record_bytes, |file, bytes| {
        file.write_all(bytes)
    })
}

fn append_bytes_with(
    file: &mut File,
    path: &Path,
    prior: &[u8],
    record_bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
) -> AppResult<()> {
    let original_len = file
        .metadata()
        .map_err(|error| AppError::from_io(error, path))?
        .len();
    let mut bytes = Vec::new();
    if !prior.is_empty() && !prior.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(record_bytes);
    // If the write fails, roll back to the pre-write length; if rollback also fails, surface both.
    if let Err(error) = write(file, &bytes) {
        if let Err(rollback) = file.set_len(original_len) {
            return Err(AppError {
                code: "io_error",
                message: format!(
                    "append failed: {error}; rollback to original length {original_len} failed: {rollback}"
                ),
                details: json!({}),
                retryable: false,
                suggested_fix: "Check the papercuts file and filesystem, then retry.".into(),
                exit_code: 74,
            });
        }
        return Err(AppError::from_io(error, path));
    }
    Ok(())
}

pub fn fold_bytes(bytes: &[u8]) -> FoldResult {
    let mut cuts = BTreeMap::<String, CutRecord>::new();
    let mut states = HashMap::<String, StatusState>::new();
    let mut counts = WarningCounts::default();
    let complete_len = if bytes.is_empty() || bytes.ends_with(b"\n") {
        bytes.len()
    } else {
        counts.torn += 1;
        bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |i| i + 1)
    };

    let complete = &bytes[..complete_len];
    let complete = complete.strip_suffix(b"\n").unwrap_or(complete);
    for raw in complete.split(|byte| *byte == b'\n') {
        if complete.is_empty() {
            break;
        }
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            counts.malformed += 1;
            continue;
        };
        let kind = value.get("kind").and_then(Value::as_str).map(str::to_owned);
        match kind.as_deref() {
            Some("cut") => match serde_json::from_value::<CutRecord>(value) {
                Ok(mut cut) => {
                    if cut.ts.parse::<jiff::Timestamp>().is_err() {
                        counts.malformed += 1;
                        continue;
                    }
                    cut.tags.sort();
                    if cuts.contains_key(&cut.id) {
                        counts.duplicate_cuts += 1;
                    } else {
                        cuts.insert(cut.id.clone(), cut);
                    }
                }
                Err(_) => counts.malformed += 1,
            },
            Some(kind @ ("resolve" | "reopen")) => {
                match serde_json::from_value::<ResolveRecord>(value) {
                    Ok(event) => {
                        if event.ts.parse::<jiff::Timestamp>().is_err() {
                            counts.malformed += 1;
                            continue;
                        }
                        // Status events for one ID replay in file order; the cut line may sit anywhere.
                        let state = states.entry(event.id.clone()).or_default();
                        if kind == "resolve" {
                            state.resolved_seen = true;
                            if state.resolution.is_some() {
                                counts.duplicate_resolves += 1;
                            } else {
                                state.resolution = Some(as_resolution(&event));
                            }
                        } else {
                            state.reopen_seen = true;
                            if state.resolution.take().is_some() {
                                state.reopened = Some(as_resolution(&event));
                            } else {
                                state.redundant_reopens += 1;
                            }
                        }
                    }
                    Err(_) => counts.malformed += 1,
                }
            }
            _ => counts.unknown += 1,
        }
    }

    for (id, state) in &states {
        if cuts.contains_key(id) {
            counts.redundant_reopens += state.redundant_reopens;
        } else {
            if state.resolved_seen {
                counts.orphans += 1;
            }
            if state.reopen_seen {
                counts.orphan_reopens += 1;
            }
        }
    }
    let mut items: Vec<_> = cuts
        .into_values()
        .map(|cut| {
            let state = states.remove(&cut.id).unwrap_or_default();
            ListItem {
                status: if state.resolution.is_some() {
                    ItemStatus::Resolved
                } else {
                    ItemStatus::Open
                },
                cut,
                resolution: state.resolution,
                reopened: state.reopened,
            }
        })
        .collect();
    items.sort_by(|left, right| {
        let left_ts = left.cut.ts.parse::<jiff::Timestamp>().ok();
        let right_ts = right.cut.ts.parse::<jiff::Timestamp>().ok();
        right
            .cut
            .severity
            .rank()
            .cmp(&left.cut.severity.rank())
            .then_with(|| right_ts.cmp(&left_ts))
            .then_with(|| left.cut.id.cmp(&right.cut.id))
    });

    let mut warnings = Vec::new();
    warning(&mut warnings, counts.torn, "torn final line");
    warning(&mut warnings, counts.malformed, "malformed line");
    warning(&mut warnings, counts.unknown, "unknown event");
    warning(&mut warnings, counts.duplicate_cuts, "duplicate cut");
    warning(
        &mut warnings,
        counts.duplicate_resolves,
        "duplicate resolve",
    );
    warning(&mut warnings, counts.redundant_reopens, "redundant reopen");
    warning(&mut warnings, counts.orphans, "orphan resolve");
    warning(&mut warnings, counts.orphan_reopens, "orphan reopen");
    FoldResult { items, warnings }
}

fn warning(warnings: &mut Vec<String>, count: usize, label: &str) {
    if count > 0 {
        warnings.push(format!(
            "skipped {count} {label}{}",
            if count == 1 { "" } else { "s" }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Severity, compute_id};
    use std::io::Write;
    use tempfile::TempDir;

    fn cut(id: &str) -> String {
        cut_with_text(id, "x")
    }

    fn cut_with_text(id: &str, text: &str) -> String {
        serde_json::json!({
            "kind":"cut", "id":id, "ts":"2026-07-09T00:00:00.000Z",
            "agent":"a", "text":text, "tags":[], "severity":"minor",
            "cwd":"/tmp", "repo":null
        })
        .to_string()
    }

    fn resolve(id: &str) -> String {
        serde_json::json!({
            "kind":"resolve", "id":id, "ts":"2026-07-10T00:00:00.000Z",
            "agent":"a", "note":null
        })
        .to_string()
    }

    fn status_event(kind: &str, id: &str, ts: &str, note: &str) -> String {
        serde_json::json!({
            "kind":kind, "id":id, "ts":ts, "agent":"a", "note":note
        })
        .to_string()
    }

    #[test]
    fn reopen_events_replay_in_file_order_per_id() {
        let id = compute_id("2026-07-09T00:00:00.000Z", "a", "x", Severity::Minor, &[]);
        let resolve_a = status_event("resolve", &id, "2026-07-10T00:00:00.000Z", "first");
        let reopen = status_event("reopen", &id, "2026-07-11T00:00:00.000Z", "handed off");
        let resolve_b = status_event("resolve", &id, "2026-07-12T00:00:00.000Z", "second");
        let cases = [
            (
                "resolve then reopen",
                format!("{}\n{resolve_a}\n{reopen}\n", cut(&id)),
                ItemStatus::Open,
                None,
                Some("handed off"),
                0,
            ),
            (
                "reopen then resolve records the new resolution",
                format!("{}\n{resolve_a}\n{reopen}\n{resolve_b}\n", cut(&id)),
                ItemStatus::Resolved,
                Some("second"),
                Some("handed off"),
                0,
            ),
            (
                "status events before the cut line still replay in order",
                format!("{resolve_a}\n{reopen}\n{resolve_b}\n{}\n", cut(&id)),
                ItemStatus::Resolved,
                Some("second"),
                Some("handed off"),
                0,
            ),
            (
                "reopen of an open cut is redundant",
                format!("{}\n{reopen}\n", cut(&id)),
                ItemStatus::Open,
                None,
                None,
                1,
            ),
            (
                "second resolve without a reopen stays a duplicate",
                format!("{}\n{resolve_a}\n{resolve_b}\n", cut(&id)),
                ItemStatus::Resolved,
                Some("first"),
                None,
                1,
            ),
            (
                "orphan reopen",
                format!(
                    "{}\n{}\n",
                    cut(&id),
                    status_event("reopen", "pc_deadbeef0000", "2026-07-11T00:00:00.000Z", "x")
                ),
                ItemStatus::Open,
                None,
                None,
                1,
            ),
        ];
        for (name, input, status, resolution_note, reopen_note, warning_count) in cases {
            let folded = fold_bytes(input.as_bytes());
            assert_eq!(folded.items.len(), 1, "{name}");
            let item = &folded.items[0];
            assert_eq!(item.status, status, "{name}");
            assert_eq!(
                item.resolution.as_ref().and_then(|r| r.note.as_deref()),
                resolution_note,
                "{name}"
            );
            assert_eq!(
                item.reopened.as_ref().and_then(|r| r.note.as_deref()),
                reopen_note,
                "{name}"
            );
            assert_eq!(
                folded.warnings.len(),
                warning_count,
                "{name}: {:?}",
                folded.warnings
            );
        }
    }

    #[test]
    fn batch_append_rollback_restores_a_torn_tail_after_partial_write_failure() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("cuts.jsonl");
        let original = b"{\"kind\":\"cut\"}\n{\"kind\":";
        std::fs::write(&path, original).unwrap();
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .unwrap();

        let error = append_bytes_with(
            &mut file,
            &path,
            original,
            b"{\"kind\":\"resolve\"}\n{\"kind\":\"resolve\"}\n",
            |file, bytes| {
                file.write_all(&bytes[..8])?;
                Err(std::io::Error::other("injected partial write failure"))
            },
        )
        .unwrap_err();

        assert_eq!(error.code, "io_error");
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn fold_matrix() {
        let id = compute_id("2026-07-09T00:00:00.000Z", "a", "x", Severity::Minor, &[]);
        let cases = [
            ("cut", format!("{}\n", cut(&id)), 1, ItemStatus::Open, 0),
            (
                "resolve before cut",
                format!("{}\n{}\n", resolve(&id), cut(&id)),
                1,
                ItemStatus::Resolved,
                0,
            ),
            (
                "duplicates",
                format!(
                    "{}\n{}\n{}\n{}\n",
                    cut(&id),
                    cut(&id),
                    resolve(&id),
                    resolve(&id)
                ),
                1,
                ItemStatus::Resolved,
                2,
            ),
            (
                "unknown malformed orphan",
                format!(
                    "{{\"kind\":\"future\"}}\nnope\n{}\n{}\n",
                    resolve("pc_deadbeef0000"),
                    cut(&id)
                ),
                1,
                ItemStatus::Open,
                3,
            ),
            (
                "torn tail",
                format!("{}\n{{\"kind\":", cut(&id)),
                1,
                ItemStatus::Open,
                1,
            ),
            (
                "all adversarial orderings interleaved",
                format!(
                    "{}\n{{\"kind\":\"future\"}}\n{}\n{}\n{}\n{}\n{}\nnope\n{{\"kind\":",
                    resolve(&id),
                    cut(&id),
                    cut(&id),
                    cut_with_text(&id, "conflicting payload"),
                    resolve(&id),
                    resolve("pc_deadbeef0000"),
                ),
                1,
                ItemStatus::Resolved,
                6,
            ),
        ];
        for (name, input, item_count, status, warning_count) in cases {
            let folded = fold_bytes(input.as_bytes());
            assert_eq!(folded.items.len(), item_count, "{name}");
            if !folded.items.is_empty() {
                assert_eq!(folded.items[0].status, status, "{name}");
                assert_eq!(folded.items[0].cut.text, "x", "{name}");
            }
            assert_eq!(folded.warnings.len(), warning_count, "{name}");
        }
    }
}
