//! 开发用的测试LSL服务器
//! 
//! 运行: cargo run --example test_lsl_server

use lsl;
use lsl::ExPushable;
use rand::Rng;
use std::time::{Duration, Instant};
use std::thread;

fn main() -> Result<(), lsl::Error> {
    println!("🧪 Starting Test LSL Server for Open-CortexArray");
    println!("=================================================");
    
    // 创建测试流
    let configs = vec![
        ("TestEEG_8ch", 8, 250.0),
        ("TestEEG_32ch", 32, 500.0),
        ("MockBiosemi", 64, 1000.0),
    ];
    
    let handles: Vec<_> = configs.into_iter().map(|(name, channels, rate)| {
        let name = name.to_string();
        thread::spawn(move || {
            if let Err(e) = start_test_stream(&name, channels, rate) {
                eprintln!("❌ Stream {} error: {}", name, e);
            }
        })
    }).collect();
    
    println!("📡 All test streams started. Press Ctrl+C to stop.");
    
    // 等待所有线程
    for handle in handles {
        handle.join().unwrap();
    }
    
    Ok(())
}

fn start_test_stream(name: &str, channels: u32, sample_rate: f64) -> Result<(), lsl::Error> {
    let mut info = lsl::StreamInfo::new(
        name,
        "EEG",
        channels,
        sample_rate,
        lsl::ChannelFormat::Double64,
        &format!("opencortex_test_{}", name),
    )?;
    
    // 添加通道标签
    let mut channels_node = info.desc().append_child("channels");
    for i in 0..channels {
        channels_node.append_child("channel")
            .append_child_value("label", &format!("Ch{}", i + 1))
            .append_child_value("unit", "microvolts")
            .append_child_value("type", "EEG");
    }
    
    let outlet = lsl::StreamOutlet::new(&info, 0, 360)?;
    println!("✅ Stream '{}' started ({} ch @ {} Hz)", name, channels, sample_rate);
    
    let mut rng = rand::thread_rng();
    let mut sample_count = 0u64;
    let sample_interval = Duration::from_secs_f64(1.0 / sample_rate);
    let mut next_time = Instant::now();
    
    loop {
        // 精确时间控制
        let now = Instant::now();
        if now < next_time {
            thread::sleep(next_time - now);
        }
        
        // 生成真实的脑电信号模拟
        let mut sample = Vec::with_capacity(channels as usize);
        let time_sec = sample_count as f64 / sample_rate;
        
        for i in 0..channels {
            let value = generate_realistic_eeg_signal(i, time_sec, &mut rng);
            sample.push(value);
        }
        
        if outlet.push_sample_ex(&sample, lsl::local_clock(), true).is_err() {
            println!("🔌 Stream '{}' disconnected", name);
            break;
        }
        
        sample_count += 1;
        next_time += sample_interval;
        
        // 状态报告
        if sample_count % (sample_rate as u64 * 30) == 0 {
            println!("📊 [{}] {} samples sent", name, sample_count);
        }
    }
    
    Ok(())
}

fn generate_realistic_eeg_signal(channel: u32, time_sec: f64, rng: &mut rand::rngs::ThreadRng) -> f64 {
    use std::f64::consts::PI;
    
    // 基础频率组件 (模拟脑电波段)
    let alpha = 25.0 * (2.0 * PI * 10.0 * time_sec).sin();  // Alpha波 (8-12Hz)
    let beta = 15.0 * (2.0 * PI * 20.0 * time_sec).sin();   // Beta波 (13-30Hz)
    let theta = 35.0 * (2.0 * PI * 6.0 * time_sec).sin();   // Theta波 (4-7Hz)
    
    // 通道特异性 (不同位置的电极有不同特征)
    let channel_factor = match channel % 4 {
        0 => 1.0,      // 额叶
        1 => 0.8,      // 顶叶
        2 => 1.2,      // 枕叶
        _ => 0.9,      // 其他
    };
    
    // 随机噪声 (模拟环境干扰)
    let noise = 8.0 * (rng.gen::<f64>() - 0.5);
    
    // 偶发大幅信号 (模拟眨眼伪影等)
    let artifact = if rng.gen::<f64>() < 0.01 {
        100.0 * (rng.gen::<f64>() - 0.5)
    } else {
        0.0
    };
    
    channel_factor * (alpha + beta + theta) + noise + artifact
}