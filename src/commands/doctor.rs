use crate::error::{AppError, AppResult};
use crate::output::{self, Meta};
use crate::store;
use crate::{CutRecord, ResolveRecord, compute_id};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
}

pub fn run(file: Option<PathBuf>, pretty: bool) -> AppResult<i32> {
    let resolved = store::discover(file)?;
    let mut warnings = Vec::new();
    let (mut data, file_existed) = match store::with_shared(&resolved.path, |log| {
        let bytes = store::read_bytes(log, &resolved.path)?;
        Ok(inspect(&bytes))
    }) {
        Ok(data) => (data, true),
        Err(error) if error.code == "not_found" && error.exit_code == 66 => {
            if resolved.explicit {
                return Err(AppError::not_found(
                    format!("papercuts file not found: {}", resolved.path.display()),
                    "Pass an existing --file PATH or omit --file to inspect discovered state.",
                ));
            }
            warnings.push("no papercuts file yet; healthy empty state".into());
            (
                DoctorData {
                    healthy: true,
                    findings: Vec::new(),
                    checked_lines: 0,
                },
                false,
            )
        }
        Err(error) => return Err(error),
    };
    if file_existed
        && let Some(repo) = resolved.repo.as_ref()
        && resolved.path.starts_with(repo)
        && Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["check-ignore", "-q", "--"])
            .arg(&resolved.path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    {
        data.findings.push(Finding {
            line: 0,
            kind: "gitignored".into(),
            message: "papercuts file is gitignored; papercuts will not appear in diffs".into(),
        });
        data.healthy = false;
    }
    let exit = i32::from(!data.healthy);
    let mut meta = Meta::new();
    meta.file = Some(resolved.path.to_string_lossy().into_owned());
    meta.warnings = warnings;
    output::write_success(data, pretty, meta)
        .map_err(|error| AppError::from_io(error, std::path::Path::new("stdout")))?;
    Ok(exit)
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
            });
            continue;
        }
        if raw.starts_with(b"<<<<<<< ") || raw.starts_with(b">>>>>>> ") {
            findings.push(Finding {
                line,
                kind: "conflict_marker".into(),
                message: "complete git conflict-marker line found".into(),
            });
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            findings.push(Finding {
                line,
                kind: "malformed".into(),
                message: "line is not valid JSON".into(),
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
                            });
                            continue;
                        }
                        resolves.push((line, kind, event.id));
                    }
                    Err(error) => findings.push(Finding {
                        line,
                        kind: "malformed".into(),
                        message: format!("invalid {kind} record: {error}"),
                    }),
                }
            }
            Some(kind) => findings.push(Finding {
                line,
                kind: "unknown_kind".into(),
                message: format!("unknown event kind '{kind}'"),
            }),
            None => findings.push(Finding {
                line,
                kind: "unknown_kind".into(),
                message: "event has no string kind field".into(),
            }),
        }
    }
    for (line, kind, id) in resolves {
        if !cut_ids.contains(&id) {
            findings.push(Finding {
                line,
                kind: format!("orphan_{kind}"),
                message: format!("{kind} references unknown cut {id}"),
            });
        }
    }
    DoctorData {
        healthy: findings.is_empty(),
        findings,
        checked_lines,
    }
}
