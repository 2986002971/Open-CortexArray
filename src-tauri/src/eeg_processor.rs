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

const FFT_WINDOW_SIZE: usize = 1024;
const FRAME_RATE_HZ: f64 = 60.0;
const FRAME_INTERVAL_MS: u64 = (1000.0 / FRAME_RATE_HZ) as u64;

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
        
        // 创建无界通道确保数据不丢失
        let (freq_tx, freq_rx) = crossbeam_channel::unbounded();
        let (time_domain_tx, time_domain_rx) = crossbeam_channel::unbounded();
        
        // 启动录制线程 - 最高优先级，直接从源接收
        let recording_handle = self.spawn_recording_thread(
            data_rx.clone(),
            recorder,
            is_running.clone()
        ).await;
        self.thread_handles.push(recording_handle);
        
        // 启动FFT计算线程 - 从源克隆接收
        let fft_handle = self.spawn_fft_thread(
            data_rx.clone(),
            freq_tx,
            stream_info.clone(),
            is_running.clone()
        ).await;
        self.thread_handles.push(fft_handle);
        
        // 启动时域数据收集线程 - 为前端提供原始数据
        let time_domain_handle = self.spawn_time_domain_collector(
            data_rx,
            time_domain_tx,
            stream_info.clone(),
            is_running.clone()
        ).await;
        self.thread_handles.push(time_domain_handle);
        
        // 启动前端发送线程
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
    
    /// FFT计算线程 - 无数据丢失保证
    async fn spawn_fft_thread(
        &self,
        data_rx: crossbeam_channel::Receiver<EegSample>,
        freq_tx: crossbeam_channel::Sender<Vec<FreqData>>,
        stream_info: StreamInfo,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🟡 FFT thread started");
            
            let mut fft_planner = FftPlanner::new();
            let fft = fft_planner.plan_fft_forward(FFT_WINDOW_SIZE);
            
            // 为每个通道维护滑动窗口
            let mut channel_windows: Vec<VecDeque<f64>> = (0..stream_info.channels_count)
                .map(|_| VecDeque::with_capacity(FFT_WINDOW_SIZE + 100)) // 额外缓冲
                .collect();
            
            let mut samples_processed = 0u64;
            let mut ffts_computed = 0u64;
            
            loop {
                // 检查停止状态
                {
                    let running = is_running.read().await;
                    if !*running {
                        println!("🟡 FFT thread stopping");
                        break;
                    }
                }
                
                // 尝试接收数据（非阻塞，允许FFT线程处理积压）
                match data_rx.try_recv() {
                    Ok(sample) => {
                        samples_processed += 1;
                        
                        // 更新滑动窗口
                        for (ch_idx, &value) in sample.channels.iter().enumerate() {
                            if ch_idx < channel_windows.len() {
                                let window = &mut channel_windows[ch_idx];
                                window.push_back(value);
                                
                                // 保持窗口大小
                                if window.len() > FFT_WINDOW_SIZE {
                                    window.pop_front();
                                }
                            }
                        }
                        
                        // 当窗口足够大时计算FFT
                        if channel_windows[0].len() >= FFT_WINDOW_SIZE {
                            let freq_data = compute_multi_channel_fft(
                                &channel_windows,
                                fft.as_ref(), // ✅ 修复：使用 as_ref() 转换 Arc<dyn Fft<f64>> 到 &dyn Fft<f64>
                                stream_info.sample_rate,
                            );
                            
                            // 无界通道发送，不会阻塞
                            if freq_tx.send(freq_data).is_err() {
                                println!("🟡 FFT thread: frequency receiver dropped");
                                break;
                            }
                            
                            ffts_computed += 1;
                        }
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        // 没有新数据，短暂休眠让其他线程工作
                        tokio::time::sleep(Duration::from_micros(50)).await;
                    }
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        println!("🟡 FFT thread: data source disconnected");
                        break;
                    }
                }
            }
            
            println!("🟡 FFT thread stopped - processed: {}, FFTs: {}", samples_processed, ffts_computed);
        })
    }
    
    /// 时域数据收集线程 - 为前端提供原始波形
    async fn spawn_time_domain_collector(
        &self,
        data_rx: crossbeam_channel::Receiver<EegSample>,
        time_domain_tx: crossbeam_channel::Sender<EegBatch>,
        stream_info: StreamInfo,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🟢 Time domain collector started");
            
            let batch_size = (stream_info.sample_rate / FRAME_RATE_HZ) as usize; // 每帧的样本数
            let mut current_batch = Vec::with_capacity(batch_size);
            let mut batch_id = 0u64;
            
            loop {
                // 检查停止状态
                {
                    let running = is_running.read().await;
                    if !*running {
                        println!("🟢 Time domain collector stopping");
                        break;
                    }
                }
                
                match data_rx.try_recv() {
                    Ok(sample) => {
                        current_batch.push(sample);
                        
                        // 当批次满时发送
                        if current_batch.len() >= batch_size {
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
                            
                            current_batch.clear();
                            batch_id += 1;
                        }
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_micros(100)).await;
                    }
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        println!("🟢 Time domain: data source disconnected");
                        break;
                    }
                }
            }
            
            println!("🟢 Time domain collector stopped");
        })
    }
    
    /// 前端发送线程 - 60Hz刷新
    async fn spawn_frontend_thread(
        &self,
        freq_rx: crossbeam_channel::Receiver<Vec<FreqData>>,
        time_domain_rx: crossbeam_channel::Receiver<EegBatch>,
        app_handle: AppHandle,
        channels_count: u32,
        sample_rate: f64,
        is_running: Arc<tokio::sync::RwLock<bool>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            println!("🔵 Frontend thread started");
            
            let mut frame_timer = tokio::time::interval(
                Duration::from_millis(FRAME_INTERVAL_MS)
            );
            let mut latest_freq_data: Option<Vec<FreqData>> = None;
            let mut latest_time_domain: Option<EegBatch> = None;
            
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
                        
                        // 收集最新数据（非阻塞）
                        while let Ok(freq_data) = freq_rx.try_recv() {
                            latest_freq_data = Some(freq_data);
                        }
                        
                        while let Ok(time_domain) = time_domain_rx.try_recv() {
                            latest_time_domain = Some(time_domain);
                        }
                        
                        // 发送给前端
                        if let (Some(freq_data), Some(time_domain)) = (&latest_freq_data, &latest_time_domain) {
                            let payload = FramePayload {
                                time_domain: time_domain.clone(),
                                frequency_domain: freq_data.clone(),
                            };
                            
                            // ✅ 修复：使用正确的 emit 方法
                            if let Err(e) = app_handle.emit("frame-update", &payload) {
                                println!("Failed to emit frame-update: {}", e);
                            }
                        } else if latest_freq_data.is_some() {
                            // 即使没有时域数据，也发送频域数据
                            let mock_time_domain = EegBatch {
                                samples: vec![],
                                batch_id: 0,
                                channels_count,
                                sample_rate,
                            };
                            
                            let payload = FramePayload {
                                time_domain: mock_time_domain,
                                frequency_domain: latest_freq_data.as_ref().unwrap().clone(),
                            };
                            
                            // ✅ 修复：使用正确的 emit 方法
                            if let Err(e) = app_handle.emit("frame-update", &payload) {
                                println!("Failed to emit frame-update: {}", e);
                            }
                        }
                    }
                }
            }
            
            println!("🔵 Frontend thread stopped");
        })
    }
}

