use crate::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Write};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub contract: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Labeled sources in `--file` order; replaces `file` when a read spans labeled logs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl Meta {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            contract: 1,
            file: None,
            sources: Vec::new(),
            agent_source: None,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMeta {
    pub label: String,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SuccessEnvelope<T> {
    pub ok: bool,
    pub data: T,
    pub meta: Meta,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub ok: bool,
    pub error: ErrorBody,
    pub meta: Meta,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub details: Value,
    pub retryable: bool,
    pub suggested_fix: String,
}

pub fn write_success<T: Serialize>(data: T, pretty: bool, meta: Meta) -> io::Result<()> {
    let envelope = SuccessEnvelope {
        ok: true,
        data,
        meta,
    };
    let mut output = io::BufWriter::new(io::stdout().lock());
    if pretty {
        serde_json::to_writer_pretty(&mut output, &envelope)?;
    } else {
        serde_json::to_writer(&mut output, &envelope)?;
    }
    writeln!(output)
}

pub fn write_error(error: &AppError) -> i32 {
    let envelope = ErrorEnvelope {
        ok: false,
        error: ErrorBody {
            code: error.code.into(),
            message: error.message.clone(),
            details: error.details.clone(),
            retryable: error.retryable,
            suggested_fix: error.suggested_fix.clone(),
        },
        meta: Meta::new(),
    };
    let mut output = io::BufWriter::new(io::stderr().lock());
    let _ = serde_json::to_writer(&mut output, &envelope);
    let _ = writeln!(output);
    error.exit_code
}
