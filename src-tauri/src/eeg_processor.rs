use crate::data_types::*;
use crate::error::AppError;
use crate::recorder::EdfRecorder;
use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::{AppHandle, Emitter};
use rustfft::{FftPlanner, num_complex::Complex};
use std::collections::VecDeque;
use crossbeam_channel;
use std::time::Duration;

// ✅ 简化的常量定义
const FFT_WINDOW_SIZE: usize = 256;        // 固定256点FFT
const OUTPUT_FREQ_BINS: usize = 50;        // 固定输出1-50Hz（50个bin）
const FRAME_INTERVAL_MS: u64 = 33;

pub struct EegProcessor {
    stream_info: StreamInfo,
    app_handle: AppHandle,
    
    // 数据源：来自LslManager的数据通道
    data_rx: Option<crossbeam_channel::Receiver<EegSample>>,
    
    // 录制器
    recorder: Arc<Mutex<Option<EdfRecorder>>>,
    
    // 运行状态
    is_running: Arc<tokio::sync::RwLock<bool>>,
    
    // 线程句柄管理
    thread_handles: Vec<tokio::task::JoinHandle<()>>,
}

impl EegProcessor {
    pub fn new(stream_info: StreamInfo, app_handle: AppHandle) -> Result<Self, AppError> {
        let processor = Self {
            stream_info,
            app_handle,
            data_rx: None,
            recorder: Arc::new(Mutex::new(None)),
            is_running: Arc::new(tokio::sync::RwLock::new(false)),
            thread_handles: Vec::new(),
        };
        
        Ok(processor)
    }
    
    /// 设置数据源（由LslManager提供）
    pub fn set_data_source(&mut self, data_rx: crossbeam_channel::Receiver<EegSample>) {
        self.data_rx = Some(data_rx);
    }
    
    /// 启动EEG处理
    pub async fn start(&mut self) -> Result<(), AppError> {
        let mut is_running = self.is_running.write().await;
        if *is_running {
            return Err(AppError::Config("Processor already running".to_string()));
        }
        
        let data_rx = self.data_rx.as_ref()
            .ok_or(AppError::NotConnected)?
            .clone();
        
        *is_running = true;
        drop(is_running); // 早释放锁
        
        // 启动全crossbeam处理管道
        self.start_crossbeam_pipeline(data_rx).await?;
        
        Ok(())
    }
    
    /// ✅ 消费式停止 - 消费 self，返回统计信息
    pub async fn stop(mut self) -> Result<EegProcessorStats, AppError> {
        println!("🛑 Stopping EEG Processor");
        
        let mut is_running = self.is_running.write().await;
        *is_running = false;
        drop(is_running);
        
        // 等待所有线程结束
        while let Some(handle) = self.thread_handles.pop() {
            if let Err(e) = handle.await {
                println!("Thread join error: {:?}", e);
            }
        }
        
        // 停止录制并获取统计信息
        let recording_stats = {
            let mut recorder_guard = self.recorder.lock().await;
            if let Some(recorder) = recorder_guard.take() {
                Some(recorder.close()?)
            } else {
                None
            }
        };
        
        // 生成处理器统计信息
        let stats = EegProcessorStats {
            stream_info: self.stream_info,
            recording_stats,
            threads_spawned: 4, // FFT, Recording, TimeDomain, Frontend
            // TODO: 添加更多统计信息
        };
        
        println!("📊 EEG Processor stopped: {:?}", stats);
        Ok(stats)
    }
    
    pub async fn start_recording(&self, filename: &str) -> Result<(), AppError> {
        let mut recorder_guard = self.recorder.lock().await;
        
        // 如果已在录制，先停止
        if recorder_guard.is_some() {
            drop(recorder_guard);
            self.stop_recording().await?;
            recorder_guard = self.recorder.lock().await;
        }
        
        // 创建新的录制器
        let new_recorder = EdfRecorder::new(
            filename.to_string(),
            self.stream_info.clone(),
            None, // patient_id
            None, // recording_info
        )?;
        
        *recorder_guard = Some(new_recorder);
        
        println!("Recording started: {}", filename);
        
        Ok(())
    }
    
