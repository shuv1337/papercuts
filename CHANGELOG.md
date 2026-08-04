# Changelog

## [0.5.0] - 2026-08-20

### Added

- Add `papercuts related "text"` to rank open and resolved cuts by lexical BM25 relevance with small tag/repo boosts.

### Changed

- Make fuzzy matches on `papercuts add` advisory-only: new content is always appended after the exact-ID duplicate check, with related matches and possible resolved fixes returned informationally (never blocking).
- Repurpose `papercuts add --no-check` to skip advisory related-match computation only.

Note: an earlier iteration of this release added FastEmbed-based semantic scoring fused with BM25 via RRF. It was evaluated against the production log and dropped before shipping: on this tool's small, jargon-dense corpus, embedding similarity tracked BM25 rather than correcting it (two topically-distinct cuts sharing vocabulary still scored >=0.95 either way), while adding a real ~90MB model dependency and ~6s of latency per `add`. Not worth it; `related` stays pure lexical BM25 + tag Jaccard + repo match, unchanged from the original plan.

## [0.3.0] - 2026-08-20 (superseded, folded into 0.4.0 above)

### Added

- Add `papercuts related "text"` to rank open and resolved cuts by lexical BM25 relevance with small tag/repo boosts.
- Add fuzzy `papercuts add` pre-flight checking: resolved high-confidence matches refuse to append and return the prior resolution, while open/lower-confidence matches are returned as non-blocking `related` suggestions.
- Add `papercuts add --no-check` to bypass fuzzy pre-flight while keeping exact-ID duplicate protection.

## [0.2.0] - 2026-07-16

### Added

- Attach bounded evidence to `add` with `--cmd`, `--exit`, `--stderr-file`, and `--evidence`.
- Resolve multiple IDs or unique prefixes atomically in one `resolve` command.

### Changed

- Redact common credential assignments, authorization values, high-entropy tokens, and URL userinfo before evidence is returned or stored. Redaction remains best-effort.
- Reject non-regular, non-UTF-8, and larger-than-1-MiB stderr inputs; truncate sanitized stored stderr to 4096 UTF-8 bytes.
- Preserve the single-ID resolve response while returning sorted, deduplicated records for multi-ID resolves and warnings for already-resolved IDs.
- Expand `schema` with the evidence record and multi-ID resolve contracts.
- Expand doctor/fold test coverage for malformed records, duplicate cuts, ID conflicts, orphan resolves, conflict markers, torn lines, and append recovery.
- Limit the published crate to source, tests, and release documentation.

### Fixed

- Roll back failed batch appends so partial multi-resolve writes do not corrupt the append-only log.
- Accept leading-hyphen values for evidence and resolution notes without swallowing later options.
- Preserve paths and URLs during best-effort credential redaction while covering token-only URL userinfo and lowercase compound credential keys.

## [0.1.0] - 2026-07-10

- Initial release.

[0.5.0]: https://github.com/treygoff24/papercuts/compare/v0.3.0...v0.5.0
[0.3.0]: https://github.com/treygoff24/papercuts/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/treygoff24/papercuts/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/treygoff24/papercuts/releases/tag/v0.1.0
