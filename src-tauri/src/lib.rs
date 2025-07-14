use std::sync::Arc;
use std::thread;
use std::time::Duration;
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio::sync::Mutex;
use lsl::Pullable;
use edfplus::{EdfWriter, SignalParam};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EegSample {
    pub timestamp: f64,
    pub channels: Vec<f32>,
    pub sample_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EegBatch {
    pub samples: Vec<EegSample>,
    pub batch_id: u64,
    pub channels_count: usize,
    pub sample_rate: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub name: String,
    pub stream_type: String,
    pub channels_count: usize,
    pub sample_rate: f32,
    pub is_connected: bool,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LslStreamInfo {
    pub name: String,
    pub stream_type: String,
    pub channels_count: usize,
    pub sample_rate: f32,
    pub source_id: String,
    pub hostname: String,
}

pub struct EegDataManager {
    sender: Sender<EegSample>,
    receiver: Receiver<EegSample>,
    is_recording: Arc<Mutex<bool>>,
    recording_writer: Arc<Mutex<Option<EdfWriter>>>,
    stream_info: Arc<Mutex<Option<StreamInfo>>>,
    // 移除inlet字段，改用通道通信控制LSL线程
    lsl_control_sender: Sender<LslCommand>,
}

#[derive(Debug)]
enum LslCommand {
    Connect(String), // stream_name
    Disconnect,
    Stop,
}

impl EegDataManager {
    pub fn new() -> Self {
        let (sender, receiver) = unbounded();
        let (lsl_control_sender, lsl_control_receiver) = unbounded();
        
        // 启动LSL处理线程
        let data_sender = sender.clone();
        thread::spawn(move || {
            Self::lsl_worker_thread(lsl_control_receiver, data_sender);
        });
        
        Self {
            sender,
            receiver,
            is_recording: Arc::new(Mutex::new(false)),
            recording_writer: Arc::new(Mutex::new(None)),
            stream_info: Arc::new(Mutex::new(None)),
            lsl_control_sender,
        }
    }

    // LSL工作线程，在独立线程中处理所有LSL操作
    fn lsl_worker_thread(control_receiver: Receiver<LslCommand>, data_sender: Sender<EegSample>) {
        let mut current_inlet: Option<lsl::StreamInlet> = None;
        let mut sample_id = 0u64;
        let mut sample_buffer = Vec::<f32>::new(); // 预分配的缓冲区
        
        loop {
            // 检查控制命令
            if let Ok(command) = control_receiver.try_recv() {
                match command {
                    LslCommand::Connect(stream_name) => {
                        println!("LSL Worker: Connecting to stream '{}'", stream_name);
                        let pred = format!("name='{}'", stream_name);
                        
                        match lsl::resolve_bypred(&pred, 1, 10.0) {
                            Ok(mut streams) if !streams.is_empty() => {
                                let stream = streams.remove(0);
                                match lsl::StreamInlet::new(&stream, 360, 0, true) {
                                    Ok(inlet) => {
                                        println!("LSL Worker: Successfully connected to stream");
                                        current_inlet = Some(inlet);
                                    },
                                    Err(e) => {
                                        eprintln!("LSL Worker: Failed to create inlet: {:?}", e);
                                    }
                                }
                            },
                            Ok(_) => {
                                eprintln!("LSL Worker: No streams found with name '{}'", stream_name);
                            },
                            Err(e) => {
                                eprintln!("LSL Worker: Failed to resolve stream: {:?}", e);
                            }
                        }
                    },
                    LslCommand::Disconnect => {
                        println!("LSL Worker: Disconnecting");
                        current_inlet = None;
                    },
                    LslCommand::Stop => {
                        println!("LSL Worker: Stopping");
                        break;
                    }
                }
            }
            
            // 如果有连接，尝试读取数据
            if let Some(ref inlet) = current_inlet {
                match inlet.pull_sample_buf(&mut sample_buffer, 0.1) { // 100ms超时
                    Ok(timestamp) => {
                        if timestamp != 0.0 { // 非零时间戳表示有新数据
                            let sample = EegSample {
                                timestamp,
                                channels: sample_buffer.clone(), // 克隆缓冲区数据
                                sample_id,
                            };
                            
                            if data_sender.try_send(sample).is_err() {
                                eprintln!("LSL Worker: Data channel is full, dropping sample");
                            }
                            
                            sample_id += 1;
                        }
                    },
                    Err(_) => {
                        // 超时或其他错误，继续循环
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            } else {
                // 没有连接，稍微等待
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    pub async fn discover_lsl_streams(&self) -> Result<Vec<LslStreamInfo>, String> {
        // 在新线程中执行LSL发现，避免阻塞async runtime
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        thread::spawn(move || {
            let result = match lsl::resolve_byprop("type", "EEG", 10, 5.0) {
                Ok(streams) => {
                    let stream_infos: Vec<LslStreamInfo> = streams.iter().map(|stream| {
                        // 使用正确的LSL API方法
                        // 注意：根据LSL文档，某些方法可能返回String而不是Option<String>
                        let name = format!("Stream_{}", stream.source_id()); // 临时使用source_id作为名称
                        let stream_type = "EEG".to_string(); // 暂时硬编码，因为我们是通过type="EEG"查询的
                        
                        LslStreamInfo {
                            name,
                            stream_type,
                            channels_count: stream.channel_count() as usize,
                            sample_rate: stream.nominal_srate() as f32,
                            source_id: stream.source_id(),
                            hostname: stream.hostname(),
                        }
                    }).collect();
                    Ok(stream_infos)
                },
                Err(e) => Err(format!("Failed to discover LSL streams: {:?}", e))
            };
            let _ = tx.send(result);
        });
        
        match rx.await {
            Ok(result) => result,
            Err(_) => Err("LSL discovery task failed".to_string())
        }
    }

    pub async fn connect_to_stream(&self, stream_name: &str, app_handle: tauri::AppHandle) -> Result<(), String> {
        // 首先获取流信息
        let stream_name = stream_name.to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let stream_name_clone = stream_name.clone();
        
        thread::spawn(move || {
            let pred = format!("name='{}'", stream_name_clone);
            let result = match lsl::resolve_bypred(&pred, 1, 10.0) {
                Ok(mut streams) if !streams.is_empty() => {
                    let stream = streams.remove(0);
                    let info = StreamInfo {
                        name: format!("Stream_{}", stream.source_id()), // 临时使用source_id
                        stream_type: "EEG".to_string(), // 硬编码类型
                        channels_count: stream.channel_count() as usize,
                        sample_rate: stream.nominal_srate() as f32,
                        is_connected: true,
                        source_id: stream.source_id(),
                    };
                    Ok(info)
                },
                Ok(_) => Err("No streams found with the specified name".to_string()),
                Err(e) => Err(format!("Failed to resolve stream: {:?}", e))
            };
            let _ = tx.send(result);
        });
        
        match rx.await {
            Ok(Ok(info)) => {
                // 更新流信息
                {
                    let mut stream_info_guard = self.stream_info.lock().await;
                    *stream_info_guard = Some(info);
                }
                
                // 发送连接命令给LSL工作线程
                if let Err(_) = self.lsl_control_sender.try_send(LslCommand::Connect(stream_name)) {
                    return Err("Failed to send connect command to LSL worker".to_string());
                }
                
                // 启动数据发送线程
                self.start_data_sender_thread(app_handle).await;
                
                Ok(())
            },
            Ok(Err(e)) => Err(e),
            Err(_) => Err("LSL connection task failed".to_string())
        }
    }

    async fn start_data_sender_thread(&self, app_handle: tauri::AppHandle) {
        let receiver = self.receiver.clone();
        let recording_writer = self.recording_writer.clone();
        let is_recording = self.is_recording.clone();
        
        tokio::spawn(async move {
            Self::data_sender_loop(receiver, app_handle, recording_writer, is_recording).await;
        });
    }

    async fn data_sender_loop(
        receiver: Receiver<EegSample>, 
        app_handle: tauri::AppHandle,
        recording_writer: Arc<Mutex<Option<EdfWriter>>>,
        is_recording: Arc<Mutex<bool>>
    ) {
        let mut batch_id = 0u64;
        let batch_size = 15; // 60Hz发送批次
        let interval = Duration::from_millis(16); // ~60Hz
        
        loop {
            let mut samples = Vec::new();
            
            // 收集一批数据
            for _ in 0..batch_size {
                if let Ok(sample) = receiver.try_recv() {
                    // 如果正在录制，写入EDF文件
                    let is_rec = *is_recording.lock().await;
                    if is_rec {
                        let mut writer_guard = recording_writer.lock().await;
                        if let Some(ref mut writer) = *writer_guard {
                            // 将单个样本转换为每通道的样本数组
                            let channel_samples: Vec<Vec<f64>> = sample.channels.iter()
                                .map(|&val| vec![val as f64])
                                .collect();
                            
                            if let Err(e) = writer.write_samples(&channel_samples) {
                                eprintln!("Failed to write EDF sample: {:?}", e);
                            }
                        }
                    }
                    
                    samples.push(sample);
                } else {
                    break;
                }
            }
            
            if !samples.is_empty() {
                let channels_count = samples[0].channels.len();
                let batch = EegBatch {
                    samples,
                    batch_id,
                    channels_count,
                    sample_rate: 250.0, // 将根据实际流更新
                };
                
                if let Err(e) = app_handle.emit("eeg-data", &batch) {
                    eprintln!("Failed to emit EEG data: {}", e);
                }
                
                batch_id += 1;
            }
            
            tokio::time::sleep(interval).await;
        }
    }

    pub async fn start_recording(&self, filename: String) -> Result<(), String> {
        let stream_info_guard = self.stream_info.lock().await;
        let stream_info = stream_info_guard.as_ref()
            .ok_or("No stream connected")?
            .clone();
        drop(stream_info_guard);
        
        // 在新线程中创建EDF writer
        let (tx, rx) = tokio::sync::oneshot::channel();
        let filename_clone = filename.clone();
        
        thread::spawn(move || {
            let result = (|| -> Result<EdfWriter, String> {
                let mut writer = EdfWriter::create(&filename_clone)
                    .map_err(|e| format!("Failed to create EDF file: {:?}", e))?;
                
                writer.set_patient_info("EEG_Patient", "U", "01-JAN-2000", "LSL Recording")
                    .map_err(|e| format!("Failed to set patient info: {:?}", e))?;
                
                // 为每个通道添加信号参数
                for ch in 0..stream_info.channels_count {
                    let signal = SignalParam {
                        label: format!("EEG_CH{}", ch + 1),
                        samples_in_file: 0,
                        physical_max: 200.0,
                        physical_min: -200.0,
                        digital_max: 32767,
                        digital_min: -32768,
                        samples_per_record: 1, // 每个记录1个样本
                        physical_dimension: "uV".to_string(),
                        prefilter: "HP:0.1Hz LP:70Hz".to_string(),
                        transducer: "AgAgCl electrodes".to_string(),
                    };
                    
                    writer.add_signal(signal)
                        .map_err(|e| format!("Failed to add signal: {:?}", e))?;
                }
                
                Ok(writer)
            })();
            let _ = tx.send(result);
        });
        
        match rx.await {
            Ok(Ok(writer)) => {
                let mut recording_writer_guard = self.recording_writer.lock().await;
                *recording_writer_guard = Some(writer);
                drop(recording_writer_guard);
                
                let mut is_recording_guard = self.is_recording.lock().await;
                *is_recording_guard = true;
                drop(is_recording_guard);
                
                println!("Started recording to: {}", filename);
                Ok(())
            },
            Ok(Err(e)) => Err(e),
            Err(_) => Err("EDF writer creation task failed".to_string())
        }
    }

    pub async fn stop_recording(&self) -> Result<(), String> {
        let mut is_recording_guard = self.is_recording.lock().await;
        *is_recording_guard = false;
        drop(is_recording_guard);
        
        let mut writer_guard = self.recording_writer.lock().await;
        if let Some(writer) = writer_guard.take() {
            drop(writer_guard);
            
            // 在新线程中finalize EDF文件
            let (tx, rx) = tokio::sync::oneshot::channel();
            thread::spawn(move || {
                let result = writer.finalize()
                    .map_err(|e| format!("Failed to finalize EDF file: {:?}", e));
                let _ = tx.send(result);
            });
            
            match rx.await {
                Ok(Ok(())) => {
                    println!("Recording stopped and file finalized");
                    Ok(())
                },
                Ok(Err(e)) => Err(e),
                Err(_) => Err("EDF finalization task failed".to_string())
            }
        } else {
            Ok(())
        }
    }

    pub async fn get_stream_info(&self) -> Option<StreamInfo> {
        self.stream_info.lock().await.clone()
    }

    pub async fn disconnect_stream(&self) -> Result<(), String> {
        // 发送断开连接命令给LSL工作线程
        if let Err(_) = self.lsl_control_sender.try_send(LslCommand::Disconnect) {
            return Err("Failed to send disconnect command to LSL worker".to_string());
        }
        
        let mut stream_info_guard = self.stream_info.lock().await;
        if let Some(ref mut info) = *stream_info_guard {
            info.is_connected = false;
        }
        
        Ok(())
    }
}

// Tauri commands
#[tauri::command]
async fn discover_lsl_streams(
    data_manager: State<'_, EegDataManager>,
) -> Result<Vec<LslStreamInfo>, String> {
    data_manager.discover_lsl_streams().await
}

#[tauri::command]
async fn connect_to_stream(
    data_manager: State<'_, EegDataManager>,
    stream_name: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    data_manager.connect_to_stream(&stream_name, app_handle).await
}

#[tauri::command]
async fn disconnect_stream(
    data_manager: State<'_, EegDataManager>,
) -> Result<(), String> {
    data_manager.disconnect_stream().await
}

#[tauri::command]
async fn start_recording(
    data_manager: State<'_, EegDataManager>,
    filename: String,
) -> Result<(), String> {
    data_manager.start_recording(filename).await
}

#[tauri::command]
async fn stop_recording(
    data_manager: State<'_, EegDataManager>,
) -> Result<(), String> {
    data_manager.stop_recording().await
}

#[tauri::command]
async fn get_stream_info(
    data_manager: State<'_, EegDataManager>,
) -> Result<Option<StreamInfo>, String> {
    Ok(data_manager.get_stream_info().await)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(EegDataManager::new())
        .invoke_handler(tauri::generate_handler![
            discover_lsl_streams,
            connect_to_stream,
            disconnect_stream,
            start_recording,
            stop_recording,
            get_stream_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