    pub async fn stop_recording(&self) -> Result<(), AppError> {
        let mut recorder_guard = self.recorder.lock().await;
        
        if let Some(recorder) = recorder_guard.take() {
            // 关闭录制器并获取统计信息
            let stats = recorder.close()?;
            println!("Recording stopped: {:?}", stats);
        }
        
        Ok(())
    }
    
    /// 全crossbeam处理管道 - 为科研数据优化
    async fn start_crossbeam_pipeline(
        &mut self,
        data_rx: crossbeam_channel::Receiver<EegSample>,
    ) -> Result<(), AppError> {
        let stream_info = self.stream_info.clone();
        let app_handle = self.app_handle.clone();
        let recorder = self.recorder.clone();
        let is_running = self.is_running.clone();
        
        // ✅ 更新通道配置
        let (freq_tx, freq_rx) = crossbeam_channel::unbounded();
        let (time_domain_tx, time_domain_rx) = crossbeam_channel::unbounded();
        let (fft_trigger_tx, fft_trigger_rx) = crossbeam_channel::unbounded(); // ✅ 新增FFT触发通道
        
        // 录制线程（保持不变）
        let recording_handle = self.spawn_recording_thread(
            data_rx.clone(),
            recorder,
            is_running.clone()
        ).await;
        self.thread_handles.push(recording_handle);
        
        // ✅ 时域收集器（带FFT触发）
        let time_domain_handle = self.spawn_time_domain_collector(
            data_rx,
            time_domain_tx,
            fft_trigger_tx, // ✅ 传递FFT触发器
            stream_info.clone(),
            is_running.clone()
        ).await;
        self.thread_handles.push(time_domain_handle);
        
        // ✅ FFT线程（由批次触发）
        let fft_handle = self.spawn_fft_thread(
            fft_trigger_rx, // ✅ 从触发器接收
            freq_tx,
            stream_info.clone(),
            is_running.clone()
        ).await;
        self.thread_handles.push(fft_handle);
        
        // 前端线程（保持不变）
        let frontend_handle = self.spawn_frontend_thread(
            freq_rx,
            time_domain_rx,
            app_handle,
            stream_info.channels_count,
            stream_info.sample_rate,
            is_running.clone()
        ).await;
        self.thread_handles.push(frontend_handle);
        
        Ok(())
    }
    
