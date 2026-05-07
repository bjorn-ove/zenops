#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unknown OS: {0}")]
    UnknownOs(&'static str),
    #[error("Failed to check if {0:?} exists: {1}")]
    ExistsFailed(String, std::io::Error),
    #[error(transparent)]
    Which(#[from] crate::utils::which::Error),
    #[error(transparent)]
    Condition(#[from] crate::config::condition::Error),
}
