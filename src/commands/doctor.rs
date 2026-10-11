use crate::cli::FileSpec;
use crate::error::{AppError, AppResult};
use crate::output::{self, Meta};
use crate::store;
use crate::{CutRecord, ResolveRecord, compute_id};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

#[derive(Debug, Serialize, Deserialize)]
pub struct DoctorData {
    pub healthy: bool,
    pub findings: Vec<Finding>,
    pub checked_lines: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Finding {
    pub line: usize,
    pub kind: String,
    pub message: String,
    /// Label of the log the finding belongs to; only set when checking labeled or multiple logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

pub fn run(files: Vec<FileSpec>, pretty: bool) -> AppResult<i32> {
    let plan = store::discover_read(files)?;
    let mut warnings = Vec::new();
    let mut data = DoctorData {
        healthy: true,
        findings: Vec::new(),
        checked_lines: 0,
    };
    for source in &plan.sources {
        let checked = check_source(source, plan.repo.as_deref())?;
        let Some(checked) = checked else {
            warnings.push("no papercuts file yet; healthy empty state".into());
            continue;
        };
        data.checked_lines += checked.checked_lines;
        data.findings
            .extend(checked.findings.into_iter().map(|mut finding| {
                finding.source.clone_from(&source.label);
                finding
            }));
    }
    data.healthy = data.findings.is_empty();
    let exit = i32::from(!data.healthy);
    let mut meta = Meta::new();
    plan.apply_meta(&mut meta);
    meta.warnings = warnings;
    output::write_success(data, pretty, meta)
        .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
    Ok(exit)
}

/// Inspects one log; `None` means a discovered default that does not exist yet.
fn check_source(source: &store::SourceFile, repo: Option<&Path>) -> AppResult<Option<DoctorData>> {
    let mut data = match store::with_shared(&source.path, |log| {
        let bytes = store::read_bytes(log, &source.path)?;
        Ok(inspect(&bytes))
    }) {
        Ok(data) => data,
        Err(error) if error.code == "not_found" && error.exit_code == 66 => {
            if source.explicit {
                return Err(AppError::not_found(
                    format!("papercuts file not found: {}", source.path.display()),
                    "Pass an existing --file PATH or omit --file to inspect discovered state.",
                ));
            }
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if let Some(repo) = repo
        && source.path.starts_with(repo)
        && Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["check-ignore", "-q", "--"])
            .arg(&source.path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    {
        data.findings.push(Finding {
            line: 0,
            kind: "gitignored".into(),
            message: "papercuts file is gitignored; papercuts will not appear in diffs".into(),
            source: None,
        });
        data.healthy = false;
    }
    Ok(Some(data))
}

fn inspect(bytes: &[u8]) -> DoctorData {
    let mut findings = Vec::new();
    let mut cuts = HashMap::<String, Vec<u8>>::new();
    let mut cut_ids = HashSet::new();
    let mut resolves = Vec::<(usize, &str, String)>::new();
    let mut checked_lines = 0;
    let torn = !bytes.is_empty() && !bytes.ends_with(b"\n");
    let line_count = bytes.split(|byte| *byte == b'\n').count();
    for (index, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if raw.is_empty() && index + 1 == line_count {
            continue;
        }
        checked_lines += 1;
        let line = index + 1;
        if torn && index + 1 == line_count {
            findings.push(Finding {
                line,
                kind: "torn_line".into(),
                message: "final physical line is not newline-terminated".into(),
                source: None,
            });
            continue;
        }
        if raw.starts_with(b"<<<<<<< ") || raw.starts_with(b">>>>>>> ") {
            findings.push(Finding {
                line,
                kind: "conflict_marker".into(),
                message: "complete git conflict-marker line found".into(),
                source: None,
            });
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            findings.push(Finding {
                line,
                kind: "malformed".into(),
                message: "line is not valid JSON".into(),
                source: None,
            });
            continue;
        };
        match value.get("kind").and_then(Value::as_str) {
            Some("cut") => match serde_json::from_value::<CutRecord>(value) {
                Ok(cut) => {
                    if cut.ts.parse::<jiff::Timestamp>().is_err() {
                        findings.push(Finding {
                            line,
                            kind: "malformed".into(),
                            message: "cut ts is not a full RFC3339 timestamp".into(),
                            source: None,
                        });
                        continue;
                    }
                    let mut tags = cut.tags.clone();
                    tags.sort();
                    let expected = compute_id(&cut.ts, &cut.agent, &cut.text, cut.severity, &tags);
                    if cut.id != expected {
                        findings.push(Finding {
                            line,
                            kind: "id_conflict".into(),
                            message: format!("cut ID {} does not recompute to {expected}", cut.id),
                            source: None,
                        });
                    }
                    if let Some(first) = cuts.get(&cut.id) {
                        let (kind, message) = if first == raw {
                            (
                                "duplicate_cut",
                                format!("byte-identical duplicate cut {}", cut.id),
                            )
                        } else {
                            (
                                "id_conflict",
                                format!(
                                    "cut {} has a different payload than its first occurrence",
                                    cut.id
                                ),
                            )
                        };
                        findings.push(Finding {
                            line,
                            kind: kind.into(),
                            message,
                            source: None,
                        });
                    } else {
                        cuts.insert(cut.id.clone(), raw.to_vec());
                    }
                    cut_ids.insert(cut.id);
                }
                Err(error) => findings.push(Finding {
                    line,
                    kind: "malformed".into(),
                    message: format!("invalid cut record: {error}"),
                    source: None,
                }),
            },
            Some(kind @ ("resolve" | "reopen")) => {
                let kind = if kind == "resolve" {
                    "resolve"
                } else {
                    "reopen"
                };
                match serde_json::from_value::<ResolveRecord>(value) {
                    Ok(event) => {
                        if event.ts.parse::<jiff::Timestamp>().is_err() {
                            findings.push(Finding {
                                line,
                                kind: "malformed".into(),
                                message: format!("{kind} ts is not a full RFC3339 timestamp"),
                                source: None,
                            });
                            continue;
                        }
                        resolves.push((line, kind, event.id));
                    }
                    Err(error) => findings.push(Finding {
                        line,
                        kind: "malformed".into(),
                        message: format!("invalid {kind} record: {error}"),
                        source: None,
                    }),
                }
            }
            Some(kind) => findings.push(Finding {
                line,
                kind: "unknown_kind".into(),
                message: format!("unknown event kind '{kind}'"),
                source: None,
            }),
            None => findings.push(Finding {
                line,
                kind: "unknown_kind".into(),
                message: "event has no string kind field".into(),
                source: None,
            }),
        }
    }
    for (line, kind, id) in resolves {
        if !cut_ids.contains(&id) {
            findings.push(Finding {
                line,
                kind: format!("orphan_{kind}"),
                message: format!("{kind} references unknown cut {id}"),
                source: None,
            });
        }
    }
    DoctorData {
        healthy: findings.is_empty(),
        findings,
        checked_lines,
    }
}
