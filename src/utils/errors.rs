use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("API Network Error: {0}")]
    Api(#[from] reqwest::Error),

    #[error("Data Parsing Error: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WebSocket Error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("System Fault: {0}")]
    System(String),
}

pub type AppResult<T> = Result<T, AppError>;
