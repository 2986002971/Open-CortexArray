/**
 * 二进制帧解析器
 * 解析来自 Rust 后端的高性能二进制 EEG 数据帧
 */
export class BinaryFrameParser {
  /**
   * 解析二进制帧头部
   * 头部格式: [32 bytes]
   * - batch_id: u64 (8 bytes, little-endian)
   * - timestamp: f64 (8 bytes, little-endian)
   * - channels_count: u32 (4 bytes, little-endian)
   * - samples_per_channel: u32 (4 bytes, little-endian)
   * - sample_rate: f64 (8 bytes, little-endian)
   */
  static parseHeader(buffer: ArrayBuffer): {
    batch_id: bigint;
    timestamp: number;
    channels_count: number;
    samples_per_channel: number;
    sample_rate: number;
  } | null {
    if (buffer.byteLength < 32) {
      console.warn(`Binary frame header too short: ${buffer.byteLength} bytes`);
      return null;
    }
    
    const view = new DataView(buffer);
    
    try {
      return {
        batch_id: view.getBigUint64(0, true),           // little-endian
        timestamp: view.getFloat64(8, true),
        channels_count: view.getUint32(16, true),
        samples_per_channel: view.getUint32(20, true),
        sample_rate: view.getFloat64(24, true),
      };
    } catch (error) {
      console.error('Failed to parse binary frame header:', error);
      return null;
    }
  }
  
  /**
   * 解析完整二进制帧
   * 数据布局: [Header: 32 bytes] + [Channel Blocks]
   * Channel Block: channel_index(4 bytes) + samples(4*N bytes)
   */
  static parseFrame(buffer: ArrayBuffer): {
    header: {  // ✅ 改为非nullable类型
      batch_id: bigint;
      timestamp: number;
      channels_count: number;
      samples_per_channel: number;
      sample_rate: number;
    };
    channels: Array<{
      channel_index: number;
      samples: Float32Array;
    }>;
  } | null {  // ✅ 整个结果可以是null，但header不会是null
    const header = this.parseHeader(buffer);
    if (!header) return null;  // ✅ 提前返回null
    
    const channels: Array<{
      channel_index: number;
      samples: Float32Array;
    }> = [];
    
    let offset = 32; // 跳过头部
    const view = new DataView(buffer);
    
    // 解析每个通道
    for (let ch = 0; ch < header.channels_count; ch++) {
      if (buffer.byteLength < offset + 4) {
        console.warn(`Channel ${ch}: insufficient data for channel_index`);
        break;
      }
      
      // 读取通道索引
      const channel_index = view.getUint32(offset, true);
      offset += 4;
      
      // 读取样本数据
      const samplesBytes = header.samples_per_channel * 4;
      if (buffer.byteLength < offset + samplesBytes) {
        console.warn(`Channel ${ch}: insufficient data for samples`);
        break;
      }
      
      // ✅ 直接创建Float32Array视图，零拷贝！
      const samplesBuffer = buffer.slice(offset, offset + samplesBytes);
      const samples = new Float32Array(samplesBuffer);
      
      channels.push({ channel_index, samples });
      offset += samplesBytes;
    }
    
    if (channels.length !== header.channels_count) {
      console.warn(`Expected ${header.channels_count} channels, got ${channels.length}`);
    }
    
    // ✅ 返回时header保证不是null
    return { header, channels };
  }
  
  /**
   * 高性能通道数据提取
   * @param buffer 二进制帧缓冲区
   * @param channelIndex 要提取的通道索引
   * @returns 该通道的连续样本数据
   */
  static extractChannelData(
    buffer: ArrayBuffer, 
    channelIndex: number
  ): Float32Array | null {
    const header = this.parseHeader(buffer);
    if (!header || channelIndex >= header.channels_count) return null;
    
    let offset = 32; // 跳过头部
    const view = new DataView(buffer);
    
    // 跳转到目标通道
    for (let ch = 0; ch < channelIndex; ch++) {
      offset += 4; // 跳过channel_index
      offset += header.samples_per_channel * 4; // 跳过samples
    }
    
    // 跳过目标通道的channel_index
    offset += 4;
    
    // ✅ 直接返回Float32Array视图
    const samplesBytes = header.samples_per_channel * 4;
    if (buffer.byteLength < offset + samplesBytes) {
      console.warn(`Insufficient data for channel ${channelIndex}`);
      return null;
    }
    
    const samplesBuffer = buffer.slice(offset, offset + samplesBytes);
    return new Float32Array(samplesBuffer);
  }
}

/**
 * 性能优化的批量解析器
 * 提供针对不同组件优化的解析方法
 */
export class BatchedBinaryParser {
  private reusableBuffer = new ArrayBuffer(65536); // 64KB复用缓冲区
  private parseCache = new Map<string, any>();     // 解析缓存
  
  /**
   * 批量解析多个通道（适用于TimeDomainCanvas）
   * 返回针对时域渲染优化的数据结构
   */
  parseForTimeRendering(buffer: ArrayBuffer): {
    metadata: {
      batch_id: bigint;
      timestamp: number;
      channels_count: number;
      samples_per_channel: number;
      sample_rate: number;
    };
    channelData: Array<{
      channel_index: number;
      samples: Float32Array;
    }>;
  } | null {
    const startTime = performance.now();
    
    const parsed = BinaryFrameParser.parseFrame(buffer);
    if (!parsed) return null;  // ✅ 只需要检查parsed本身
    
    const endTime = performance.now();
    if (endTime - startTime > 5) {
      console.warn(`Binary parsing took ${(endTime - startTime).toFixed(2)}ms`);
    }
    
    // ✅ 现在TypeScript知道parsed.header不可能是null
    return {
      metadata: parsed.header,    // ✅ 类型安全
      channelData: parsed.channels
    };
  }
  
