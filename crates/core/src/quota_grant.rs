//! Deterministic quota reservation and governed-grant boundary.
//!
//! This module is intentionally an in-process model of the semantics that the
//! later durable control-plane adapter must preserve transactionally. It does
//! not issue policy itself, persist state, inspect bank data, call a model, or
//! execute Agent Core. A caller supplies a grant already authorized by the
//! external policy authority and receives a deterministic receipt.

use crate::run_config::{RunConfig, RunConfigIdentity};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct QuotaResource(String);

impl QuotaResource {
    pub fn new(value: impl Into<String>) -> Result<Self, QuotaGrantError> {
        let value = value.into();
        validate_identifier(&value, "resource")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A stable global quota key. It deliberately excludes `RunConfigIdentity`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct QuotaWindow {
    tenant_id: String,
    resource: QuotaResource,
    start_unix_seconds: u64,
    end_unix_seconds: u64,
}

/// Identity of a globally shared quota bucket. The closing boundary is
/// metadata, not identity: changing it cannot create a second bucket for the
/// same calendar/window start.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct QuotaWindowKey {
    tenant_id: String,
    resource: QuotaResource,
    start_unix_seconds: u64,
}

impl From<&QuotaWindow> for QuotaWindowKey {
    fn from(window: &QuotaWindow) -> Self {
        Self {
            tenant_id: window.tenant_id.clone(),
            resource: window.resource.clone(),
            start_unix_seconds: window.start_unix_seconds,
        }
    }
}

/// The policy-controlled capacity shared by every config revision in a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaLimit {
    window: QuotaWindow,
    limit_units: u64,
}

impl QuotaLimit {
    pub fn new(window: QuotaWindow, limit_units: u64) -> Result<Self, QuotaGrantError> {
        if limit_units == 0 {
            return Err(QuotaGrantError::QuotaLimitMustBePositive);
        }
        Ok(Self {
            window,
            limit_units,
        })
    }
}

impl QuotaWindow {
    pub fn new(
        tenant_id: impl Into<String>,
        resource: QuotaResource,
        start_unix_seconds: u64,
        end_unix_seconds: u64,
    ) -> Result<Self, QuotaGrantError> {
        let tenant_id = tenant_id.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        if start_unix_seconds >= end_unix_seconds {
            return Err(QuotaGrantError::InvalidWindow);
        }
        Ok(Self {
            tenant_id,
            resource,
            start_unix_seconds,
            end_unix_seconds,
        })
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn resource(&self) -> &QuotaResource {
        &self.resource
    }

    pub fn start_unix_seconds(&self) -> u64 {
        self.start_unix_seconds
    }

    pub fn end_unix_seconds(&self) -> u64 {
        self.end_unix_seconds
    }
}

/// A policy-authorized delegation. `authority_ref` is a proof reference, not
/// a local assertion that this crate can mint permissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedGrant {
    grant_id: String,
    authority_ref: String,
    tenant_id: String,
    resource: QuotaResource,
    max_units: u64,
    expires_at_unix_seconds: u64,
}

/// The complete namespace of a policy delegation. Grant IDs are only unique
/// inside this scope; treating the raw ID as globally unique would allow one
/// tenant or authority to consume or revoke another's delegation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GrantScopeKey {
    tenant_id: String,
    authority_ref: String,
    grant_id: String,
}

