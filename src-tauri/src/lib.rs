mod lsl_manager;
mod data_types;
mod eeg_processor;
mod recorder;
mod error;

use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::State;

use data_types::*;
use lsl_manager::LslManager;
use eeg_processor::EegProcessor;

// 全局应用状态 - 重新设计
#[derive(Default)]
struct AppState {
    lsl_manager: Arc<Mutex<Option<LslManager>>>,        // ✅ 可选的LSL管理器
    eeg_processor: Arc<Mutex<Option<EegProcessor>>>,    // ✅ 可选的数据处理器
}

// Tauri命令接口实现

#[tauri::command]
async fn discover_lsl_streams(
    state: State<'_, AppState>
) -> Result<Vec<LslStreamInfo>, String> {
    // ✅ 修复：获取可变引用
    let mut manager_guard = state.lsl_manager.lock().await;
    
    if let Some(manager) = manager_guard.as_mut() {
        manager.discover_streams()
            .await
            .map_err(|e| e.to_string())
    } else {
        // 如果没有管理器，先创建一个临时的来发现流
        let mut temp_manager = LslManager::new();
        temp_manager.start().await.map_err(|e| e.to_string())?;
        
        let result = temp_manager.discover_streams()
            .await
            .map_err(|e| e.to_string());
        
        temp_manager.stop().await.map_err(|e| e.to_string())?;
        result
    }
}

#[tauri::command]
async fn connect_to_stream(
    stream_name: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle
) -> Result<StreamInfo, String> {
    println!("🔌 Connecting to stream: {}", stream_name);
    
    // Step 1: 停止现有连接（消费式）
    {
        let mut processor_guard = state.eeg_processor.lock().await;
        if let Some(processor) = processor_guard.take() {
            println!("🛑 Stopping existing processor");
            let stats = processor.stop().await.map_err(|e| e.to_string())?;
            println!("📊 Processor stats: {:?}", stats);
        }
    }
    
    {
        let mut manager_guard = state.lsl_manager.lock().await;
        if let Some(manager) = manager_guard.take() {
            println!("🛑 Stopping existing LSL manager");
            let stats = manager.stop().await.map_err(|e| e.to_string())?;
            println!("📊 Manager stats: {:?}", stats);
        }
    }
    
    // Step 2: 创建新的LSL管理器并连接
    let mut manager = LslManager::new();
    
    manager.start().await.map_err(|e| e.to_string())?;
    
    let stream_info = manager.connect_to_stream(&stream_name)
        .await
        .map_err(|e| e.to_string())?;
    
    println!("✅ Connected to stream: {} ({} channels @ {}Hz)", 
             stream_info.name, stream_info.channels_count, stream_info.sample_rate);
    
    // Step 3: 获取数据通道
    let data_rx = manager.get_data_receiver()
        .ok_or("Failed to get data receiver from LSL manager")?;
    
    // Step 4: 创建EEG处理器
    let mut processor = EegProcessor::new(stream_info.clone(), app.clone())
        .map_err(|e| e.to_string())?;
    
    // Step 5: 设置数据源并启动处理器
    processor.set_data_source(data_rx);
    processor.start().await.map_err(|e| e.to_string())?;
    
    println!("🚀 EEG processor started");
    
    // Step 6: 保存状态
    {
        let mut manager_guard = state.lsl_manager.lock().await;
        *manager_guard = Some(manager);
    }
    
    {
        let mut processor_guard = state.eeg_processor.lock().await;
        *processor_guard = Some(processor);
    }
    
    println!("💾 Connection state saved");
    
    Ok(stream_info)
}

// 极简版本
#[tauri::command]
async fn disconnect_stream(
    state: State<'_, AppState>
) -> Result<String, String> {
    println!("🔌 Disconnecting stream");
    
    let mut components_stopped = 0;
    
    // 停止处理器
    {
        let mut processor_guard = state.eeg_processor.lock().await;
        if let Some(processor) = processor_guard.take() {
            println!("🛑 Stopping EEG processor");
            if let Err(e) = processor.stop().await {
                println!("⚠️  Error stopping processor: {}", e);
            } else {
                components_stopped += 1;
            }
        }
    }
    
    // 停止管理器
    {
        let mut manager_guard = state.lsl_manager.lock().await;
        if let Some(manager) = manager_guard.take() {
            println!("🛑 Stopping LSL manager");
            if let Err(e) = manager.stop().await {
                println!("⚠️  Error stopping manager: {}", e);
            } else {
                components_stopped += 1;
            }
        }
    }
    
    println!("✅ Stream disconnected successfully");
    
    if components_stopped > 0 {
        Ok(format!("Successfully disconnected {} components", components_stopped))
    } else {
        Ok("No active connections to disconnect".to_string())
    }
}

