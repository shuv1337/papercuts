use crate::{ItemStatus, ListItem, Resolution};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

const K1: f64 = 1.2;
const B: f64 = 0.75;
const TAG_WEIGHT: f64 = 0.15;
const REPO_WEIGHT: f64 = 0.10;
const BM25_SATURATION: f64 = 1.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedMatch {
    pub score: f64,
    #[serde(flatten)]
    pub item: ListItem,
}

#[derive(Debug, Clone)]
pub struct Query<'a> {
    pub text: &'a str,
    pub tags: &'a [String],
    pub repo: Option<&'a str>,
}

impl<'a> Query<'a> {
    pub fn new(text: &'a str, tags: &'a [String], repo: Option<&'a str>) -> Self {
        Self { text, tags, repo }
    }
}

#[derive(Debug, Clone)]
pub struct RankReport {
    pub matches: Vec<RelatedMatch>,
    pub warnings: Vec<String>,
}

/// Lexical BM25 (title/body text) blended with tag-Jaccard and exact-repo
/// boosts. Semantic (embedding-based) ranking was evaluated and dropped: on
/// this tool's small, jargon-dense logs, FastEmbed cosine similarity agreed
/// with BM25 rather than correcting it (a real, unrelated bug and its
/// resolved-but-different neighbor both scored >=0.95, same as lexical-only),
/// while costing ~6s per `add` to embed the whole corpus. Not worth the
/// dependency or the latency; keep this fast and instant instead.
pub fn rank(query: Query<'_>, items: &[ListItem]) -> RankReport {
    RankReport {
        matches: rank_lexical(query, items),
        warnings: Vec::new(),
    }
}

pub fn rank_lexical(query: Query<'_>, items: &[ListItem]) -> Vec<RelatedMatch> {
    if items.is_empty() {
        return Vec::new();
    }

    let query_tokens = tokenize(query.text);
    let query_terms: BTreeSet<_> = query_tokens.into_iter().collect();
    if query_terms.is_empty() && query.tags.is_empty() && query.repo.is_none() {
        return Vec::new();
    }

    let documents: Vec<Vec<String>> = items.iter().map(|item| tokenize(&item.cut.text)).collect();
    let average_len = documents.iter().map(Vec::len).sum::<usize>() as f64 / documents.len() as f64;
    let document_frequency = document_frequencies(&documents);
    let query_tags: BTreeSet<_> = query.tags.iter().map(String::as_str).collect();

    let mut matches: Vec<_> = items
        .iter()
        .zip(&documents)
        .map(|(item, document)| {
            let raw_bm25 = bm25(
                &query_terms,
                document,
                average_len,
                items.len(),
                &document_frequency,
            );
            let lexical_score = normalize_bm25(raw_bm25);
            let tag_score = jaccard_tags(&query_tags, &item.cut.tags);
            let repo_score = exact_repo_match(query.repo, item.cut.repo.as_deref());
            let score =
                (lexical_score + TAG_WEIGHT * tag_score + REPO_WEIGHT * repo_score).min(1.0);
            RelatedMatch {
                score: round_score(score),
                item: item.clone(),
            }
        })
        .filter(|result| result.score > 0.0)
        .collect();

    sort_matches(&mut matches);
    matches
}

pub fn top_matches(
    query: Query<'_>,
    items: &[ListItem],
    limit: usize,
    min_score: f64,
) -> RankReport {
    let mut report = rank(query, items);
    report.matches.retain(|result| result.score >= min_score);
    report.matches.truncate(limit);
    report
}

pub fn normalize_bm25(raw_score: f64) -> f64 {
    if raw_score <= 0.0 {
        0.0
    } else {
        raw_score / (raw_score + BM25_SATURATION)
    }
}

pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn bm25(
    query_terms: &BTreeSet<String>,
    document: &[String],
    average_len: f64,
    corpus_len: usize,
    document_frequency: &HashMap<&str, usize>,
) -> f64 {
    if query_terms.is_empty() || document.is_empty() || average_len == 0.0 {
        return 0.0;
    }

    let mut term_frequency = HashMap::<&str, usize>::new();
    for token in document {
        *term_frequency.entry(token).or_insert(0) += 1;
    }

    let document_len = document.len() as f64;
    query_terms
        .iter()
        .map(|term| {
            let tf = *term_frequency.get(term.as_str()).unwrap_or(&0) as f64;
            if tf == 0.0 {
                return 0.0;
            }
            let df = *document_frequency.get(term.as_str()).unwrap_or(&0) as f64;
            let idf = ((corpus_len as f64 - df + 0.5) / (df + 0.5) + 1.0).ln();
            let denominator = tf + K1 * (1.0 - B + B * document_len / average_len);
            idf * (tf * (K1 + 1.0)) / denominator
        })
        .sum()
}

fn document_frequencies(documents: &[Vec<String>]) -> HashMap<&str, usize> {
    let mut frequencies = HashMap::new();
    for document in documents {
        let terms: HashSet<&str> = document.iter().map(String::as_str).collect();
        for term in terms {
            *frequencies.entry(term).or_insert(0) += 1;
        }
    }
    frequencies
}

