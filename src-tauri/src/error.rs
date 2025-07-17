use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("LSL error: {0}")]
    Lsl(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Channel communication error: {0}")]
    Channel(String),
    
    #[error("FFT calculation error: {0}")]
    Fft(String),
    
    #[error("Recording error: {0}")]
    Recording(String),
    
    #[error("Stream not connected")]
    NotConnected,
    
    #[error("Invalid configuration: {0}")]
    Config(String),
}

impl From<tokio::sync::mpsc::error::SendError<crate::data_types::ControlCommand>> for AppError {
    fn from(err: tokio::sync::mpsc::error::SendError<crate::data_types::ControlCommand>) -> Self {
        AppError::Channel(err.to_string())
    }
}

// 添加对std::sync::mpsc的支持
impl<T> From<std::sync::mpsc::SendError<T>> for AppError {
    fn from(err: std::sync::mpsc::SendError<T>) -> Self {
        AppError::Channel(err.to_string())
    }
}

// 添加对crossbeam通道的支持
impl<T> From<crossbeam_channel::SendError<T>> for AppError {
    fn from(err: crossbeam_channel::SendError<T>) -> Self {
        AppError::Channel(err.to_string())
    }
}

// 添加对oneshot通道的支持
impl From<tokio::sync::oneshot::error::RecvError> for AppError {
    fn from(err: tokio::sync::oneshot::error::RecvError) -> Self {
        AppError::Channel(format!("OneShot receive error: {}", err))
    }
}