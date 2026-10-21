//! `OwnKeysEnricher`: a reserved stub (P3-ENR-05). The full implementation
//! (People Data Labs, Apollo, Clearbit, a generic email verifier) is
//! unscheduled; see this crate's README. It exists so `enrich.backend =
//! "ownkeys"` is a known value that fails clearly instead of silently.

use crate::{EnrichError, EnrichJob, EnrichResult, Enricher, QuoteLine};

const NOT_YET: &str =
    "the ownkeys backend is reserved but not implemented yet; use enrich.backend = \"treg\" or \"none\"";

/// See the module docs. Supports nothing; every call is `Unsupported`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OwnKeysEnricher;

impl Enricher for OwnKeysEnricher {
    fn name(&self) -> &'static str {
        "ownkeys"
    }
    fn supports(&self, _op: &str) -> bool {
        false
    }
    fn quote(&self, _jobs: &[EnrichJob]) -> Result<Vec<QuoteLine>, EnrichError> {
        Err(EnrichError::Unsupported(NOT_YET.to_string()))
    }
    fn enrich(&self, _job: &EnrichJob) -> Result<EnrichResult, EnrichError> {
        Err(EnrichError::Unsupported(NOT_YET.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EnrichRequest;

    #[test]
    fn stub_refuses_everything_without_egress() {
        let e = OwnKeysEnricher;
        let job = EnrichJob {
            person_id: None,
            request: EnrichRequest::Company {
                domain: "example.com".into(),
            },
        };
        assert_eq!(e.name(), "ownkeys");
        assert!(!e.supports("company"));
        match e.quote(std::slice::from_ref(&job)) {
            Err(EnrichError::Unsupported(m)) => assert!(m.contains("ownkeys"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(e.enrich(&job), Err(EnrichError::Unsupported(_))));
    }
}
