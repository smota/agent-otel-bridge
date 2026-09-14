//! Explicit backend visibility checks. A captured OTLP document is not a
//! backend query and therefore cannot produce a visible state.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityState {
    NotChecked,
    VisiblePartial,
    VisibleComplete,
    NotFound,
    QueryFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityQuery {
    pub trace_id: String,
    pub start_unix_nanos: u128,
    pub end_unix_nanos: u128,
    pub expected_span_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityResult {
    pub state: VisibilityState,
    pub checked_at_unix_nanos: Option<u128>,
    pub window_start_unix_nanos: u128,
    pub window_end_unix_nanos: u128,
    pub expected_span_ids: Vec<String>,
    pub observed_span_ids: Vec<String>,
    pub missing_span_ids: Vec<String>,
    pub source: String,
    pub error: Option<String>,
}

/// Implementations must perform an actual query against their configured
/// backend. File imports and local captures should not implement this trait.
pub trait VisibilityBackend {
    fn query(&self, query: &VisibilityQuery) -> Result<Vec<String>, String>;
    fn source(&self) -> &str {
        "configured.backend"
    }
}

pub fn not_checked(query: &VisibilityQuery) -> VisibilityResult {
    VisibilityResult {
        state: VisibilityState::NotChecked,
        checked_at_unix_nanos: None,
        window_start_unix_nanos: query.start_unix_nanos,
        window_end_unix_nanos: query.end_unix_nanos,
        expected_span_ids: query.expected_span_ids.clone(),
        observed_span_ids: Vec::new(),
        missing_span_ids: query.expected_span_ids.clone(),
        source: "none".into(),
        error: None,
    }
}

pub fn query_visibility<B: VisibilityBackend>(
    backend: &B,
    query: &VisibilityQuery,
    checked_at_unix_nanos: u128,
) -> VisibilityResult {
    let expected: BTreeSet<_> = query.expected_span_ids.iter().cloned().collect();
    match backend.query(query) {
        Ok(ids) => {
            let observed: Vec<_> = ids.into_iter().filter(|id| expected.contains(id)).collect();
            let observed_set: BTreeSet<_> = observed.iter().cloned().collect();
            let missing: Vec<_> = expected.difference(&observed_set).cloned().collect();
            let state = if observed.is_empty() {
                VisibilityState::NotFound
            } else if missing.is_empty() {
                VisibilityState::VisibleComplete
            } else {
                VisibilityState::VisiblePartial
            };
            VisibilityResult {
                state,
                checked_at_unix_nanos: Some(checked_at_unix_nanos),
                window_start_unix_nanos: query.start_unix_nanos,
                window_end_unix_nanos: query.end_unix_nanos,
                expected_span_ids: query.expected_span_ids.clone(),
                observed_span_ids: observed,
                missing_span_ids: missing,
                source: backend.source().into(),
                error: None,
            }
        }
        Err(error) => VisibilityResult {
            state: VisibilityState::QueryFailed,
            checked_at_unix_nanos: Some(checked_at_unix_nanos),
            window_start_unix_nanos: query.start_unix_nanos,
            window_end_unix_nanos: query.end_unix_nanos,
            expected_span_ids: query.expected_span_ids.clone(),
            observed_span_ids: Vec::new(),
            missing_span_ids: query.expected_span_ids.clone(),
            source: backend.source().into(),
            error: Some(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        ids: Result<Vec<String>, String>,
    }
    impl VisibilityBackend for Fake {
        fn query(&self, _: &VisibilityQuery) -> Result<Vec<String>, String> {
            self.ids.clone()
        }
        fn source(&self) -> &str {
            "fake.backend"
        }
    }
    fn q() -> VisibilityQuery {
        VisibilityQuery {
            trace_id: "a".into(),
            start_unix_nanos: 1,
            end_unix_nanos: 2,
            expected_span_ids: vec!["one".into(), "two".into()],
        }
    }
    #[test]
    fn fake_query_reports_partial_and_complete() {
        let partial = query_visibility(
            &Fake {
                ids: Ok(vec!["one".into()]),
            },
            &q(),
            3,
        );
        assert_eq!(partial.state, VisibilityState::VisiblePartial);
        let complete = query_visibility(
            &Fake {
                ids: Ok(vec!["one".into(), "two".into()]),
            },
            &q(),
            3,
        );
        assert_eq!(complete.state, VisibilityState::VisibleComplete);
    }
    #[test]
    fn errors_are_query_failed_and_capture_is_not_a_query() {
        assert_eq!(not_checked(&q()).state, VisibilityState::NotChecked);
        let result = query_visibility(
            &Fake {
                ids: Err("offline".into()),
            },
            &q(),
            3,
        );
        assert_eq!(result.state, VisibilityState::QueryFailed);
    }
}
