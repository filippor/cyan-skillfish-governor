use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error(transparent)]
    Smu(#[from] cyan_skillfish_governor_smu::SmuError),

    #[error(transparent)]
    TryRecv(#[from] std::sync::mpsc::TryRecvError),

    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, AppError>;

impl From<&str> for AppError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

impl From<String> for AppError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}
