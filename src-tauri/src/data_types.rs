use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LslStreamInfo {
    pub name: String,
    pub stream_type: String,
    pub channels_count: u32,
    pub sample_rate: f64,
    pub source_id: String,
    pub hostname: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StreamInfo {
    pub name: String,
    pub stream_type: String,
    pub channels_count: u32,
    pub sample_rate: f64,
    pub is_connected: bool,
    pub source_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EegSample {
    pub timestamp: f64,
    pub channels: Vec<f64>,
    pub sample_id: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EegBatch {
    pub samples: Vec<EegSample>,
    pub batch_id: u64,
    pub channels_count: u32,
    pub sample_rate: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FreqData {
    pub channel_index: u32,
    pub spectrum: Vec<f64>,
    pub frequency_bins: Vec<f64>,
    pub batch_id: Option<u64>,  // ✅ 添加批次ID关联
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FramePayload {
    pub time_domain: EegBatch,
    pub frequency_domain: Vec<FreqData>,
}

// 内部控制命令
#[derive(Debug, Clone)]
pub enum ControlCommand {
    Connect(String),
    Disconnect,
    Stop,
    StartRecording(String),
    StopRecording,
}

#[derive(Debug, Clone)]
pub enum StatusUpdate {
    Connected(StreamInfo),
    Disconnected,
    RecordingStarted(String),
    RecordingStopped,
    Error(String),
}

// 新增录制统计信息的序列化支持
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecordingStats {
    pub filename: String,
    pub duration_seconds: f64,
    pub samples_written: u64,
    pub channels_count: u32,
    pub sample_rate: f64,
    pub start_time: String,  // 序列化为字符串
    pub file_size_bytes: u64,
}

// 在 data_types.rs 中添加

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConnectionStatus {
    pub is_lsl_connected: bool,
    pub is_processor_running: bool,
    pub current_stream: Option<StreamInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemHealth {
    pub lsl_manager_status: String,
    pub processor_status: String,
    pub memory_usage_mb: u64,
    pub uptime_seconds: u64,
}
