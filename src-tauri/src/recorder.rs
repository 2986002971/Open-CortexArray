use crate::data_types::*;
use crate::error::AppError;
use edfplus::{EdfWriter, SignalParam};
use std::collections::VecDeque;
use chrono::{DateTime, Utc};

pub struct EdfRecorder {
    writer: EdfWriter,
    filename: String,
    stream_info: StreamInfo,
    samples_written: u64,
    
    // 数据缓冲区 - 每个通道一个队列
    channel_buffers: Vec<VecDeque<f64>>,
    
    // EDF+配置参数
    samples_per_record: usize,    // 每个数据记录的样本数
    record_duration_sec: f64,     // 每个记录的时长（秒）
    
    // 录制元数据
    start_time: DateTime<Utc>,
    patient_id: String,
    recording_info: String,
}

impl EdfRecorder {
    pub fn new(
        filename: String, 
        stream_info: StreamInfo,
        patient_id: Option<String>,
        recording_info: Option<String>
    ) -> Result<Self, AppError> {
        
        // 计算EDF+参数
        let record_duration_sec = 1.0; // 1秒每个数据记录
        let samples_per_record = (stream_info.sample_rate * record_duration_sec) as usize;
        
        let mut writer = EdfWriter::create(&filename)
            .map_err(|e| AppError::Recording(format!("Failed to create EDF file: {}", e)))?;
        
        // 设置文件头信息
        let start_time = Utc::now();
        let patient_id = patient_id.unwrap_or_else(|| "Unknown".to_string());
        let recording_info = recording_info.unwrap_or_else(|| 
            format!("EEG Recording from {} at {}", stream_info.source_id, start_time.format("%Y-%m-%d %H:%M:%S"))
        );
        
        // 为每个EEG通道添加信号参数
        for ch_idx in 0..stream_info.channels_count {
            let signal_param = SignalParam {
                label: format!("EEG Ch{:02}", ch_idx + 1),
                samples_in_file: 0,
                physical_max: 100.0,     // μV 物理最大值
                physical_min: -100.0,    // μV 物理最小值
                digital_max: 32767,      // 16位ADC最大值
                digital_min: -32768,     // 16位ADC最小值
                samples_per_record: samples_per_record as i32,  // ✅ 修复：转换为i32
                physical_dimension: "uV".to_string(),
                prefilter: "HP:0.1Hz LP:70Hz".to_string(),
                transducer: "AgAgCl electrodes".to_string(),
            };
            
            writer.add_signal(signal_param)
                .map_err(|e| AppError::Recording(format!("Failed to add signal {}: {}", ch_idx, e)))?;
        }
        
        // 初始化通道缓冲区
        let channel_buffers = (0..stream_info.channels_count)
            .map(|_| VecDeque::with_capacity(samples_per_record * 2))
            .collect();
        
        Ok(Self {
            writer,
            filename: filename.clone(),
            stream_info,
            samples_written: 0,
            channel_buffers,
            samples_per_record,
            record_duration_sec,
            start_time,
            patient_id,
            recording_info,
        })
    }
    
    pub fn write_sample(&mut self, sample: &EegSample) -> Result<(), AppError> {
        // 将样本数据加入各通道缓冲区
        for (ch_idx, &value) in sample.channels.iter().enumerate() {
            if ch_idx < self.channel_buffers.len() {
                self.channel_buffers[ch_idx].push_back(value);
            }
        }
        
        self.samples_written += 1;
        
        // 检查是否需要写入一个完整的数据记录
        if self.channel_buffers[0].len() >= self.samples_per_record {
            self.write_data_record()?;
        }
        
        Ok(())
    }
    
