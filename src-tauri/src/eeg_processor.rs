use crate::data_types::*;
use crate::error::AppError;
use crate::recorder::EdfRecorder;
use crate::fft_processor::{FftProcessor, utils as fft_utils}; // ✅ 导入FFT模块
use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::{AppHandle, Emitter};
use crossbeam_channel;
use std::time::Duration;

// ✅ 只保留时域处理相关的常量
const FRAME_INTERVAL_MS: u64 = 33;

pub struct EegProcessor {
    stream_info: StreamInfo,
    app_handle: AppHandle,
    data_rx: Option<crossbeam_channel::Receiver<EegSample>>,
    recorder: Arc<Mutex<Option<EdfRecorder>>>,
    is_running: Arc<tokio::sync::RwLock<bool>>,
    thread_handles: Vec<tokio::task::JoinHandle<()>>,
    fft_processor: Option<FftProcessor>, // ✅ 添加FFT处理器
}

impl EegProcessor {
    pub fn new(stream_info: StreamInfo, app_handle: AppHandle) -> Result<Self, AppError> {
        let processor = Self {
            stream_info: stream_info.clone(),
            app_handle,
            data_rx: None,
            recorder: Arc::new(Mutex::new(None)),
            is_running: Arc::new(tokio::sync::RwLock::new(false)),
            thread_handles: Vec::new(),
            fft_processor: None, // 延迟初始化
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
        let threads_spawned = self.thread_handles.len() as u32;
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
            stream_info: self.stream_info.clone(),
            recording_stats: recording_stats.clone(),
            threads_spawned,
        };
        
        // ✅ 实际使用统计字段
        println!("📊 EEG Processor stopped:");
        println!("   - Stream: {} ({}Hz, {} channels)", 
                 stats.stream_info.name, 
                 stats.stream_info.sample_rate, 
                 stats.stream_info.channels_count);
        println!("   - Threads spawned: {}", stats.threads_spawned);
        
        if let Some(ref rec_stats) = stats.recording_stats {
            println!("   - Recording stats:");
            println!("     • Samples recorded: {}", rec_stats.samples_written);
            println!("     • Duration: {:.2}s", rec_stats.duration_seconds);
            println!("     • File size: {} bytes", rec_stats.file_size_bytes);
        } else {
            println!("   - No recording session");
        }
        
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
    
    /// ✅ 数据分发器 - 确保每个样本都复制给所有消费者
    async fn spawn_data_distributor(
        &self,
        data_rx: crossbeam_channel::Receiver<EegSample>,
        recording_tx: crossbeam_channel::Sender<EegSample>,
        time_domain_tx: crossbeam_channel::Sender<EegSample>,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🟣 Data distributor started - ensuring no data loss");
            
            let mut samples_distributed = 0u64;
            let mut recording_failures = 0u64;
            let mut time_domain_failures = 0u64;
            let mut last_stats_time = std::time::Instant::now();
            
            loop {
                // 非阻塞检查停止状态
                {
                    let running = is_running.try_read();
                    if let Ok(running) = running {
                        if !*running {
                            println!("🟣 Data distributor stopping");
                            break;
                        }
                    }
                }
                
                // ✅ 阻塞接收确保不丢失任何样本
                match data_rx.recv() {
                    Ok(sample) => {
                        samples_distributed += 1;
                        
                        // ✅ 克隆样本并分发到所有消费者
                        let sample_for_recording = sample.clone();
                        let sample_for_time_domain = sample;
                        
                        // 分发到录制线程（高优先级）
                        if let Err(_) = recording_tx.send(sample_for_recording) {
                            recording_failures += 1;
                            if recording_failures <= 5 {
                                println!("⚠️ Recording channel dropped (failure #{})", recording_failures);
                            }
                        }
                        
                        // 分发到时域收集器
                        if let Err(_) = time_domain_tx.send(sample_for_time_domain) {
                            time_domain_failures += 1;
                            if time_domain_failures <= 5 {
                                println!("⚠️ Time domain channel dropped (failure #{})", time_domain_failures);
                            }
                        }
                        
                        // ✅ 每秒统计分发状态
                        if last_stats_time.elapsed() >= Duration::from_secs(1) {
                            println!("🟣 Distributor: {}Hz distributed, failures: rec={}, time={}", 
                                     samples_distributed, recording_failures, time_domain_failures);
                            last_stats_time = std::time::Instant::now();
                        }
                        
                        // 如果两个通道都断开，退出分发器
                        if recording_failures > 0 && time_domain_failures > 0 {
                            println!("🟣 All consumers disconnected, distributor stopping");
                            break;
                        }
                    }
                    Err(_) => {
                        println!("🟣 Data distributor: source disconnected");
                        break;
                    }
                }
            }
            
            println!("🟣 Data distributor stopped - total distributed: {}, failures: rec={}, time={}", 
                     samples_distributed, recording_failures, time_domain_failures);
        })
    }
    
    /// 全crossbeam处理管道
    async fn start_crossbeam_pipeline(
        &mut self,
        data_rx: crossbeam_channel::Receiver<EegSample>,
    ) -> Result<(), AppError> {
        let stream_info = self.stream_info.clone();
        let app_handle = self.app_handle.clone();
        let recorder = self.recorder.clone();
        let is_running = self.is_running.clone();
        
        // ✅ 初始化FFT处理器
        self.fft_processor = Some(FftProcessor::new(
            stream_info.clone(),
            is_running.clone(),
        ));
        
        // ✅ 创建分发通道 - 避免数据竞争
        let (recording_tx, recording_rx) = crossbeam_channel::unbounded::<EegSample>();
        let (time_domain_data_tx, time_domain_data_rx) = crossbeam_channel::unbounded::<EegSample>();
        
        // 下游通道保持不变
        let (freq_tx, freq_rx) = crossbeam_channel::unbounded();
        let (time_domain_tx, time_domain_rx) = crossbeam_channel::unbounded();
        let (fft_trigger_tx, fft_trigger_rx) = crossbeam_channel::unbounded();
        
        // ✅ 数据分发器 - 第一优先级线程
        let distributor_handle = self.spawn_data_distributor(
            data_rx,                    // 从LSL接收
            recording_tx,               // 分发给录制线程
            time_domain_data_tx,        // 分发给时域收集器
            is_running.clone()
        ).await;
        self.thread_handles.push(distributor_handle);
        
        // ✅ 录制线程 - 使用专用通道，不再竞争
        let recording_handle = self.spawn_recording_thread(
            recording_rx,               // 专用录制通道
            recorder,
            is_running.clone()
        ).await;
        self.thread_handles.push(recording_handle);
        
        // ✅ 时域收集器 - 使用专用通道，不再竞争
        let time_domain_handle = self.spawn_time_domain_collector(
            time_domain_data_rx,        // 专用时域通道
            time_domain_tx,
            fft_trigger_tx,
            stream_info.clone(),
            is_running.clone()
        ).await;
        self.thread_handles.push(time_domain_handle);
        
        // FFT线程和前端线程保持不变
        if let Some(fft_processor) = &self.fft_processor {
            let fft_handle = fft_processor.spawn_fft_thread(
                fft_trigger_rx,
                freq_tx,
            ).await;
            self.thread_handles.push(fft_handle);
        }
        
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
    
    /// 录制线程 - 最高优先级，专用通道，确保数据完整性
    async fn spawn_recording_thread(
        &self,
        recording_rx: crossbeam_channel::Receiver<EegSample>,  // ✅ 专用通道
        recorder: Arc<Mutex<Option<EdfRecorder>>>,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🔴 Recording thread started (DEDICATED CHANNEL)");
            
            let mut samples_recorded = 0u64;
            let mut recording_errors = 0u64;
            let mut last_report = std::time::Instant::now();
            
            loop {
                // ✅ 阻塞接收，确保不丢失任何样本
                match recording_rx.recv() {
                    Ok(sample) => {
                        // 非阻塞检查停止状态
                        {
                            let running = is_running.try_read();
                            if let Ok(running) = running {
                                if !*running {
                                    // 即使停止，也要处理完当前样本
                                    println!("🔴 Recording stopping after processing current sample");
                                }
                            }
                        }
                        
                        // 录制样本
                        let mut recorder_guard = recorder.lock().await;
                        if let Some(recorder) = recorder_guard.as_mut() {
                            match recorder.write_sample(&sample) {
                                Ok(_) => {
                                    samples_recorded += 1;
                                    
                                    // 每秒报告录制状态
                                    if last_report.elapsed() >= Duration::from_secs(1) {
                                        println!("🔴 Recording: {}Hz (errors: {})", 
                                                 samples_recorded, recording_errors);
                                        last_report = std::time::Instant::now();
                                    }
                                }
                                Err(e) => {
                                    recording_errors += 1;
                                    if recording_errors <= 10 {
                                        println!("❌ Recording error #{}: {}", recording_errors, e);
                                    }
                                }
                            }
                        }
                        
                        // 检查停止状态（在处理完样本后）
                        {
                            let running = is_running.try_read();
                            if let Ok(running) = running {
                                if !*running {
                                    break;
                                }
                            }
                        }
                    }
                    Err(_) => {
                        println!("🔴 Recording: data distributor disconnected");
                        break;
                    }
                }
            }
            
            println!("🔴 Recording thread stopped - recorded: {}, errors: {}", 
                     samples_recorded, recording_errors);
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
    

    /// 前端发送线程 - 使用FFT工具函数
    async fn spawn_frontend_thread(
        &self,
        freq_rx: crossbeam_channel::Receiver<(u64, Vec<FreqData>)>,
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
            
            let mut freq_buffer: std::collections::HashMap<u64, Vec<FreqData>> = std::collections::HashMap::new();
            let mut time_buffer: std::collections::HashMap<u64, EegBatch> = std::collections::HashMap::new();
            
            let mut frame_count = 0u64;
            let mut next_expected_batch_id = 0u64;
            
            // ✅ 使用FFT模块的工具函数
            let create_empty_freq_data = move || fft_utils::create_empty_freq_data(channels_count);
            
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
                        
                        // 收集数据到缓冲区
                        while let Ok((batch_id, freq_data)) = freq_rx.try_recv() {
                            freq_buffer.insert(batch_id, freq_data);
                        }
                        
                        while let Ok(time_domain) = time_domain_rx.try_recv() {
                            time_buffer.insert(time_domain.batch_id, time_domain);
                        }
                        
                        // 发送匹配的数据对
                        let mut sent_data = false;
                        
                        if let (Some(time_domain), freq_data) = (
                            time_buffer.remove(&next_expected_batch_id),
                            freq_buffer.remove(&next_expected_batch_id)
                        ) {
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
                        
                        // 清理缓冲区
                        let cleanup_threshold = next_expected_batch_id.saturating_sub(10);
                        freq_buffer.retain(|&batch_id, _| batch_id >= cleanup_threshold);
                        time_buffer.retain(|&batch_id, _| batch_id >= cleanup_threshold);
                        
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

/// 新增：EEG处理器统计信息
#[derive(Debug, Clone)]
pub struct EegProcessorStats {
    pub stream_info: StreamInfo,
    pub recording_stats: Option<crate::recorder::RecordingStats>,
    pub threads_spawned: u32,
}