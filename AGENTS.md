# AGENTS.md — papercuts

Machine-facing contract for agents working in this repo.

## What this is

`papercuts` is a Rust CLI (clap 4 derive) that lets AI agents log friction into an append-only JSONL file. Agent-only tool: JSON envelopes on stdout, structured errors on stderr, stable exit codes. The normative contract is `docs/plans/2026-07-09-papercuts-design.md` (r3) — treat it as law; its Amendments sections record review provenance and deliberate deviations from the rust-agent-cli skill (exit 74 extension, diagnose-only doctor, no --quiet).

## Build and gate

```bash
cargo build --release
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

All four must pass before any commit. Run the test suite 5x when touching store.rs or anything concurrency-adjacent — a single green run proves nothing about races.

Use `cargo fmt --check` (or `git diff --check`) for whitespace validation.

For focused Rust tests, `cargo test` accepts one positional filter: use one shared substring or separate invocations. Report-generation transforms must pass verbatim replacement text to `re.sub`/`re.subn` through a callable such as `lambda _: replacement`; use `seen.update(tokens)` or `seen |= tokens` for a Python set union, never `set.add(other_set)`.

## Layout

- `src/store.rs` — file discovery, locking (bounded try_lock → exit 75), append (write_all + tear-heal + rollback), the normative fold. The riskiest file; change with care and tests.
- `src/commands/*.rs` — one file per subcommand. Mutations run read→fold→decide→append inside one exclusive-lock critical section.
- `src/error.rs` — the public error contract (codes ↔ exit codes). Never add an undocumented code.
- `src/output.rs` — envelope types. Every output shape is a serde struct.
- `tests/cli.rs` — black-box assert_cmd tests. Env via `Command::env` only, never `std::env::set_var` (parallel-test races).

## Triaging the papercut log

When triaging open cuts in a papercut log (for example `~/.local/share/papercuts/shuv.jsonl`), route each cut by where its fix would land:

- **The fix lands in a git repo with a GitHub remote** (cut `repo`/`cwd`, or the parent repo of a deleted `/tmp` or worktree path): do not fix it. Search that repo's open issues for an existing match first. If none exists, open an issue there (`gh issue create -R <owner>/<repo>`). The issue states the cut id, severity, what still happens, the current evidence (file:line, commit, or command output), and the suggested fix. That repo's own agents do the fix. Then resolve the cut with `--note "handed off: <issue URL>"`.
- **No GitHub repo is involved** (machine config, dotfiles, global `~/AGENTS.md`, `~/.local/bin` scripts, system services, installed tools): fix it directly, verify, and resolve with `--note` naming the fix and the verification. Ask before changing production, secrets, or system services.
- **The repo is third-party upstream, has no GitHub remote, or belongs to a dead project** (GitHub issues turned off): do not file anything. Resolve the cut with `--note "Closed without fix: <reason>"`.

Already-fixed or obsolete cuts can be resolved directly, once an independent check confirms them. If a resolution note becomes untrue later (the fix is abandoned, or the cut is handed off), run `papercuts reopen <id> --note "<why>"` and then resolve it again with the correct note. A second `resolve` on its own is a no-op.

## Invariants (do not break)

- Append-only: nothing rewrites the log file, ever. The only bytes added are appends (including the tear-healing `\n`).
- stdout = data only, one envelope; stderr = errors only. `--format md` is the sole raw-output exception.
- Deterministic: same input + `PAPERCUTS_NOW` → byte-identical output.
- Empty results are exit 0. Not-found IDs are 66. Lock timeout is 75 + `retryable:true`.
- Dogfood: when you hit friction working here, `cargo run -- add "..."`.
