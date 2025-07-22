use lsl;

fn main() {
    println!("🔍 测试 liblsl-rust 的 resolve_streams 能力");
    let timeout = 5.0;
    match lsl::resolve_streams(timeout) {
        Ok(streams) => {
            println!("发现 {} 个LSL流：", streams.len());
            for (i, stream) in streams.iter().enumerate() {
                println!("--- 流 #{} ---", i + 1);
                println!("名称 (name): {}", stream.stream_name());
                println!("类型 (type): {}", stream.stream_type());
                println!("源ID (source_id): {}", stream.source_id());
                println!("通道数: {}", stream.channel_count());
                println!("采样率: {}", stream.nominal_srate());
                println!("主机: {}", stream.hostname());
            }
            if streams.is_empty() {
                println!("⚠️  没有发现任何LSL流，请检查网络和LSL服务端。");
            }
        }
        Err(e) => {
            println!("❌ resolve_streams 调用失败: {:?}", e);
        }
    }
}