fn jaccard_tags(query_tags: &BTreeSet<&str>, document_tags: &[String]) -> f64 {
    if query_tags.is_empty() && document_tags.is_empty() {
        return 0.0;
    }
    let document_tags: BTreeSet<_> = document_tags.iter().map(String::as_str).collect();
    let intersection = query_tags.intersection(&document_tags).count();
    let union = query_tags.union(&document_tags).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn exact_repo_match(query_repo: Option<&str>, document_repo: Option<&str>) -> f64 {
    match (query_repo, document_repo) {
        (Some(query), Some(document)) if query == document => 1.0,
        _ => 0.0,
    }
}

fn round_score(score: f64) -> f64 {
    (score * 1_000_000.0).round() / 1_000_000.0
}

fn status_rank(status: ItemStatus) -> u8 {
    match status {
        ItemStatus::Open => 0,
        ItemStatus::Resolved => 1,
    }
}

fn sort_matches(matches: &mut [RelatedMatch]) {
    matches.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| status_rank(left.item.status).cmp(&status_rank(right.item.status)))
            .then_with(|| left.item.cut.id.cmp(&right.item.cut.id))
    });
}

impl RelatedMatch {
    pub fn resolution(&self) -> Option<&Resolution> {
        self.item.resolution.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CutRecord, Severity};

    fn item(
        id: &str,
        text: &str,
        tags: &[&str],
        repo: Option<&str>,
        status: ItemStatus,
    ) -> ListItem {
        ListItem {
            cut: CutRecord {
                kind: "cut".into(),
                id: id.into(),
                ts: "2026-07-09T00:00:00.000Z".into(),
                agent: "tester".into(),
                text: text.into(),
                tags: tags.iter().map(|tag| (*tag).into()).collect(),
                severity: Severity::Minor,
                cwd: "/tmp".into(),
                repo: repo.map(str::to_owned),
                evidence: None,
            },
            status,
            resolution: (status == ItemStatus::Resolved).then_some(Resolution {
                ts: "2026-07-10T00:00:00.000Z".into(),
                agent: "fixer".into(),
                note: Some("fixed".into()),
            }),
            reopened: None,
        }
    }

    #[test]
    fn tokenization_keeps_technical_terms_without_stopword_removal() {
        assert_eq!(
            tokenize("Yarn web:test can't find apps/web specs"),
            [
                "yarn", "web", "test", "can", "t", "find", "apps", "web", "specs"
            ]
        );
    }

    #[test]
    fn bm25_ranks_lexical_match_above_tag_only_match() {
        let items = vec![
            item(
                "pc_111111111111",
                "cargo clippy fails on workspace lint",
                &["rust"],
                None,
                ItemStatus::Open,
            ),
            item(
                "pc_222222222222",
                "browser screenshot crop is blurry",
                &["rust"],
                None,
                ItemStatus::Open,
            ),
        ];
        let matches = rank_lexical(
            Query::new("cargo clippy workspace", &["rust".into()], None),
            &items,
        );
        assert_eq!(matches[0].item.cut.id, "pc_111111111111");
        assert!(matches[0].score > matches[1].score);
    }

    #[test]
    fn tag_and_repo_boosts_are_bounded_and_deterministic() {
        let items = vec![
            item(
                "pc_111111111111",
                "same lexical issue",
                &["tooling"],
                Some("/repo"),
                ItemStatus::Open,
            ),
            item(
                "pc_222222222222",
                "same lexical issue",
                &["docs"],
                Some("/other"),
                ItemStatus::Open,
            ),
        ];
        let matches = rank_lexical(
            Query::new("same lexical issue", &["tooling".into()], Some("/repo")),
            &items,
        );
        assert!(matches[0].score <= 1.0);
        assert!(matches[1].score <= 1.0);
        assert_eq!(matches[0].item.cut.id, "pc_111111111111");
        assert!(matches[0].score > matches[1].score);
    }

    #[test]
    fn empty_query_with_no_filters_returns_no_matches() {
        let items = vec![item(
            "pc_111111111111",
            "anything",
            &[],
            None,
            ItemStatus::Open,
        )];
        assert!(rank_lexical(Query::new("", &[], None), &items).is_empty());
    }

    #[test]
    fn resolved_and_open_matches_with_equal_score_prefer_open_first() {
        let items = vec![
            item(
                "pc_222222222222",
                "duplicate wording here",
                &[],
                None,
                ItemStatus::Resolved,
            ),
            item(
                "pc_111111111111",
                "duplicate wording here",
                &[],
                None,
                ItemStatus::Open,
            ),
        ];
        let matches = rank_lexical(Query::new("duplicate wording here", &[], None), &items);
        assert_eq!(matches[0].item.cut.id, "pc_111111111111");
    }

    #[test]
    fn top_matches_applies_min_score_and_limit() {
        let items = vec![
            item(
                "pc_111111111111",
                "cargo clippy fails workspace lint",
                &[],
                None,
                ItemStatus::Open,
            ),
            item(
                "pc_222222222222",
                "totally unrelated screenshot text",
                &[],
                None,
                ItemStatus::Open,
            ),
        ];
        let report = top_matches(
            Query::new("cargo clippy workspace lint", &[], None),
            &items,
            1,
            0.1,
        );
        assert_eq!(report.matches.len(), 1);
        assert_eq!(report.matches[0].item.cut.id, "pc_111111111111");
    }
}