    fn write_data_record(&mut self) -> Result<(), AppError> {
        // 为每个通道收集samples_per_record个样本
        let mut record_data: Vec<Vec<f64>> = Vec::new();
        
        for channel_buffer in &mut self.channel_buffers {
            let mut channel_samples = Vec::with_capacity(self.samples_per_record);
            
            // 从缓冲区取出样本
            for _ in 0..self.samples_per_record {
                if let Some(sample) = channel_buffer.pop_front() {
                    channel_samples.push(sample);
                } else {
                    // 如果缓冲区不够，用0填充（这种情况应该很少发生）
                    channel_samples.push(0.0);
                }
            }
            
            record_data.push(channel_samples);
        }
        
        // 写入EDF+数据记录
        self.writer.write_samples(&record_data)
            .map_err(|e| AppError::Recording(format!("Failed to write data record: {}", e)))?;
        
        println!("EDF+ data record written: {} samples per channel", self.samples_per_record);
        
        Ok(())
    }
    
    pub fn write_annotation(&mut self, timestamp: f64, description: &str) -> Result<(), AppError> {
        // TODO: 如果edfplus库支持注释写入，在这里实现
        // 目前只是打印日志
        println!("Annotation at {:.3}s: {}", timestamp, description);
        Ok(())
    }
    
    pub fn get_recording_stats(&self) -> RecordingStats {
        let duration_sec = self.samples_written as f64 / self.stream_info.sample_rate;
        
        RecordingStats {
            filename: self.filename.clone(),
            duration_seconds: duration_sec,
            samples_written: self.samples_written,
            channels_count: self.stream_info.channels_count,
            sample_rate: self.stream_info.sample_rate,
            start_time: self.start_time,
            file_size_bytes: 0, // TODO: 获取实际文件大小
        }
    }
    
    pub fn close(mut self) -> Result<RecordingStats, AppError> {
        // ✅ 修复：在finalize之前先收集统计信息
        let stats = RecordingStats {
            filename: self.filename.clone(),
            duration_seconds: self.samples_written as f64 / self.stream_info.sample_rate,
            samples_written: self.samples_written,
            channels_count: self.stream_info.channels_count,
            sample_rate: self.stream_info.sample_rate,
            start_time: self.start_time,
            file_size_bytes: 0, // TODO: 获取实际文件大小
        };
        
        // 写入剩余的缓冲数据
        if !self.channel_buffers.is_empty() && self.channel_buffers[0].len() > 0 {
            println!("Writing remaining {} samples before closing", self.channel_buffers[0].len());
            
            // 如果剩余样本不足一个完整记录，用0填充
            let remaining_samples = self.channel_buffers[0].len();
            if remaining_samples < self.samples_per_record {
                for channel_buffer in &mut self.channel_buffers {
                    let padding_needed = self.samples_per_record - remaining_samples;
                    for _ in 0..padding_needed {
                        channel_buffer.push_back(0.0);
                    }
                }
            }
            
            // 写入最后一个数据记录
            self.write_data_record()?;
        }
        
        // 完成EDF+文件写入 - 这会消费self.writer
        self.writer.finalize()
            .map_err(|e| AppError::Recording(format!("Failed to finalize EDF file: {}", e)))?;
        
        println!("Recording completed successfully:");
        println!("  File: {}", stats.filename);
        println!("  Duration: {:.2} seconds", stats.duration_seconds);
        println!("  Samples: {} per channel", stats.samples_written);
        println!("  Channels: {}", stats.channels_count);
        
        Ok(stats)
    }
}

// 录制统计信息
#[derive(Debug, Clone)]
pub struct RecordingStats {
    pub filename: String,
    pub duration_seconds: f64,
    pub samples_written: u64,
    pub channels_count: u32,
    pub sample_rate: f64,
    pub start_time: DateTime<Utc>,
    pub file_size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_edf_recorder_creation() {
        let stream_info = StreamInfo {
            name: "Test EEG".to_string(),
            stream_type: "EEG".to_string(),
            channels_count: 8,
            sample_rate: 250.0,
            is_connected: true,
            source_id: "test_device".to_string(),
        };
        
        let recorder = EdfRecorder::new(
            "test_recording.edf".to_string(),
            stream_info,
            Some("Test Patient".to_string()),
            Some("Test Recording".to_string())
        );
        
        assert!(recorder.is_ok());
    }
}