use crate::data_types::*;
use rustfft::{FftPlanner, num_complex::Complex};
use std::collections::VecDeque;
use crossbeam_channel;
use std::sync::Arc;
use std::time::Duration;

// FFT相关常量
const FFT_WINDOW_SIZE: usize = 256;
const OUTPUT_FREQ_BINS: usize = 50;

/// FFT处理器 - 专门负责频域分析
pub struct FftProcessor {
    stream_info: StreamInfo,
    is_running: Arc<tokio::sync::RwLock<bool>>,
}

impl FftProcessor {
    pub fn new(
        stream_info: StreamInfo,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> Self {
        Self {
            stream_info,
            is_running,
        }
    }
    
    /// 启动FFT处理线程
    pub async fn spawn_fft_thread(
        &self,
        fft_trigger_rx: crossbeam_channel::Receiver<(u64, Vec<EegSample>)>,
        freq_tx: crossbeam_channel::Sender<(u64, Vec<FreqData>)>,
    ) -> tokio::task::JoinHandle<()> {
        let stream_info = self.stream_info.clone();
        let is_running = self.is_running.clone();
        
        tokio::spawn(async move {
            println!("🟡 FFT thread started (batch-triggered, 1-50Hz)");
            
            let mut fft_planner = FftPlanner::new();
            let fft = fft_planner.plan_fft_forward(FFT_WINDOW_SIZE);
            
            // 为每个通道维护滑动窗口
            let mut channel_windows: Vec<VecDeque<f64>> = (0..stream_info.channels_count)
                .map(|_| VecDeque::with_capacity(FFT_WINDOW_SIZE + 100))
                .collect();
            
            let mut batches_processed = 0u64;
            let mut ffts_computed = 0u64;
            
            let freq_resolution = stream_info.sample_rate / FFT_WINDOW_SIZE as f64;
            println!("🟡 FFT config: size={}, resolution={:.2}Hz/bin, target=1-50Hz", 
                     FFT_WINDOW_SIZE, freq_resolution);
            
            loop {
                tokio::select! {
                    batch_result = tokio::task::spawn_blocking({
                        let fft_trigger_rx = fft_trigger_rx.clone();
                        move || fft_trigger_rx.recv()
                    }) => {
                        match batch_result {
                            Ok(Ok((batch_id, sample_batch))) => {
                                batches_processed += 1;
                                
                                // 更新滑动窗口
                                for sample in sample_batch {
                                    for (ch_idx, &value) in sample.channels.iter().enumerate() {
                                        if ch_idx < channel_windows.len() {
                                            let window = &mut channel_windows[ch_idx];
                                            window.push_back(value);
                                            
                                            if window.len() > FFT_WINDOW_SIZE {
                                                window.pop_front();
                                            }
                                        }
                                    }
                                }
                                
                                // 计算FFT并关联批次ID
                                if channel_windows[0].len() >= FFT_WINDOW_SIZE {
                                    let mut freq_data = compute_fixed_range_fft(
                                        &channel_windows,
                                        fft.as_ref(),
                                        stream_info.sample_rate,
                                    );
                                    
                                    // 为每个频域数据关联批次ID
                                    for freq_item in &mut freq_data {
                                        freq_item.batch_id = Some(batch_id);
                                    }
                                    
                                    if freq_tx.send((batch_id, freq_data)).is_err() {
                                        println!("🟡 FFT: frequency receiver dropped");
                                        break;
                                    }
                                    
                                    ffts_computed += 1;
                                    
                                    if ffts_computed <= 5 {
                                        println!("🟡 FFT #{} for batch #{} → {} channels, 1-50Hz", 
                                                 ffts_computed, batch_id, stream_info.channels_count);
                                    } else if ffts_computed % 60 == 0 {
                                        println!("🟡 FFT progress: {} computations completed", ffts_computed);
                                    }
                                }
                            }
                            Ok(Err(_)) => {
                                println!("🟡 FFT: trigger channel disconnected");
                                break;
                            }
                            Err(e) => {
                                println!("🟡 FFT: batch processing error: {:?}", e);
                            }
                        }
                    }
                    
                    // 定期检查停止状态
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        let running = is_running.read().await;
                        if !*running {
                            println!("🟡 FFT thread stopping");
                            break;
                        }
                    }
                }
            }
            
            println!("🟡 FFT thread stopped - batches: {}, FFTs: {}", batches_processed, ffts_computed);
        })
    }
}

/// 计算固定1-50Hz范围的FFT
fn compute_fixed_range_fft(
    channel_windows: &[VecDeque<f64>],
    fft: &dyn rustfft::Fft<f64>,
    sample_rate: f64,
) -> Vec<FreqData> {
    let mut results = Vec::new();
    let freq_resolution = sample_rate / FFT_WINDOW_SIZE as f64;
    
    for (ch_idx, window) in channel_windows.iter().enumerate() {
        if window.len() < FFT_WINDOW_SIZE {
            continue;
        }
        
        // 准备FFT输入数据
        let mut fft_input: Vec<Complex<f64>> = window
            .iter()
            .take(FFT_WINDOW_SIZE)
            .map(|&x| Complex::new(x, 0.0))
            .collect();
        
        // 应用Hanning窗函数
        apply_hanning_window(&mut fft_input);
        
        // 执行FFT
        fft.process(&mut fft_input);
        
        // 构建1-50Hz的输出
        let mut spectrum = Vec::with_capacity(OUTPUT_FREQ_BINS);
        let mut frequency_bins = Vec::with_capacity(OUTPUT_FREQ_BINS);
        
        for target_freq in 1..=50 {
            let target_freq_f64 = target_freq as f64;
            let fft_bin_index = (target_freq_f64 / freq_resolution).round() as usize;
            
            let magnitude = if fft_bin_index < fft_input.len() / 2 {
                fft_input[fft_bin_index].norm() / FFT_WINDOW_SIZE as f64
            } else {
                0.0
            };
            
            spectrum.push(magnitude);
            frequency_bins.push(target_freq_f64);
        }
        
        results.push(FreqData {
            channel_index: ch_idx as u32,
            spectrum,
            frequency_bins,
            batch_id: None,
        });
    }
    
    results
}

/// 应用Hanning窗函数
fn apply_hanning_window(data: &mut [Complex<f64>]) {
    let n = data.len();
    for (i, sample) in data.iter_mut().enumerate() {
        let window_val = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos());
        sample.re *= window_val;
        sample.im *= window_val;
    }
}

/// FFT相关的公共常量和函数
pub mod constants {
    pub const OUTPUT_FREQ_BINS: usize = 50;
    pub const TARGET_FREQ_MIN: u32 = 1;
    pub const TARGET_FREQ_MAX: u32 = 50;
}

/// FFT配置和优化相关的实用函数
pub mod utils {
    use super::constants::*;
    
    /// 创建空的频域数据
    pub fn create_empty_freq_data(channels_count: u32) -> Vec<crate::data_types::FreqData> {
        (0..channels_count).map(|i| crate::data_types::FreqData {
            channel_index: i,
            spectrum: vec![0.0; OUTPUT_FREQ_BINS],
            frequency_bins: (TARGET_FREQ_MIN..=TARGET_FREQ_MAX).map(|f| f as f64).collect(),
            batch_id: None,
        }).collect()
    }
}