impl From<&AuthorizedGrant> for GrantScopeKey {
    fn from(grant: &AuthorizedGrant) -> Self {
        Self {
            tenant_id: grant.tenant_id.clone(),
            authority_ref: grant.authority_ref.clone(),
            grant_id: grant.grant_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct IdempotencyScopeKey {
    tenant_id: String,
    idempotency_key: String,
}

impl AuthorizedGrant {
    pub fn issue(
        grant_id: impl Into<String>,
        authority_ref: impl Into<String>,
        tenant_id: impl Into<String>,
        resource: QuotaResource,
        max_units: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self, QuotaGrantError> {
        let grant_id = grant_id.into();
        let authority_ref = authority_ref.into();
        let tenant_id = tenant_id.into();
        validate_identifier(&grant_id, "grant_id")?;
        validate_identifier(&authority_ref, "authority_ref")?;
        validate_identifier(&tenant_id, "tenant_id")?;
        if max_units == 0 {
            return Err(QuotaGrantError::GrantUnitsMustBePositive);
        }
        if expires_at_unix_seconds == 0 {
            return Err(QuotaGrantError::GrantExpiryMustBePositive);
        }
        Ok(Self {
            grant_id,
            authority_ref,
            tenant_id,
            resource,
            max_units,
            expires_at_unix_seconds,
        })
    }

    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }
}

/// A request whose idempotency key belongs to exactly one immutable payload.
#[derive(Debug, Clone)]
pub struct QuotaReservation {
    idempotency_key: String,
    window: QuotaWindow,
    config_identity: RunConfigIdentity,
    grant: AuthorizedGrant,
    requested_units: u64,
    requested_at_unix_seconds: u64,
}

impl QuotaReservation {
    pub fn new(
        idempotency_key: impl Into<String>,
        window: QuotaWindow,
        config: &RunConfig,
        grant: AuthorizedGrant,
        requested_units: u64,
        requested_at_unix_seconds: u64,
    ) -> Result<Self, QuotaGrantError> {
        let idempotency_key = idempotency_key.into();
        validate_identifier(&idempotency_key, "idempotency_key")?;
        if requested_units == 0 {
            return Err(QuotaGrantError::RequestedUnitsMustBePositive);
        }
        if requested_at_unix_seconds < window.start_unix_seconds
            || requested_at_unix_seconds >= window.end_unix_seconds
        {
            return Err(QuotaGrantError::ReservationOutsideWindow);
        }
        Ok(Self {
            idempotency_key,
            window,
            config_identity: config.identity().clone(),
            grant,
            requested_units,
            requested_at_unix_seconds,
        })
    }

    /// The caller-owned trigger key that scopes this reservation's idempotency.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    pub fn tenant_id(&self) -> &str {
        &self.window.tenant_id
    }

    pub fn config_identity(&self) -> &RunConfigIdentity {
        &self.config_identity
    }

    /// Stable request identity, exposed so a compound durable reducer can
    /// reject a changed quota payload before it returns an old job receipt.
    pub fn request_digest(&self) -> String {
        reservation_digest(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaReceiptOutcome {
    Reserved,
    DeferredQuotaExhausted,
    DeferredGrantExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaReceipt {
    receipt_id: String,
    outcome: QuotaReceiptOutcome,
    window: QuotaWindow,
    config_identity: RunConfigIdentity,
    grant_id: String,
    requested_units: u64,
    reserved_units_after: u64,
    grant_units_after: u64,
}

impl QuotaReceipt {
    pub fn receipt_id(&self) -> &str {
        &self.receipt_id
    }

    pub fn is_reserved(&self) -> bool {
        self.outcome == QuotaReceiptOutcome::Reserved
    }

    pub fn is_deferred_for_quota(&self) -> bool {
        self.outcome == QuotaReceiptOutcome::DeferredQuotaExhausted
    }

    pub fn config_identity(&self) -> &RunConfigIdentity {
        &self.config_identity
    }

    pub fn outcome(&self) -> &QuotaReceiptOutcome {
        &self.outcome
    }
}

#[derive(Debug, Default)]
pub struct QuotaLedger {
    windows: BTreeMap<QuotaWindowKey, WindowBalance>,
    grant_usage: BTreeMap<GrantScopeKey, u64>,
    revoked_grants: BTreeMap<GrantScopeKey, GrantRevocationReceipt>,
    idempotency: BTreeMap<IdempotencyScopeKey, IdempotentReservation>,
}

#[derive(Debug, Clone)]
struct WindowBalance {
    limit_units: u64,
    reserved_units: u64,
    end_unix_seconds: u64,
}

#[derive(Debug, Clone)]
struct IdempotentReservation {
    request_digest: String,
    receipt: QuotaReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRevocationReceipt {
    receipt_id: String,
    grant_id: String,
    tenant_id: String,
    authority_ref: String,
    revoked_at_unix_seconds: u64,
    reason_code: String,
}

impl GrantRevocationReceipt {
    pub fn receipt_id(&self) -> &str {
        &self.receipt_id
    }
}

impl QuotaLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs the effective, externally-approved limit before admission.
    /// Updating a limit after the window has reservations is rejected; a
    /// future durable policy revision must instead make an explicit rule about
    /// lowering limits while retaining existing consumption.
    pub fn configure_limit(&mut self, limit: QuotaLimit) -> Result<(), QuotaGrantError> {
        let window_key = QuotaWindowKey::from(&limit.window);
        if let Some(existing) = self.windows.get(&window_key) {
            if existing.end_unix_seconds != limit.window.end_unix_seconds {
                return Err(window_end_mismatch(
                    &limit.window,
                    existing.end_unix_seconds,
                ));
            }
            return Err(QuotaGrantError::QuotaWindowAlreadyConfigured);
        }
        self.windows.insert(
            window_key,
            WindowBalance {
                limit_units: limit.limit_units,
                reserved_units: 0,
                end_unix_seconds: limit.window.end_unix_seconds,
            },
        );
        Ok(())
    }

    /// Revocation is idempotent for the same immutable revocation event. A
    /// conflicting second event is rejected rather than silently replacing it.
    pub fn revoke_grant(
        &mut self,
        tenant_id: impl Into<String>,
        authority_ref: impl Into<String>,
        grant_id: impl Into<String>,
        revoked_at_unix_seconds: u64,
        reason_code: impl Into<String>,
    ) -> Result<GrantRevocationReceipt, QuotaGrantError> {
        let tenant_id = tenant_id.into();
        let authority_ref = authority_ref.into();
        let grant_id = grant_id.into();
        let reason_code = reason_code.into();
        validate_identifier(&tenant_id, "tenant_id")?;
        validate_identifier(&authority_ref, "authority_ref")?;
        validate_identifier(&grant_id, "grant_id")?;
        validate_identifier(&reason_code, "reason_code")?;
        if revoked_at_unix_seconds == 0 {
            return Err(QuotaGrantError::InvalidRevocationTime);
        }

        let grant_scope = GrantScopeKey {
            tenant_id: tenant_id.clone(),
            authority_ref: authority_ref.clone(),
            grant_id: grant_id.clone(),
        };
        let receipt = GrantRevocationReceipt {
            receipt_id: digest_receipt(&[
                ("kind", "grant_revocation"),
                ("tenant_id", &tenant_id),
                ("authority_ref", &authority_ref),
                ("grant_id", &grant_id),
                ("revoked_at", &revoked_at_unix_seconds.to_string()),
                ("reason_code", &reason_code),
            ]),
            grant_id: grant_id.clone(),
            tenant_id,
            authority_ref,
            revoked_at_unix_seconds,
            reason_code,
        };
        if let Some(existing) = self.revoked_grants.get(&grant_scope) {
            if existing == &receipt {
                return Ok(existing.clone());
            }
            return Err(QuotaGrantError::ConflictingGrantRevocation { grant_id });
        }
        self.revoked_grants.insert(grant_scope, receipt.clone());
        Ok(receipt)
    }

    /// Atomically reserves units in this in-memory boundary. A durable adapter
    /// must perform the equivalent check-and-update under the quota-window row
    /// lock described in the tech spec.
    pub fn reserve(
        &mut self,
        reservation: QuotaReservation,
    ) -> Result<QuotaReceipt, QuotaGrantError> {
        let request_digest = reservation_digest(&reservation);
        let idempotency_scope = IdempotencyScopeKey {
            tenant_id: reservation.window.tenant_id.clone(),
            idempotency_key: reservation.idempotency_key.clone(),
        };
        if let Some(existing) = self.idempotency.get(&idempotency_scope) {
            if existing.request_digest == request_digest {
                return Ok(existing.receipt.clone());
            }
            return Err(QuotaGrantError::IdempotencyConflict {
                idempotency_key: reservation.idempotency_key,
            });
        }
        self.validate_grant(&reservation)?;

        let grant_scope = GrantScopeKey::from(&reservation.grant);
        let grant_used = self.grant_usage.get(&grant_scope).copied().unwrap_or(0);
        let balance = self
            .windows
            .get_mut(&QuotaWindowKey::from(&reservation.window))
            .ok_or_else(|| QuotaGrantError::QuotaWindowNotConfigured {
                tenant_id: reservation.window.tenant_id.clone(),
                resource: reservation.window.resource.as_str().to_owned(),
            })?;
        if balance.end_unix_seconds != reservation.window.end_unix_seconds {
            return Err(window_end_mismatch(
                &reservation.window,
                balance.end_unix_seconds,
            ));
        }

        let outcome = if reservation.requested_units
            > reservation.grant.max_units.saturating_sub(grant_used)
        {
            QuotaReceiptOutcome::DeferredGrantExhausted
        } else if reservation.requested_units
            > balance.limit_units.saturating_sub(balance.reserved_units)
        {
            QuotaReceiptOutcome::DeferredQuotaExhausted
        } else {
            balance.reserved_units += reservation.requested_units;
            self.grant_usage.insert(
                grant_scope.clone(),
                grant_used + reservation.requested_units,
            );
            QuotaReceiptOutcome::Reserved
        };
        let grant_units_after = self
            .grant_usage
            .get(&grant_scope)
            .copied()
            .unwrap_or(grant_used);
        let receipt = QuotaReceipt {
            receipt_id: receipt_for(
                &reservation,
                &outcome,
                balance.reserved_units,
                grant_units_after,
            ),
            outcome,
            window: reservation.window,
            config_identity: reservation.config_identity,
            grant_id: reservation.grant.grant_id,
            requested_units: reservation.requested_units,
            reserved_units_after: balance.reserved_units,
            grant_units_after,
        };
        self.idempotency.insert(
            idempotency_scope,
            IdempotentReservation {
                request_digest,
                receipt: receipt.clone(),
            },
        );
        Ok(receipt)
    }

    pub fn reserved_units(&self, window: &QuotaWindow) -> u64 {
        self.windows
            .get(&QuotaWindowKey::from(window))
            .map_or(0, |balance| balance.reserved_units)
    }

    fn validate_grant(&self, reservation: &QuotaReservation) -> Result<(), QuotaGrantError> {
        let grant = &reservation.grant;
        if grant.tenant_id != reservation.window.tenant_id {
            return Err(QuotaGrantError::GrantTenantMismatch);
        }
        if grant.resource != reservation.window.resource {
            return Err(QuotaGrantError::GrantResourceMismatch);
        }
        if reservation.requested_at_unix_seconds >= grant.expires_at_unix_seconds {
            return Err(QuotaGrantError::GrantExpired {
                grant_id: grant.grant_id.clone(),
            });
        }
        if let Some(revocation) = self.revoked_grants.get(&GrantScopeKey::from(grant))
            && reservation.requested_at_unix_seconds >= revocation.revoked_at_unix_seconds
        {
            return Err(QuotaGrantError::GrantRevoked {
                grant_id: grant.grant_id.clone(),
                revocation_receipt_id: revocation.receipt_id.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaGrantError {
    InvalidIdentifier {
        field: &'static str,
    },
    InvalidWindow,
    GrantUnitsMustBePositive,
    QuotaLimitMustBePositive,
    GrantExpiryMustBePositive,
    RequestedUnitsMustBePositive,
    ReservationOutsideWindow,
    InvalidRevocationTime,
    GrantTenantMismatch,
    GrantResourceMismatch,
    GrantExpired {
        grant_id: String,
    },
    GrantRevoked {
        grant_id: String,
        revocation_receipt_id: String,
    },
    ConflictingGrantRevocation {
        grant_id: String,
    },
    IdempotencyConflict {
        idempotency_key: String,
    },
    QuotaWindowAlreadyConfigured,
    QuotaWindowNotConfigured {
        tenant_id: String,
        resource: String,
    },
    QuotaWindowEndMismatch {
        tenant_id: String,
        resource: String,
        start_unix_seconds: u64,
        expected_end_unix_seconds: u64,
        actual_end_unix_seconds: u64,
    },
}

impl fmt::Display for QuotaGrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field } => {
                write!(formatter, "{field} must be a lowercase identifier")
            }
            Self::InvalidWindow => formatter.write_str("quota window start must be before end"),
            Self::GrantUnitsMustBePositive => {
                formatter.write_str("grant max units must be positive")
            }
            Self::QuotaLimitMustBePositive => formatter.write_str("quota limit must be positive"),
            Self::GrantExpiryMustBePositive => formatter.write_str("grant expiry must be positive"),
            Self::RequestedUnitsMustBePositive => {
                formatter.write_str("requested units must be positive")
            }
            Self::ReservationOutsideWindow => {
                formatter.write_str("reservation time is outside quota window")
            }
            Self::InvalidRevocationTime => formatter.write_str("revocation time must be positive"),
            Self::GrantTenantMismatch => {
                formatter.write_str("grant tenant does not match quota window")
            }
            Self::GrantResourceMismatch => {
                formatter.write_str("grant resource does not match quota window")
            }
            Self::GrantExpired { grant_id } => write!(formatter, "grant {grant_id} is expired"),
            Self::GrantRevoked { grant_id, .. } => write!(formatter, "grant {grant_id} is revoked"),
            Self::ConflictingGrantRevocation { grant_id } => write!(
                formatter,
                "grant {grant_id} already has a different revocation"
            ),
            Self::IdempotencyConflict { idempotency_key } => write!(
                formatter,
                "idempotency key {idempotency_key} has a different reservation"
            ),
            Self::QuotaWindowAlreadyConfigured => {
                formatter.write_str("quota window is already configured")
            }
            Self::QuotaWindowNotConfigured {
                tenant_id,
                resource,
            } => {
                write!(
                    formatter,
                    "quota window for {tenant_id}.{resource} is not configured"
                )
            }
            Self::QuotaWindowEndMismatch {
                tenant_id,
                resource,
                start_unix_seconds,
                expected_end_unix_seconds,
                actual_end_unix_seconds,
            } => write!(
                formatter,
                "quota window {tenant_id}.{resource} starting at {start_unix_seconds} expects end {expected_end_unix_seconds}, not {actual_end_unix_seconds}"
            ),
        }
    }
}

fn window_end_mismatch(window: &QuotaWindow, expected_end_unix_seconds: u64) -> QuotaGrantError {
    QuotaGrantError::QuotaWindowEndMismatch {
        tenant_id: window.tenant_id.clone(),
        resource: window.resource.as_str().to_owned(),
        start_unix_seconds: window.start_unix_seconds,
        expected_end_unix_seconds,
        actual_end_unix_seconds: window.end_unix_seconds,
    }
}

impl std::error::Error for QuotaGrantError {}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), QuotaGrantError> {
    let mut characters = value.chars();
    if !matches!(characters.next(), Some(character) if character.is_ascii_lowercase())
        || !characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '_'
                || character == '-'
        })
    {
        return Err(QuotaGrantError::InvalidIdentifier { field });
    }
    Ok(())
}

fn reservation_digest(reservation: &QuotaReservation) -> String {
    digest_receipt(&[
        ("kind", "quota_reservation_request"),
        ("idempotency_key", &reservation.idempotency_key),
        ("tenant_id", &reservation.window.tenant_id),
        ("resource", reservation.window.resource.as_str()),
        (
            "window_start",
            &reservation.window.start_unix_seconds.to_string(),
        ),
        (
            "window_end",
            &reservation.window.end_unix_seconds.to_string(),
        ),
        ("config_identity", reservation.config_identity.as_str()),
        ("grant_id", &reservation.grant.grant_id),
        ("authority_ref", &reservation.grant.authority_ref),
        ("grant_max_units", &reservation.grant.max_units.to_string()),
        (
            "grant_expires_at",
            &reservation.grant.expires_at_unix_seconds.to_string(),
        ),
        ("requested_units", &reservation.requested_units.to_string()),
        (
            "requested_at",
            &reservation.requested_at_unix_seconds.to_string(),
        ),
    ])
}

fn receipt_for(
    reservation: &QuotaReservation,
    outcome: &QuotaReceiptOutcome,
    reserved_units_after: u64,
    grant_units_after: u64,
) -> String {
    let outcome = match outcome {
        QuotaReceiptOutcome::Reserved => "reserved",
        QuotaReceiptOutcome::DeferredQuotaExhausted => "deferred_quota_exhausted",
        QuotaReceiptOutcome::DeferredGrantExhausted => "deferred_grant_exhausted",
    };
    digest_receipt(&[
        ("kind", "quota_receipt"),
        ("request_digest", &reservation_digest(reservation)),
        ("outcome", outcome),
        ("reserved_units_after", &reserved_units_after.to_string()),
        ("grant_units_after", &grant_units_after.to_string()),
    ])
}

fn digest_receipt(parts: &[(&str, &str)]) -> String {
    let mut canonical = String::new();
    for (name, value) in parts {
        canonical.push_str(name);
        canonical.push(':');
        canonical.push_str(&value.len().to_string());
        canonical.push(':');
        canonical.push_str(value);
        canonical.push('\n');
    }
    format!(
        "quota-receipt:sha256:{:x}",
        Sha256::digest(canonical.as_bytes())
    )
}
