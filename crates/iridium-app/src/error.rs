use iridium_acquire::AcquireError;
use iridium_recovery::RecoveryError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("acquisition failed: {0}")]
    Acquire(#[from] AcquireError),

    #[error("recovery failed: {0}")]
    Recovery(#[from] RecoveryError),

    #[error("verify failed: {0}")]
    Verify(#[from] VerifyError),

    #[error("audit log error: {0}")]
    Audit(#[from] iridium_audit::AuditError),

    #[error("config error: {0}")]
    Config(String),
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("I/O error reading image: {0}")]
    Io(#[from] std::io::Error),

    #[error("EWF read error: {0}")]
    Ewf(#[from] iridium_ewf::EwfError),

    #[error("digest mismatch for {algorithm}: expected {expected}, got {actual}")]
    Mismatch {
        algorithm: String,
        expected: String,
        actual: String,
    },
}