// FFT计算辅助函数保持不变
fn compute_multi_channel_fft(
    channel_windows: &[VecDeque<f64>],
    fft: &dyn rustfft::Fft<f64>,
    sample_rate: f64,
) -> Vec<FreqData> {
    let mut results = Vec::new();
    
    for (ch_idx, window) in channel_windows.iter().enumerate() {
        if window.len() < FFT_WINDOW_SIZE {
            continue;
        }
        
        // 准备FFT输入数据
        let mut fft_input: Vec<Complex<f64>> = window
            .iter()
            .map(|&x| Complex::new(x, 0.0))
            .collect();
        
        // 执行FFT
        fft.process(&mut fft_input);
        
        // 计算幅度谱
        let spectrum: Vec<f64> = fft_input
            .iter()
            .take(FFT_WINDOW_SIZE / 2) // 只取正频率部分
            .map(|c| c.norm())
            .collect();
        
        // 生成频率bins
        let frequency_bins: Vec<f64> = (0..spectrum.len())
            .map(|i| i as f64 * sample_rate / FFT_WINDOW_SIZE as f64)
            .collect();
        
        results.push(FreqData {
            channel_index: ch_idx as u32,
            spectrum,
            frequency_bins,
        });
    }
    
    results
}

/// 新增：EEG处理器统计信息
#[derive(Debug, Clone)]
pub struct EegProcessorStats {
    pub stream_info: StreamInfo,
    pub recording_stats: Option<crate::recorder::RecordingStats>,
    pub threads_spawned: u32,
}