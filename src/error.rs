use thiserror::Error;

#[derive(Error, Debug)]
pub enum SmuError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Transport not opened")]
    TransportNotOpened,
    
    #[error("Queue {0} not configured")]
    QueueNotConfigured(u8),
    
    #[error("Queue 0 access disabled; enable with allow_queue0=true")]
    Queue0Disabled,
    
    #[error("SMU returned status 0x{status:02X} for queue {queue} msg 0x{msg:02X}")]
    SmuStatus {
        status: u8,
        queue: u8,
        msg: u8,
    },
    
    #[error("Test message failed: expected {expected}, got {actual}")]
    TestMessageFailed {
        expected: u32,
        actual: u32,
    },
    
    #[error("SMU timeout waiting for response")]
    Timeout,
}

pub type Result<T> = std::result::Result<T, SmuError>;

// Add this implementation to allow SmuError to be converted to std::io::Error
impl From<SmuError> for std::io::Error {
    fn from(err: SmuError) -> Self {
        match err {
            SmuError::Io(e) => e,
            other => std::io::Error::new(std::io::ErrorKind::Other, other),
        }
    }
}
