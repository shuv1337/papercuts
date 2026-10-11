use crate::cli::SchemaTarget;
use crate::error;
use serde_json::{Value, json};

pub fn contract(target: SchemaTarget) -> Value {
    let records = json!({
        "cut": {"kind":"cut","id":"pc_<12 lowercase hex>","ts":"RFC3339 UTC milliseconds","agent":"string","text":"string <= 10000 bytes","tags":["string"],"severity":"minor|major|blocker","cwd":"absolute path","repo":"absolute path|null","evidence":"optional {cmd:string,exit:integer,stderr:string,note:string}; absent fields omitted; values best-effort redacted"},
        "resolve": {"kind":"resolve","id":"pc_<12 lowercase hex>","ts":"RFC3339 UTC milliseconds","agent":"string","note":"string|null"},
        "reopen": {"kind":"reopen","id":"pc_<12 lowercase hex>","ts":"RFC3339 UTC milliseconds","agent":"string","note":"string|null"},
        "list_item": {"cut":"all cut fields","status":"open|resolved","resolution":"{ts,agent,note}|omitted","reopened":"{ts,agent,note} of the latest reopen that took effect|omitted","source":"label of the log the cut was read from; only in labeled reads, otherwise omitted"},
        "related_match": {"score":"0.0..1.0 rounded to 6 decimals; BM25 raw_score/(raw_score+1.0) + 0.15 tag Jaccard + 0.10 exact repo match, capped at 1.0","cut":"all cut fields","status":"open|resolved","resolution":"{ts,agent,note}|omitted","reopened":"{ts,agent,note}|omitted","source":"label|omitted, as list_item"}
    });
    let errors = json!({
        "shape": {"ok":false,"error":{"code":"string","message":"string","details":{},"retryable":false,"suggested_fix":"string"},"meta":{"contract":1}},
        "codes": error::error_codes()
    });
    let exit_codes: Value = json!(error::exit_code_map());
    match target {
        SchemaTarget::Record => json!({"contract":1,"records":records}),
        SchemaTarget::Error => json!({"contract":1,"errors":errors}),
        SchemaTarget::ExitCodes => json!({"contract":1,"exit_codes":exit_codes}),
        SchemaTarget::All => json!({
            "contract": 1,
            "success_envelope": {"ok":true,"data":"command-specific object","meta":{"contract":1,"file":"absolute path where relevant; omitted in labeled reads","sources":"[{label,path}] in --file order; labeled reads only, otherwise omitted","agent_source":"flag|env|detected|default where relevant","warnings":["string; omitted when empty"]}},
            "commands": {
                "add": {"alias":["log"],"positional":"TEXT or -; optional when stdin is piped","flags":{"--agent":"NAME","--tag":"TAG; repeatable","--severity":"minor|major|blocker; default minor","--cmd":"TEXT; optional evidence command; leading hyphens accepted","--exit":"N; optional evidence exit status","--stderr-file":"PATH; regular UTF-8 file up to 1 MiB, redacted then stored up to 4096 bytes; symlinks follow targets","--evidence":"TEXT; optional evidence note; leading hyphens accepted","--dry-run":"boolean","--no-check":"boolean; skip related-match advisory computation, exact ID duplicate check still runs"},"output":"{changed,record,related?:[related_match],related_resolution?:{ts,agent,note}}; fuzzy matches are advisory-only and never block appending new content","read_only":false,"appends":true,"destructive":false},
                "list": {"flags":{"--status":"open|resolved|all; default open","--agent":"NAME","--tag":"TAG","--severity":"minor|major|blocker","--since":"full RFC3339|Nd|Nh","--limit":"N; default 50","--format":"json|md; default json"},"output":"{items,count,total,truncated}; md is raw markdown","read_only":true,"appends":false,"destructive":false},
                "related": {"positional":"TEXT","flags":{"--tag":"TAG; repeatable; boosts matching tags","--limit":"N; default 5","--min-score":"F; default 0.0"},"output":"{items:[related_match],count,total,truncated}; ranks open and resolved cuts by lexical BM25/tag/repo score; read-only probe, not a status filter","read_only":true,"appends":false,"destructive":false},
                "resolve": {"positional":"one or more IDs; optional pc_ plus at least 4 hex digits each","flags":{"--note":"TEXT; leading hyphens accepted","--agent":"NAME","--dry-run":"boolean"},"output":{"one":"{changed,record-with-resolution}","two_or_more":"{changed,records:[...]}; IDs are canonicalized, sorted, and duplicate inputs collapse; mixed already-resolved IDs warn with a sorted count/list"},"read_only":false,"appends":true,"destructive":false},
                "reopen": {"positional":"one or more IDs; optional pc_ plus at least 4 hex digits each","flags":{"--note":"TEXT; leading hyphens accepted","--agent":"NAME","--dry-run":"boolean"},"output":{"one":"{changed,record-with-reopened}","two_or_more":"{changed,records:[...]}; IDs are canonicalized, sorted, and duplicate inputs collapse; mixed already-open IDs warn with a sorted count/list"},"notes":"appends one reopen event per resolved ID; a later resolve records a fresh resolution, which is how a stale resolution note is corrected","read_only":false,"appends":true,"destructive":false},
                "schema": {"positional":"all|record|error|exit-codes; default all","read_only":true,"appends":false,"destructive":false},
                "doctor": {"flags":{},"output":"{healthy,findings:[{line,kind,message,source?}],checked_lines}; in labeled reads each finding names its source and checked_lines sums all sources","exit_codes":{"0":"healthy","1":"findings"},"read_only":true,"appends":false,"destructive":false}
            },
            "global_flags": ["--file <[LABEL=]PATH>","--pretty"],
            "sources": {"single":"zero or one unlabeled --file keeps discovery and meta.file unchanged","labeled":"list, related, and doctor accept repeated --file values, before or after the subcommand; two or more values, or any LABEL=PATH, read every log as an explicit source","label":"LABEL=PATH when the text before the first '=' matches [A-Za-z0-9][A-Za-z0-9._-]{0,63}; otherwise the file stem; ./ keeps a name containing '=' a path","fold":"each source folds independently; merged items use the normative sort; a cut ID seen in an earlier source wins with a 'duplicate cut across sources' warning; per-source warnings are prefixed with 'LABEL: '","errors":"duplicate labels, duplicate paths, and underivable stem labels are invalid_argument (2); a missing source is not_found (66)","mutations":"add, resolve, and reopen accept one unlabeled --file and return invalid_argument (2) otherwise, appending nothing"},
            "env": {
                "PAPERCUTS_FILE":"log-file override",
                "PAPERCUTS_AGENT":"agent-name fallback",
                "PAPERCUTS_NOW":"full RFC3339 clock override"
            },
            "records": records,
            "id": {"prefix":"pc_","hex_digits":12,"hash":"SHA-256 first 6 bytes","fields_in_order":["ts","agent","text","severity","sorted tags joined with comma"],"encoding":"u32 little-endian UTF-8 byte length before each field"},
            "discovery": ["--file","PAPERCUTS_FILE","nearest .git directory or file then <root>/.papercuts.jsonl","$HOME/.papercuts/log.jsonl"],
            "errors": errors,
            "exit_codes": exit_codes,
            "storage": {"format":"append-only JSONL","locking":"local filesystems only; 50 retries x 100ms","durability":"best effort; no fsync per append"}
        }),
    }
}