    /// 录制线程 - 最高优先级，确保数据完整性
    async fn spawn_recording_thread(
        &self,
        data_rx: crossbeam_channel::Receiver<EegSample>,
        recorder: Arc<Mutex<Option<EdfRecorder>>>,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🔴 Recording thread started (HIGH PRIORITY)");
            
            let mut samples_processed = 0u64;
            let mut last_report = std::time::Instant::now();
            
            loop {
                // 检查运行状态
                {
                    let running = is_running.read().await;
                    if !*running {
                        println!("🔴 Recording thread stopping - processed {} samples", samples_processed);
                        break;
                    }
                }
                
                // 阻塞接收确保不丢失数据
                match data_rx.recv() {
                    Ok(sample) => {
                        let mut recorder_guard = recorder.lock().await;
                        
                        if let Some(recorder) = recorder_guard.as_mut() {
                            if let Err(e) = recorder.write_sample(&sample) {
                                println!("❌ CRITICAL: Recording error: {}", e);
                                // 对于科研数据，可能需要更严格的错误处理
                            } else {
                                samples_processed += 1;
                                
                                // 每秒报告一次处理状态
                                if last_report.elapsed() > Duration::from_secs(1) {
                                    println!("📊 Recording: {} samples/sec", samples_processed);
                                    last_report = std::time::Instant::now();
                                }
                            }
                        }
                    }
                    Err(_) => {
                        println!("🔴 Recording: data source disconnected");
                        break;
                    }
                }
            }
            
            println!("🔴 Recording thread stopped - total: {} samples", samples_processed);
        })
    }
    
    
    /// 重构：时域收集器 + FFT触发器
    async fn spawn_time_domain_collector(
        &self,
        data_rx: crossbeam_channel::Receiver<EegSample>,
        time_domain_tx: crossbeam_channel::Sender<EegBatch>,
        fft_trigger_tx: crossbeam_channel::Sender<(u64, Vec<EegSample>)>, // ✅ 传递(batch_id, samples)
        stream_info: StreamInfo,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🟢 Time domain collector started (with FFT sync)");
            
            let send_interval = Duration::from_millis(FRAME_INTERVAL_MS); // 33ms
            let mut current_batch = Vec::new();
            let mut batch_id = 0u64;
            let mut batch_timer = tokio::time::interval(send_interval);
            
            batch_timer.tick().await;
            
            loop {
                tokio::select! {
                    _ = batch_timer.tick() => {
                        {
                            let running = is_running.read().await;
                            if !*running {
                                if !current_batch.is_empty() {
                                    let final_batch = EegBatch {
                                        samples: current_batch.clone(),
                                        batch_id,
                                        channels_count: stream_info.channels_count,
                                        sample_rate: stream_info.sample_rate,
                                    };
                                    let _ = time_domain_tx.send(final_batch);
                                    
                                    // ✅ 最后一次FFT触发
                                    let _ = fft_trigger_tx.send((batch_id, current_batch));
                                }
                                println!("🟢 Time domain collector stopping");
                                break;
                            }
                        }
                        
                        // ✅ 发送时域批次
                        let batch = EegBatch {
                            samples: current_batch.clone(),
                            batch_id,
                            channels_count: stream_info.channels_count,
                            sample_rate: stream_info.sample_rate,
                        };
                        
                        if time_domain_tx.send(batch).is_err() {
                            println!("🟢 Time domain: receiver dropped");
                            break;
                        }
                        
                        // ✅ 同步触发FFT计算（传递批次ID）
                        if !current_batch.is_empty() {
                            if let Err(_) = fft_trigger_tx.send((batch_id, current_batch.clone())) {
                                println!("🟢 Time domain: FFT trigger dropped");
                            }
                        }
                        
                        if batch_id % 30 == 0 && batch_id > 0 {
                            println!("🟢 Batch #{}: {} samples → FFT trigger", 
                                     batch_id, current_batch.len());
                        }
                        
                        current_batch.clear();
                        batch_id += 1;
                    }
                    
                    _ = tokio::time::sleep(Duration::from_micros(100)) => {
                        while let Ok(sample) = data_rx.try_recv() {
                            current_batch.push(sample);
                        }
                    }
                }
            }
            
            println!("🟢 Time domain collector stopped");
        })
    }
    
    /// 重构：基于批次触发的FFT线程
    async fn spawn_fft_thread(
        &self,
        fft_trigger_rx: crossbeam_channel::Receiver<(u64, Vec<EegSample>)>, // ✅ 接收(batch_id, samples)
        freq_tx: crossbeam_channel::Sender<(u64, Vec<FreqData>)>, // ✅ 发送(batch_id, freq_data)
        stream_info: StreamInfo,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🟡 FFT thread started (batch-triggered with ID tracking)");
            
            let mut fft_planner = FftPlanner::new();
            let fft = fft_planner.plan_fft_forward(FFT_WINDOW_SIZE);
            
            let mut channel_windows: Vec<VecDeque<f64>> = (0..stream_info.channels_count)
                .map(|_| VecDeque::with_capacity(FFT_WINDOW_SIZE + 100))
                .collect();
            
            let mut batches_processed = 0u64;
            let mut ffts_computed = 0u64;
            
            loop {
                tokio::select! {
                    batch_result = tokio::task::spawn_blocking({
                        let fft_trigger_rx = fft_trigger_rx.clone();
                        move || fft_trigger_rx.recv()
                    }) => {
                        match batch_result {
                            Ok(Ok((batch_id, sample_batch))) => {  // ✅ 解包批次ID
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
                                
                                // ✅ 计算FFT并关联批次ID
                                if channel_windows[0].len() >= FFT_WINDOW_SIZE {
                                    let mut freq_data = compute_fixed_range_fft(
                                        &channel_windows,
                                        fft.as_ref(),
                                        stream_info.sample_rate,
                                    );
                                    
                                    // ✅ 为每个频域数据关联批次ID
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


    /// 前端发送线程 - 30Hz刷新，总是发送可用数据
    async fn spawn_frontend_thread(
        &self,
        freq_rx: crossbeam_channel::Receiver<(u64, Vec<FreqData>)>, // ✅ 接收带批次ID的频域数据
        time_domain_rx: crossbeam_channel::Receiver<EegBatch>,
        app_handle: AppHandle,
        channels_count: u32,
        sample_rate: f64,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🔵 Frontend thread started (with batch matching)");
            
            let mut frame_timer = tokio::time::interval(
                Duration::from_millis(FRAME_INTERVAL_MS)
            );
            
            // ✅ 缓冲区：存储等待匹配的数据
            let mut freq_buffer: std::collections::HashMap<u64, Vec<FreqData>> = std::collections::HashMap::new();
            let mut time_buffer: std::collections::HashMap<u64, EegBatch> = std::collections::HashMap::new();
            
            let mut frame_count = 0u64;
            let mut next_expected_batch_id = 0u64;
            
            // 创建空数据函数
            let create_empty_freq_data = || -> Vec<FreqData> {
                (0..channels_count).map(|i| FreqData {
                    channel_index: i,
                    spectrum: vec![0.0; OUTPUT_FREQ_BINS],
                    frequency_bins: (1..=50).map(|f| f as f64).collect(),
                    batch_id: None,
                }).collect()
            };
            
            loop {
                tokio::select! {
                    // 定时发送frame-update事件
                    _ = frame_timer.tick() => {
                        // 检查停止状态
                        {
                            let running = is_running.read().await;
                            if !*running {
                                println!("🔵 Frontend thread stopping");
                                break;
                            }
                        }
                        
                        // ✅ 收集所有可用数据到缓冲区
                        while let Ok((batch_id, freq_data)) = freq_rx.try_recv() {
                            freq_buffer.insert(batch_id, freq_data);
                        }
                        
                        while let Ok(time_domain) = time_domain_rx.try_recv() {
                            time_buffer.insert(time_domain.batch_id, time_domain);
                        }
                        
                        // ✅ 尝试发送匹配的数据对
                        let mut sent_data = false;
                        
                        // 检查是否有完整的数据对可以发送
                        if let (Some(time_domain), freq_data) = (
                            time_buffer.remove(&next_expected_batch_id),
                            freq_buffer.remove(&next_expected_batch_id)
                        ) {
                            // ✅ 有匹配的数据对
                            let freq_data = freq_data.unwrap_or_else(|| create_empty_freq_data());
                            
                            let payload = FramePayload {
                                time_domain,
                                frequency_domain: freq_data,
                            };
                            
                            if let Err(e) = app_handle.emit("frame-update", &payload) {
                                println!("Failed to emit frame-update: {}", e);
                            } else {
                                frame_count += 1;
                                sent_data = true;
                                
                                if frame_count <= 5 {
                                    println!("🔵 Frame #{} sent - matched batch #{}", 
                                             frame_count, next_expected_batch_id);
                                }
                            }
                            
                            next_expected_batch_id += 1;
                        } else if let Some(time_domain) = time_buffer.remove(&next_expected_batch_id) {
                            // ✅ 只有时域数据，FFT还在计算中
                            let freq_data = create_empty_freq_data();
                            
                            let payload = FramePayload {
                                time_domain,
                                frequency_domain: freq_data,
                            };
                            
                            if let Err(e) = app_handle.emit("frame-update", &payload) {
                                println!("Failed to emit frame-update: {}", e);
                            } else {
                                frame_count += 1;
                                sent_data = true;
                                
                                if frame_count <= 10 {
                                    println!("🔵 Frame #{} sent - batch #{} (time only, FFT pending)", 
                                             frame_count, next_expected_batch_id);
                                }
                            }
                            
                            next_expected_batch_id += 1;
                        }
                        
                        // ✅ 如果没有匹配数据，发送空帧保持节拍
                        if !sent_data {
                            let empty_time = EegBatch {
                                samples: vec![],
                                batch_id: frame_count,
                                channels_count,
                                sample_rate,
                            };
                            
                            let payload = FramePayload {
                                time_domain: empty_time,
                                frequency_domain: create_empty_freq_data(),
                            };
                            
                            if let Err(e) = app_handle.emit("frame-update", &payload) {
                                println!("Failed to emit frame-update: {}", e);
                            } else {
                                frame_count += 1;
                            }
                        }
                        
                        // ✅ 清理过旧的缓冲区数据（防止内存泄漏）
                        let cleanup_threshold = next_expected_batch_id.saturating_sub(10);
                        freq_buffer.retain(|&batch_id, _| batch_id >= cleanup_threshold);
                        time_buffer.retain(|&batch_id, _| batch_id >= cleanup_threshold);
                        
                        // 定期报告缓冲区状态
                        if frame_count % 300 == 0 && frame_count > 0 {
                            println!("🔵 Buffer status: freq={}, time={}, next_expected={}", 
                                     freq_buffer.len(), time_buffer.len(), next_expected_batch_id);
                        }
                    }
                }
            }
            
            println!("🔵 Frontend thread stopped - frames sent: {}", frame_count);
        })
    }
}

// FFT计算辅助函数保持不变
// ✅ 改进的FFT计算函数
fn compute_fixed_range_fft(
    channel_windows: &[VecDeque<f64>],
    fft: &dyn rustfft::Fft<f64>,
    sample_rate: f64,
) -> Vec<FreqData> {
    let mut results = Vec::new();
    
    // ✅ 预计算频率bin映射
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
        
        // ✅ 直接构建1-50Hz的输出
        let mut spectrum = Vec::with_capacity(OUTPUT_FREQ_BINS);
        let mut frequency_bins = Vec::with_capacity(OUTPUT_FREQ_BINS);
        
        for target_freq in 1..=50 {  // 1Hz到50Hz
            let target_freq_f64 = target_freq as f64;
            
            // 找到最接近的FFT bin
            let fft_bin_index = (target_freq_f64 / freq_resolution).round() as usize;
            
            // 获取幅度（如果bin存在）
            let magnitude = if fft_bin_index < fft_input.len() / 2 {
                fft_input[fft_bin_index].norm() / FFT_WINDOW_SIZE as f64
            } else {
                0.0  // 超出Nyquist频率，设为0
            };
            
            spectrum.push(magnitude);
            frequency_bins.push(target_freq_f64);
        }
        
        results.push(FreqData {
            channel_index: ch_idx as u32,
            spectrum,
            frequency_bins,
            batch_id: None,  // ✅ 默认无批次关联
        });
    }
    
    results
}

// ✅ 新增：Hanning窗函数
fn apply_hanning_window(data: &mut [Complex<f64>]) {
    let n = data.len();
    for (i, sample) in data.iter_mut().enumerate() {
        let window_val = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos());
        sample.re *= window_val;
        sample.im *= window_val;
    }
}

/// 新增：EEG处理器统计信息
#[derive(Debug, Clone)]
pub struct EegProcessorStats {
    pub stream_info: StreamInfo,
    pub recording_stats: Option<crate::recorder::RecordingStats>,
    pub threads_spawned: u32,
}