#[tauri::command]
async fn get_stream_info(
    state: State<'_, AppState>
) -> Result<Option<StreamInfo>, String> {
    let manager_guard = state.lsl_manager.lock().await;
    
    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager.get_current_stream_info().await)
    } else {
        Ok(None)
    }
}

#[tauri::command]
async fn start_recording(
    filename: String,
    state: State<'_, AppState>
) -> Result<(), String> {
    println!("🔴 Starting recording: {}", filename);
    
    let processor_guard = state.eeg_processor.lock().await;
    
    if let Some(processor) = processor_guard.as_ref() {
        processor.start_recording(&filename)
            .await
            .map_err(|e| e.to_string())
    } else {
        Err("No active stream connection".to_string())
    }
}

#[tauri::command]
async fn stop_recording(
    state: State<'_, AppState>
) -> Result<(), String> {
    println!("⏹️  Stopping recording");
    
    let processor_guard = state.eeg_processor.lock().await;
    
    if let Some(processor) = processor_guard.as_ref() {
        processor.stop_recording()
            .await
            .map_err(|e| e.to_string())
    } else {
        Err("No active stream connection".to_string())
    }
}

#[tauri::command]
async fn get_connection_status(
    state: State<'_, AppState>
) -> Result<ConnectionStatus, String> {
    let manager_guard = state.lsl_manager.lock().await;
    let processor_guard = state.eeg_processor.lock().await;
    
    let status = ConnectionStatus {
        is_lsl_connected: manager_guard.is_some(),
        is_processor_running: processor_guard.is_some(),
        current_stream: if let Some(manager) = manager_guard.as_ref() {
            manager.get_current_stream_info().await
        } else {
            None
        },
    };
    
    Ok(status)
}

#[tauri::command]
async fn initialize_system(
    state: State<'_, AppState>
) -> Result<(), String> {
    println!("🚀 Initializing EEG system");
    
    // 检查是否已经初始化
    let manager_guard = state.lsl_manager.lock().await;
    if manager_guard.is_some() {
        return Ok(()); // 已经初始化
    }
    drop(manager_guard);
    
    // 系统初始化逻辑可以在这里添加
    // 例如：检查LSL库是否可用、设备权限等
    
    println!("✅ EEG system initialized");
    Ok(())
}

#[tauri::command]
async fn shutdown_system(
    state: State<'_, AppState>
) -> Result<(), String> {
    println!("🔌 Shutting down EEG system");
    
    // 优雅关闭所有组件
    disconnect_stream(state).await?;
    
    println!("✅ EEG system shutdown complete");
    Ok(())
}

// 新增：获取系统健康状态
#[tauri::command]
async fn get_system_health(
    state: State<'_, AppState>
) -> Result<SystemHealth, String> {
    let manager_guard = state.lsl_manager.lock().await;
    let processor_guard = state.eeg_processor.lock().await;
    
    let health = SystemHealth {
        lsl_manager_status: if manager_guard.is_some() { 
            "Running".to_string() 
        } else { 
            "Stopped".to_string() 
        },
        processor_status: if processor_guard.is_some() { 
            "Running".to_string() 
        } else { 
            "Stopped".to_string() 
        },
        memory_usage_mb: 0, // TODO: 实现内存监控
        uptime_seconds: 0,  // TODO: 实现运行时间统计
    };
    
    Ok(health)
}

// Tauri应用配置
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    println!("🧠 Starting Open-CortexArray EEG Visualization System");
    
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            discover_lsl_streams,
            connect_to_stream,
            disconnect_stream,
            get_stream_info,
            start_recording,
            stop_recording,
            get_connection_status,
            initialize_system,
            shutdown_system,
            get_system_health
        ])
        .setup(|app| {
            println!("🎯 EEG Visualization Backend Started");
            println!("📡 Ready to discover LSL streams");
            println!("🖥️  Frontend interface available");
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { .. } => {
                    println!("🔌 Window closing, shutting down gracefully");
                    // TODO: 在这里可以添加优雅关闭逻辑
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}