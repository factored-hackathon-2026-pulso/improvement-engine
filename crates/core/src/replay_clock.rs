//! Deterministic, cohort-safe visibility planning for replay campaigns.
//!
//! This module is only the temporal kernel: it does not execute a detector,
//! score outcomes, update memory, or prove a frozen/prequential campaign.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayCase {
    id: String,
    opened_at: i64,
}

impl ReplayCase {
    pub fn new(id: impl Into<String>, opened_at: i64) -> Self {
        Self {
            id: id.into(),
            opened_at,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn opened_at(&self) -> i64 {
        self.opened_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayEvent {
    id: String,
    case_id: String,
    event_at: i64,
    available_at: i64,
    ingested_at: Option<i64>,
}

impl ReplayEvent {
    pub fn new(
        id: impl Into<String>,
        case_id: impl Into<String>,
        event_at: i64,
        available_at: i64,
        ingested_at: Option<i64>,
    ) -> Self {
        Self {
            id: id.into(),
            case_id: case_id.into(),
            event_at,
            available_at,
            ingested_at,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayCohort {
    opened_at: i64,
    case_ids: Vec<String>,
    visible_event_ids: Vec<String>,
}

impl ReplayCohort {
    pub fn opened_at(&self) -> i64 {
        self.opened_at
    }
    pub fn case_ids(&self) -> &[String] {
        &self.case_ids
    }
    pub fn visible_event_ids(&self) -> &[String] {
        &self.visible_event_ids
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayPlan {
    cohorts: Vec<ReplayCohort>,
    availability_profile: ReplayAvailabilityProfile,
}

impl ReplayPlan {
    pub fn cohorts(&self) -> &[ReplayCohort] {
        &self.cohorts
    }
    pub fn availability_profile(&self) -> ReplayAvailabilityProfile {
        self.availability_profile
    }
}

/// Explicit clock provenance; missing ingestion timestamps are never silently
/// interpreted as measured availability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayAvailabilityProfile {
    MeasuredIngestion,
    EventTimeZeroLagAssumption,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplayPlanError {
    EmptyCaseId,
    EmptyEventId,
    DuplicateCaseId(String),
    DuplicateEventId(String),
    UnknownEventCase { event_id: String, case_id: String },
    MissingIngestionTime(String),
}

impl fmt::Display for ReplayPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCaseId => write!(f, "case id must not be empty"),
            Self::EmptyEventId => write!(f, "event id must not be empty"),
            Self::DuplicateCaseId(id) => write!(f, "duplicate case id: {id}"),
            Self::DuplicateEventId(id) => write!(f, "duplicate event id: {id}"),
            Self::UnknownEventCase { event_id, case_id } => {
                write!(f, "event {event_id} references unknown case {case_id}")
            }
            Self::MissingIngestionTime(id) => {
                write!(f, "event {id} is missing measured ingestion time")
            }
        }
    }
}

impl std::error::Error for ReplayPlanError {}

/// Groups ties into indivisible cohorts and assigns each cohort only events
/// from strictly earlier cases that were temporally available by its cutoff.
/// Optional ingestion time is an additional upper bound when present.
pub fn plan_replay(
    cases: Vec<ReplayCase>,
    events: Vec<ReplayEvent>,
    availability_profile: ReplayAvailabilityProfile,
) -> Result<ReplayPlan, ReplayPlanError> {
    let mut cases_by_id = BTreeMap::new();
    let mut cases_by_time: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for case in cases {
        if case.id.trim().is_empty() {
            return Err(ReplayPlanError::EmptyCaseId);
        }
        if cases_by_id
            .insert(case.id.clone(), case.opened_at)
            .is_some()
        {
            return Err(ReplayPlanError::DuplicateCaseId(case.id));
        }
        cases_by_time
            .entry(case.opened_at)
            .or_default()
            .push(case.id);
    }
    for ids in cases_by_time.values_mut() {
        ids.sort();
    }

    let mut event_ids = BTreeSet::new();
    let mut events = events;
    for event in &events {
        if event.id.trim().is_empty() {
            return Err(ReplayPlanError::EmptyEventId);
        }
        if !event_ids.insert(event.id.clone()) {
            return Err(ReplayPlanError::DuplicateEventId(event.id.clone()));
        }
        if availability_profile == ReplayAvailabilityProfile::MeasuredIngestion
            && event.ingested_at.is_none()
        {
            return Err(ReplayPlanError::MissingIngestionTime(event.id.clone()));
        }
        if !cases_by_id.contains_key(&event.case_id) {
            return Err(ReplayPlanError::UnknownEventCase {
                event_id: event.id.clone(),
                case_id: event.case_id.clone(),
            });
        }
    }
    events.sort_by(|a, b| (&a.id, &a.case_id).cmp(&(&b.id, &b.case_id)));

    let mut cohorts: Vec<_> = cases_by_time
        .into_iter()
        .map(|(opened_at, case_ids)| ReplayCohort {
            opened_at,
            case_ids,
            visible_event_ids: Vec::new(),
        })
        .collect();

    // Find each event's first visible cohort by binary search, then expose it
    // to that cohort and all later cohorts. With C cases/cohorts, E events,
    // and V returned event/cohort visibility pairs, sorting, validation,
    // assignment and per-cohort canonicalization cost
    // O(C log C + E log E + E log C + V log E) time and O(C + E + V) memory.
    for event in events {
        let source_opened_at = cases_by_id[&event.case_id];
        let ingestion_cutoff = event.ingested_at.unwrap_or(event.event_at);
        let availability_cutoff = event.available_at.max(ingestion_cutoff).max(event.event_at);
        let first_after_source =
            cohorts.partition_point(|cohort| cohort.opened_at <= source_opened_at);
        let first_at_availability =
            cohorts.partition_point(|cohort| cohort.opened_at < availability_cutoff);
        let first_visible = first_after_source.max(first_at_availability);
        for cohort in cohorts.iter_mut().skip(first_visible) {
            cohort.visible_event_ids.push(event.id.clone());
        }
    }

    for cohort in &mut cohorts {
        cohort.visible_event_ids.sort();
    }

    Ok(ReplayPlan {
        cohorts,
        availability_profile,
    })
}

#[cfg(test)]
mod tests {
    use super::{ReplayAvailabilityProfile as Profile, ReplayCase, ReplayEvent, plan_replay};

    const ASSUMED: Profile = Profile::EventTimeZeroLagAssumption;

    fn case(id: &str, opened_at: i64) -> ReplayCase {
        ReplayCase::new(id, opened_at)
    }

    fn event(id: &str, case_id: &str, event_at: i64, available_at: i64) -> ReplayEvent {
        ReplayEvent::new(id, case_id, event_at, available_at, None)
    }

    #[test]
    fn cases_opened_at_the_same_instant_are_one_cohort() {
        let plan = plan_replay(
            vec![
                case("b", 10),
                case("a", 10),
                case("prior", 9),
                case("c", 11),
            ],
            vec![
                event("ea", "a", 9, 9),
                event("eb", "b", 9, 9),
                event("boundary", "prior", 10, 10),
            ],
            ASSUMED,
        )
        .unwrap();

        assert_eq!(plan.cohorts().len(), 3);
        assert_eq!(plan.cohorts()[1].case_ids(), &["a", "b"]);
        assert_eq!(plan.cohorts()[1].visible_event_ids(), &["boundary"]);
        assert_eq!(
            plan.cohorts()[2].visible_event_ids(),
            &["boundary", "ea", "eb"]
        );
    }

    #[test]
    fn excludes_events_not_yet_available_or_not_yet_ingested() {
        let plan = plan_replay(
            vec![case("prior", 10), case("target", 20)],
            vec![
                event("ready", "prior", 8, 9),
                event("late-available", "prior", 8, 21),
                ReplayEvent::new("late-ingest", "prior", 8, 9, Some(21)),
                event("future-event-time", "prior", 21, 9),
            ],
            ASSUMED,
        )
        .unwrap();

        assert_eq!(plan.cohorts()[1].visible_event_ids(), &["ready"]);
        assert_eq!(plan.availability_profile(), ASSUMED);
    }

    #[test]
    fn rejects_duplicate_case_and_event_ids_and_unknown_event_cases() {
        assert_eq!(
            plan_replay(vec![case("", 1)], vec![], ASSUMED),
            Err(super::ReplayPlanError::EmptyCaseId)
        );
        assert_eq!(
            plan_replay(vec![case("x", 1)], vec![event("", "x", 1, 1)], ASSUMED),
            Err(super::ReplayPlanError::EmptyEventId)
        );
        assert!(plan_replay(vec![case("x", 1), case("x", 2)], vec![], ASSUMED).is_err());
        assert!(
            plan_replay(
                vec![case("x", 1)],
                vec![event("e", "x", 1, 1), event("e", "x", 1, 1)],
                ASSUMED,
            )
            .is_err()
        );
        assert!(
            plan_replay(
                vec![case("x", 1)],
                vec![event("e", "missing", 1, 1)],
                ASSUMED
            )
            .is_err()
        );
        assert_eq!(
            plan_replay(
                vec![case("x", 1)],
                vec![event("missing-ingest", "x", 1, 1)],
                Profile::MeasuredIngestion,
            ),
            Err(super::ReplayPlanError::MissingIngestionTime(
                "missing-ingest".into()
            ))
        );
    }

    #[test]
    fn output_is_stable_independent_of_input_order() {
        let left = plan_replay(
            vec![case("z", 20), case("b", 10), case("a", 10)],
            vec![event("z-event", "b", 9, 9), event("a-event", "a", 9, 9)],
            ASSUMED,
        )
        .unwrap();
        let right = plan_replay(
            vec![case("a", 10), case("b", 10), case("z", 20)],
            vec![event("a-event", "a", 9, 9), event("z-event", "b", 9, 9)],
            ASSUMED,
        )
        .unwrap();
        assert_eq!(left, right);
    }
}
