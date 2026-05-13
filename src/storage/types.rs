//! Public types stored in `pcp_records`.

use serde::{Deserialize, Serialize};

use crate::storage::error::StorageError;

/// Opaque server-assigned signup identifier. Sensitive per oxide's logging
/// rules; do not write to logs.
pub type SignupId = String;

/// Shard identifier within a signup. `0` for untiered PCPs; `0..N` for
/// tiered PCPs.
pub type Tier = u8;

/// Lifecycle state of a single PCP row. Stored as TEXT to match oxide's
/// JSON serialisation and to keep migration byte-trivial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum PackageStatus {
    Downloaded,
    EnrollmentRequestInitiated,
    EnrollmentRequestSuccess,
    Enrolled,
    EnrollmentAbandoned,
    EnrollmentFailed,
    Unverified,
}

impl PackageStatus {
    /// Stable string used as the SQL `TEXT` value. Matches oxide's
    /// `serde_json` serialisation of the `PackageStatus` enum.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Downloaded => "Downloaded",
            Self::EnrollmentRequestInitiated => "EnrollmentRequestInitiated",
            Self::EnrollmentRequestSuccess => "EnrollmentRequestSuccess",
            Self::Enrolled => "Enrolled",
            Self::EnrollmentAbandoned => "EnrollmentAbandoned",
            Self::EnrollmentFailed => "EnrollmentFailed",
            Self::Unverified => "Unverified",
        }
    }

    /// Parse a `TEXT` value back into the enum.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidState`] if the string is not one of
    /// the seven valid values.
    pub fn parse(s: &str) -> Result<Self, StorageError> {
        match s {
            "Downloaded" => Ok(Self::Downloaded),
            "EnrollmentRequestInitiated" => Ok(Self::EnrollmentRequestInitiated),
            "EnrollmentRequestSuccess" => Ok(Self::EnrollmentRequestSuccess),
            "Enrolled" => Ok(Self::Enrolled),
            "EnrollmentAbandoned" => Ok(Self::EnrollmentAbandoned),
            "EnrollmentFailed" => Ok(Self::EnrollmentFailed),
            "Unverified" => Ok(Self::Unverified),
            other => Err(StorageError::InvalidState(format!(
                "unknown PackageStatus: {other}"
            ))),
        }
    }

    /// Whether `next` is a legal transition from `self`.
    ///
    /// Used by [`crate::storage::OrbPcpStore::update_status`] to reject
    /// illegal flips at the boundary.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Downloaded,
                Self::EnrollmentRequestInitiated | Self::EnrollmentAbandoned,
            ) | (
                Self::EnrollmentRequestInitiated,
                Self::EnrollmentRequestSuccess
                    | Self::EnrollmentFailed
                    | Self::EnrollmentAbandoned,
            ) | (
                Self::EnrollmentRequestSuccess,
                Self::Enrolled | Self::EnrollmentFailed,
            ) | (Self::Enrolled, Self::Unverified)
        )
    }
}

/// How a PCP row came to exist on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum CreationSource {
    UserCentricEnrollment,
    UserCentricReEnrollment,
    ReAuthentication,
    Sync,
    CredentialRecovery,
}

impl CreationSource {
    /// Stable string used as the SQL `TEXT` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserCentricEnrollment => "UserCentricEnrollment",
            Self::UserCentricReEnrollment => "UserCentricReEnrollment",
            Self::ReAuthentication => "ReAuthentication",
            Self::Sync => "Sync",
            Self::CredentialRecovery => "CredentialRecovery",
        }
    }

    /// Parse a `TEXT` value back into the enum.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidState`] if the string is not one of
    /// the five valid values.
    pub fn parse(s: &str) -> Result<Self, StorageError> {
        match s {
            "UserCentricEnrollment" => Ok(Self::UserCentricEnrollment),
            "UserCentricReEnrollment" => Ok(Self::UserCentricReEnrollment),
            "ReAuthentication" => Ok(Self::ReAuthentication),
            "Sync" => Ok(Self::Sync),
            "CredentialRecovery" => Ok(Self::CredentialRecovery),
            other => Err(StorageError::InvalidState(format!(
                "unknown CreationSource: {other}"
            ))),
        }
    }
}

/// A row of `pcp_records`, hydrated as a struct.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct PcpRecord {
    pub signup_id: SignupId,
    pub tier: Tier,
    pub version: String,
    pub signup_reason: Option<String>,
    pub status: PackageStatus,
    pub is_download_acknowledged: bool,
    pub creation_source: CreationSource,
    pub package_blob_cid: [u8; 32],
    pub orb_created_at: u64,
    pub created_at: u64,
    pub updated_at: u64,
}
