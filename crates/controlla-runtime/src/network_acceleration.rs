//! Safe first-party network candidates for read acceleration.
//!
//! This module only classifies already-observed responses. It never performs a
//! new request, forwards cookies, or treats a candidate as verified content.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkCandidate {
    pub url: String,
    pub method: String,
    pub mime_type: String,
    pub status: u16,
    pub document_id: String,
    pub body_handle: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateDecision {
    Eligible,
    Ineligible,
}

pub fn classify(candidate: &NetworkCandidate, page_origin: &str) -> CandidateDecision {
    let Ok(url) = url::Url::parse(&candidate.url) else {
        return CandidateDecision::Ineligible;
    };
    if url.origin().ascii_serialization() != page_origin
        || candidate.method != "GET"
        || !(200..300).contains(&candidate.status)
        || !candidate.mime_type.to_ascii_lowercase().contains("json")
        || candidate.body_handle.trim().is_empty()
    {
        return CandidateDecision::Ineligible;
    }
    CandidateDecision::Eligible
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate() -> NetworkCandidate {
        NetworkCandidate {
            url: "https://example.test/api/items".into(),
            method: "GET".into(),
            mime_type: "application/json".into(),
            status: 200,
            document_id: "d".into(),
            body_handle: "a1".into(),
        }
    }
    #[test]
    fn same_origin_json_get_is_eligible() {
        assert_eq!(
            classify(&candidate(), "https://example.test"),
            CandidateDecision::Eligible
        );
    }
    #[test]
    fn cross_origin_or_mutating_candidates_are_ineligible() {
        let mut c = candidate();
        c.method = "POST".into();
        assert_eq!(
            classify(&c, "https://example.test"),
            CandidateDecision::Ineligible
        );
    }
}
