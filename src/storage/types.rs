//! Public types stored in `pcp_records`.

use std::fmt;

use crate::storage::error::StorageError;

/// Server-assigned signup identifier. Sensitive; do not log.
pub type SignupId = String;

/// Shard index within a signup. `0` for untiered PCPs.
pub type Tier = u8;

/// Lifecycle state of one PCP row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Stable string used as the SQL `TEXT` value.
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

    /// Parse a value previously produced by [`Self::as_str`].
    ///
    /// # Errors
    ///
    /// [`StorageError::InvalidState`] for any other input.
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

    /// `true` iff moving from `self` to `next` is allowed by the state
    /// machine. Used by [`crate::storage::OrbPcpStore::update_status`].
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

impl fmt::Display for PackageStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a PCP row came to exist on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Parse a value previously produced by [`Self::as_str`].
    ///
    /// # Errors
    ///
    /// [`StorageError::InvalidState`] for any other input.
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

/// A row of `pcp_records`. Timestamps are Unix seconds.
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