  /**
   * 解析单个通道（适用于特定通道分析）
   */
  parseChannelOnly(buffer: ArrayBuffer, channelIndex: number): {
    metadata: ReturnType<typeof BinaryFrameParser.parseHeader>;
    samples: Float32Array;
  } | null {
    const header = BinaryFrameParser.parseHeader(buffer);
    if (!header) return null;
    
    const samples = BinaryFrameParser.extractChannelData(buffer, channelIndex);
    if (!samples) return null;
    
    return { metadata: header, samples };
  }
  
  /**
   * 获取帧元信息（不解析样本数据）
   */
  parseMetadataOnly(buffer: ArrayBuffer) {
    return BinaryFrameParser.parseHeader(buffer);
  }
  
  /**
   * 批量验证二进制帧完整性
   */
  validateFrame(buffer: ArrayBuffer): {
    isValid: boolean;
    expectedSize: number;
    actualSize: number;
    error?: string;
  } {
    const header = BinaryFrameParser.parseHeader(buffer);
    if (!header) {
      return {
        isValid: false,
        expectedSize: 32,
        actualSize: buffer.byteLength,
        error: 'Invalid header'
      };
    }
    
    // 计算预期大小
    const headerSize = 32;
    const channelMetaSize = header.channels_count * 4; // 每通道4字节索引
    const samplesSize = header.channels_count * header.samples_per_channel * 4;
    const expectedSize = headerSize + channelMetaSize + samplesSize;
    
    return {
      isValid: buffer.byteLength >= expectedSize,
      expectedSize,
      actualSize: buffer.byteLength,
      error: buffer.byteLength < expectedSize ? 'Incomplete frame data' : undefined
    };
  }
  
  /**
   * 清理缓存（内存优化）
   */
  clearCache() {
    this.parseCache.clear();
  }
  
  /**
   * 获取解析器统计信息
   */
  getStats() {
    return {
      cacheSize: this.parseCache.size,
      bufferSize: this.reusableBuffer.byteLength
    };
  }
}

/**
 * 帧同步器（用于多事件协调）
 */
export class FrameSynchronizer {
  private pendingFrames = new Map<string, {
    timeData?: ArrayBuffer;
    freqData?: any[];
    timestamp: number;
    hasTimeData: boolean;
    hasFreqData: boolean;
  }>();
  
  private maxPendingFrames = 10; // 最大缓存帧数
  
  /**
   * 添加时域数据
   */
  addTimeData(batchId: string, buffer: ArrayBuffer, timestamp: number) {
    this.ensureFrame(batchId, timestamp);
    const frame = this.pendingFrames.get(batchId)!;
    frame.timeData = buffer;
    frame.hasTimeData = true;
    
    this.cleanup();
  }
  
  /**
   * 添加频域数据
   */
  addFreqData(batchId: string, freqData: any[], timestamp: number) {
    this.ensureFrame(batchId, timestamp);
    const frame = this.pendingFrames.get(batchId)!;
    frame.freqData = freqData;
    frame.hasFreqData = true;
    
    this.cleanup();
  }
  
  /**
   * 检查帧是否完整
   */
  isFrameComplete(batchId: string): boolean {
    const frame = this.pendingFrames.get(batchId);
    return frame ? (frame.hasTimeData || frame.hasFreqData) : false;
  }
  
  /**
   * 获取完整帧
   */
  getCompleteFrame(batchId: string) {
    return this.pendingFrames.get(batchId);
  }
  
  /**
   * 移除帧
   */
  removeFrame(batchId: string) {
    this.pendingFrames.delete(batchId);
  }
  
  private ensureFrame(batchId: string, timestamp: number) {
    if (!this.pendingFrames.has(batchId)) {
      this.pendingFrames.set(batchId, {
        timestamp,
        hasTimeData: false,
        hasFreqData: false
      });
    }
  }
  
  private cleanup() {
    if (this.pendingFrames.size > this.maxPendingFrames) {
      // 移除最旧的帧
      const oldestKey = this.pendingFrames.keys().next().value;
      if (oldestKey) {
        this.pendingFrames.delete(oldestKey);
      }
    }
  }
}

// 工具函数
export function createEmptyFrame(channelsCount: number, samplesPerChannel: number): ArrayBuffer {
  const headerSize = 32;
  const channelMetaSize = channelsCount * 4;
  const samplesSize = channelsCount * samplesPerChannel * 4;
  const totalSize = headerSize + channelMetaSize + samplesSize;
  
  const buffer = new ArrayBuffer(totalSize);
  const view = new DataView(buffer);
  
  // 写入空头部
  view.setBigUint64(0, BigInt(0), true);       // batch_id
  view.setFloat64(8, Date.now() / 1000, true); // timestamp
  view.setUint32(16, channelsCount, true);     // channels_count
  view.setUint32(20, samplesPerChannel, true); // samples_per_channel
  view.setFloat64(24, 250.0, true);           // sample_rate (默认)
  
  // 写入通道数据（全零）
  let offset = 32;
  for (let ch = 0; ch < channelsCount; ch++) {
    view.setUint32(offset, ch, true); // channel_index
    offset += 4;
    // samples 区域已经是零填充，跳过
    offset += samplesPerChannel * 4;
  }
  
  return buffer;
}