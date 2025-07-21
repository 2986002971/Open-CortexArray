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

// ✅ 简化的通道优先数据结构
#[derive(Clone, Debug)]
pub struct OptimizedEegBatch {
    pub batch_id: u64,
    pub timestamp: f64,
    pub channels_count: u32,
    pub samples_per_channel: u32,
    pub sample_rate: f64,
    
    // ✅ 纯数据，去除冗余元信息
    pub channel_data: Vec<ChannelSamples>,
}

#[derive(Clone, Debug)]
pub struct ChannelSamples {
    pub channel_index: u32,
    pub samples: Vec<f32>,             // 单通道连续数据，仅此而已
}

// ✅ 极简二进制帧构建器
pub struct BinaryFrameBuilder {
    buffer: Vec<u8>,
}

impl BinaryFrameBuilder {
    pub fn new() -> Self {
        Self { 
            buffer: Vec::with_capacity(65536),      // 64KB预分配
        }
    }
    
    /// ✅ 构建最简二进制帧
    /// 内存布局：
    /// [Header: 32 bytes] + [Channel Data Blocks]
    /// Header: batch_id(8) + timestamp(8) + channels_count(4) + samples_per_channel(4) + sample_rate(8)
    /// Channel Block: channel_index(4) + [samples: 4*N bytes]
    pub fn build_channel_major_frame(&mut self, batch: &OptimizedEegBatch) -> Vec<u8> {
        self.buffer.clear();
        
        // ✅ 写入帧头部 (32 bytes)
        self.buffer.extend(&batch.batch_id.to_le_bytes());           // 8 bytes
        self.buffer.extend(&batch.timestamp.to_le_bytes());          // 8 bytes  
        self.buffer.extend(&batch.channels_count.to_le_bytes());     // 4 bytes
        self.buffer.extend(&batch.samples_per_channel.to_le_bytes()); // 4 bytes
        self.buffer.extend(&batch.sample_rate.to_le_bytes());        // 8 bytes
        
        // ✅ 写入通道数据块（通道优先）
        for channel in &batch.channel_data {
            // 通道索引 (4 bytes)
            self.buffer.extend(&channel.channel_index.to_le_bytes());
            
            // ✅ SIMD优化的样本数据写入
            self.write_samples_simd(&channel.samples);
        }
        
        self.buffer.clone()
    }
    
    /// ✅ 使用SIMD加速的样本写入
    #[cfg(target_arch = "x86_64")]
    fn write_samples_simd(&mut self, samples: &[f32]) {
        use std::arch::x86_64::*;
        
        let required_size = samples.len() * 4;
        self.buffer.reserve(required_size);
        
        unsafe {
            let samples_ptr = samples.as_ptr();
            
            // 以16字节（4个f32）为单位处理
            let simd_chunks = samples.len() / 4;
            
            for i in 0..simd_chunks {
                // 加载4个f32到SIMD寄存器
                let chunk_ptr = samples_ptr.add(i * 4);
                let values = _mm_loadu_ps(chunk_ptr);
                
                // 直接转换为字节并存储
                let bytes = std::mem::transmute::<__m128, [u8; 16]>(values);
                self.buffer.extend_from_slice(&bytes);
            }
            
            // 处理剩余的样本
            let remainder_start = simd_chunks * 4;
            for i in remainder_start..samples.len() {
                self.buffer.extend(&samples[i].to_le_bytes());
            }
        }
    }
    
    /// ✅ 非SIMD版本
    #[cfg(not(target_arch = "x86_64"))]
    fn write_samples_simd(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.buffer.extend(&sample.to_le_bytes());
        }
    }
}

// ✅ 高性能数据转换器
pub struct DataConverter {
    channel_buffers: Vec<Vec<f32>>,
}

impl DataConverter {
    pub fn new(channels_count: usize) -> Self {
        Self {
            channel_buffers: (0..channels_count)
                .map(|_| Vec::with_capacity(128))    // 预分配每通道缓冲区
                .collect(),
        }
    }
    
    /// ✅ 调整通道数量时重新分配缓冲区
    pub fn resize_for_channels(&mut self, new_channels_count: usize) {
        if new_channels_count != self.channel_buffers.len() {
            self.channel_buffers.clear();
            self.channel_buffers = (0..new_channels_count)
                .map(|_| Vec::with_capacity(128))
                .collect();
        }
    }
    
    /// ✅ 将现有EegBatch转换为优化格式（用于前端发送）
    pub fn convert_eeg_batch_to_optimized(
        &mut self,
        eeg_batch: &EegBatch,
        batch_id: u64,
    ) -> OptimizedEegBatch {
        if eeg_batch.samples.is_empty() {
            return OptimizedEegBatch {
                batch_id,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap().as_secs_f64(),
                channels_count: eeg_batch.channels_count,
                samples_per_channel: 0,
                sample_rate: eeg_batch.sample_rate,
                channel_data: Vec::new(),
            };
        }
        
        let channels_count = eeg_batch.channels_count as usize;
        let samples_per_channel = eeg_batch.samples.len() as u32;
        
        // 确保缓冲区大小正确
        self.resize_for_channels(channels_count);
        
        // 清空并准备缓冲区
        for buffer in &mut self.channel_buffers {
            buffer.clear();
            buffer.reserve(samples_per_channel as usize);
        }
        
        // ✅ 通道优先收集（从EegBatch.samples转换）
        for sample in &eeg_batch.samples {
            for (ch, &value) in sample.channels.iter().enumerate() {
                if ch < self.channel_buffers.len() {
                    self.channel_buffers[ch].push(value as f32);
                }
            }
        }
        
        // ✅ 构建通道数据
        let mut channel_data = Vec::with_capacity(channels_count);
        for (ch_idx, samples) in self.channel_buffers.iter().enumerate() {
            if ch_idx >= channels_count { break; }
            
            channel_data.push(ChannelSamples {
                channel_index: ch_idx as u32,
                samples: samples.clone(),
            });
        }
        
        OptimizedEegBatch {
            batch_id,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap().as_secs_f64(),
            channels_count: eeg_batch.channels_count,
            samples_per_channel,
            sample_rate: eeg_batch.sample_rate,
            channel_data,
        }
    }
}

