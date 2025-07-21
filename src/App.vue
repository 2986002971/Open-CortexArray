<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import TimeDomainCanvas from "./components/TimeDomainCanvas.vue";
import FrequencyDomainCanvas from "./components/FrequencyDomainCanvas.vue";

// ✅ 保留必要的类型定义
interface StreamInfo {
  name: string;
  stream_type: string;
  channels_count: number;
  sample_rate: number;
  is_connected: boolean;
  source_id: string;
}

interface LslStreamInfo {
  name: string;
  stream_type: string;
  channels_count: number;
  sample_rate: number;
  source_id: string;
  hostname: string;
}

interface FramePayload {
  time_domain: {
    samples: any[];
    batch_id: number;
    channels_count: number;
    sample_rate: number;
  };
  frequency_domain: any[];
}

// ✅ 连接和录制状态（核心职责）
const isConnected = ref(false);
const isRecording = ref(false);
const isDiscovering = ref(false);
const streamInfo = ref<StreamInfo | null>(null);
const availableStreams = ref<LslStreamInfo[]>([]);
const selectedStream = ref<string>("");
const recordingFilename = ref("");

// ✅ UI交互状态（App需要管理）
const channelVisibility = ref<boolean[]>([]);
const hoveredChannel = ref<number>(-1);
const selectedChannels = ref<Set<number>>(new Set());

// ✅ 动态获取的流参数
let SAMPLE_RATE = 250;
let CHANNELS_COUNT = 0;

// ✅ 性能监控（App层面的统计）
const backendDataRate = ref(0);
const timedomainRenderRate = ref(0);
const frequencyRenderRate = ref(0);
const waveFrontPosition = ref(0);

let lastBackendDataTime = 0;

// 组件引用
const timeDomainCanvasRef = ref<InstanceType<typeof TimeDomainCanvas> | null>(null);
const frequencyDomainCanvasRef = ref<InstanceType<typeof FrequencyDomainCanvas> | null>(null);

// ✅ LSL连接控制函数（保留）
async function discoverStreams() {
  try {
    isDiscovering.value = true;
    const streams = await invoke('discover_lsl_streams') as LslStreamInfo[];
    availableStreams.value = streams;
    
    if (streams.length > 0) {
      selectedStream.value = streams[0].name;
    }
  } catch (error) {
    console.error('Failed to discover LSL streams:', error);
  } finally {
    isDiscovering.value = false;
  }
}

async function connectToSelectedStream() {
  if (!selectedStream.value) {
    console.error('No stream selected');
    return;
  }
  
  try {
    await invoke('connect_to_stream', { streamName: selectedStream.value });
    isConnected.value = true;
    
    // ✅ 获取流信息并更新全局状态
    const info = await invoke('get_stream_info') as StreamInfo | null;
    streamInfo.value = info;
    
    if (info) {
      CHANNELS_COUNT = info.channels_count;
      SAMPLE_RATE = info.sample_rate;
      
      // ✅ 初始化通道可见性
      channelVisibility.value = Array(CHANNELS_COUNT).fill(true);
      
      console.log(`🔌 已连接到流: ${info.name}, ${CHANNELS_COUNT}通道, ${SAMPLE_RATE}Hz`);
      console.log('📡 画布将独立监听二进制/频域事件');
    }
  } catch (error) {
    console.error('Failed to connect to stream:', error);
  }
}

async function disconnectStream() {
  try {
    await invoke('disconnect_stream');
    isConnected.value = false;
    streamInfo.value = null;
    
    // ✅ 重置状态
    CHANNELS_COUNT = 0;
    SAMPLE_RATE = 250;
    channelVisibility.value = [];
    
    console.log('🔌 已断开连接');
  } catch (error) {
    console.error('Failed to disconnect stream:', error);
  }
}

// ✅ 录制控制函数（保留）
async function startRecording() {
  if (!recordingFilename.value) {
    recordingFilename.value = `eeg_recording_${new Date().toISOString().replace(/[:.]/g, '-')}.edf`;
  }
  
  try {
    await invoke('start_recording', { filename: recordingFilename.value });
    isRecording.value = true;
  } catch (error) {
    console.error('Failed to start recording:', error);
  }
}

async function stopRecording() {
  try {
    await invoke('stop_recording');
    isRecording.value = false;
  } catch (error) {
    console.error('Failed to stop recording:', error);
  }
}

// ✅ UI交互控制函数（保留）
function toggleChannel(channelIndex: number) {
  channelVisibility.value[channelIndex] = !channelVisibility.value[channelIndex];
}

function selectChannel(channelIndex: number, isMultiSelect: boolean) {
  if (isMultiSelect) {
    if (selectedChannels.value.has(channelIndex)) {
      selectedChannels.value.delete(channelIndex);
    } else {
      selectedChannels.value.add(channelIndex);
    }
  } else {
    selectedChannels.value.clear();
    selectedChannels.value.add(channelIndex);
  }
  selectedChannels.value = new Set(selectedChannels.value);
}

function hoverChannel(channelIndex: number) {
  hoveredChannel.value = channelIndex;
}

// ✅ 性能监控回调（来自画布组件）
function updateTimedomainRenderRate(rate: number) {
  timedomainRenderRate.value = rate;
}

function updateFrequencyRate(rate: number) {
  frequencyRenderRate.value = rate;
}

function updateWaveFront(position: number) {
  waveFrontPosition.value = position * 100;
}

// ✅ 简化的数据监控（仅用于性能统计）
function monitorBackendData(payload: FramePayload) {
  const now = Date.now();
  
  // 跟踪后端数据率
  if (lastBackendDataTime > 0) {
    const delta = now - lastBackendDataTime;
    backendDataRate.value = 1000 / delta;
  }
  lastBackendDataTime = now;
  
  // ✅ 检测流参数变化（可能影响UI）
  const batch = payload.time_domain;
  if (SAMPLE_RATE !== batch.sample_rate) {
    console.log(`📊 采样率变化: ${SAMPLE_RATE} → ${batch.sample_rate}`);
    SAMPLE_RATE = batch.sample_rate;
    if (streamInfo.value) {
      streamInfo.value.sample_rate = batch.sample_rate;
    }
  }
  
  if (CHANNELS_COUNT !== batch.channels_count) {
    console.log(`📊 通道数变化: ${CHANNELS_COUNT} → ${batch.channels_count}`);
    CHANNELS_COUNT = batch.channels_count;
    if (streamInfo.value) {
      streamInfo.value.channels_count = batch.channels_count;
    }
    // 重新初始化通道可见性
    channelVisibility.value = Array(CHANNELS_COUNT).fill(true);
  }
}

// ✅ 生命周期（保持监听但职责简化）
onMounted(async () => {
  // ✅ App层面监听frame-update主要用于：
  // 1. 性能统计
  // 2. 流参数变化检测
  // 3. 整体状态监控
  const unlisten = await listen('frame-update', (event) => {
    const payload = event.payload as FramePayload;
    monitorBackendData(payload);
  });
  
  onUnmounted(() => {
    unlisten();
  });
  
  console.log('🚀 App.vue已初始化 - 混合架构：连接管理 + 画布独立监听');
});
</script>

<template>
  <div class="eeg-visualizer">
    <!-- ✅ 标题栏保持不变 -->
    <header class="header">
      <h1>Open CortexArray - EEG可视化系统 V2.5 (混合架构)</h1>
      <div class="status-info">
        <span v-if="streamInfo" class="stream-info">
          {{ streamInfo.name }} | {{ streamInfo.channels_count }}通道 | {{ streamInfo.sample_rate }}Hz
        </span>
        <span :class="['connection-status', isConnected ? 'connected' : 'disconnected']">
          {{ isConnected ? '已连接' : '未连接' }}
        </span>
      </div>
    </header>

    <!-- ✅ 控制面板完全保留 -->
    <div class="control-panel">
      <!-- LSL流发现和连接 -->
      <div class="control-group">
        <button 
          @click="discoverStreams" 
          :disabled="isDiscovering || isConnected"
          class="btn btn-primary"
        >
          {{ isDiscovering ? '搜索中...' : '发现LSL流' }}
        </button>
        
        <select 
          v-model="selectedStream" 
          :disabled="isConnected || availableStreams.length === 0"
          class="stream-select"
        >
          <option v-if="availableStreams.length === 0" value="">无可用流</option>
          <option 
            v-for="stream in availableStreams" 
            :key="stream.source_id" 
            :value="stream.name"
          >
            {{ stream.name }} ({{ stream.channels_count }}ch, {{ stream.sample_rate }}Hz)
          </option>
        </select>
        
        <button 
          @click="connectToSelectedStream" 
          :disabled="!selectedStream || isConnected"
          class="btn btn-success"
        >
          连接到流
        </button>
        
        <button 
          @click="disconnectStream" 
          :disabled="!isConnected"
          class="btn btn-danger"
        >
          断开连接
        </button>
      </div>

      <!-- 录制控制 -->
      <div class="control-group">
        <input 
          v-model="recordingFilename" 
          placeholder="录制文件名.edf"
          :disabled="isRecording"
          class="filename-input"
        />
        <button 
          @click="startRecording" 
          :disabled="!isConnected || isRecording"
          class="btn btn-success"
        >
          开始录制
        </button>
        <button 
          @click="stopRecording" 
          :disabled="!isRecording"
          class="btn btn-danger"
        >
          停止录制
        </button>
        <span v-if="isRecording" class="recording-indicator">🔴 录制中</span>
      </div>

      <!-- 通道操作提示 -->
      <div v-if="isConnected && CHANNELS_COUNT > 0" class="channel-help">
        <span class="control-label">通道操作:</span>
        <span class="help-text">点击左侧标签切换显示 | Ctrl+点击多选高亮</span>
      </div>
    </div>

    <!-- 可视化区域 -->
    <div class="visualization-area">
      <div v-if="!isConnected" class="connection-prompt">
        <h3>请先连接到LSL流</h3>
        <div class="architecture-info">
          <h4>🚀 混合架构特性</h4>
          <ul>
            <li>✅ App.vue：连接管理 + UI交互 + 性能监控</li>
            <li>✅ TimeDomainCanvas：独立监听 binary-frame-update</li>
            <li>✅ FrequencyDomainCanvas：独立监听 frequency-update</li>
            <li>✅ 最佳的职责分离和性能优化</li>
          </ul>
        </div>
      </div>

      <div v-else class="dual-canvas-layout">
        <!-- ✅ 传递streamInfo而不是单独的参数 -->
        <TimeDomainCanvas
          ref="timeDomainCanvasRef"
          :stream-info="streamInfo"
          :channel-visibility="channelVisibility"
          :selected-channels="selectedChannels"
          :hovered-channel="hoveredChannel"
          :is-connected="isConnected"
          @toggle-channel="toggleChannel"
          @select-channel="selectChannel"
          @hover-channel="hoverChannel"
          @update-render-rate="updateTimedomainRenderRate"
          @update-wave-front="updateWaveFront"
        />

        <FrequencyDomainCanvas
          ref="frequencyDomainCanvasRef"
          :stream-info="streamInfo"
          :channel-visibility="channelVisibility"
          :selected-channels="selectedChannels"
          :max-freq="60"
          @update-frequency-rate="updateFrequencyRate"
        />
      </div>
    </div>

    <!-- ✅ 信息面板保留性能监控 -->
    <div class="info-panel">
      <div class="info-item">
        <strong>架构:</strong> 混合模式（连接管理 + 独立画布监听） 🎯
      </div>
      <div class="info-item">
        <strong>时域渲染率:</strong> {{ Math.round(timedomainRenderRate) }}Hz
      </div>
      <div class="info-item">
        <strong>频域更新率:</strong> {{ Math.round(frequencyRenderRate) }}Hz
      </div>
    </div>
  </div>
</template>

<!-- 样式保持不变 -->
<style scoped>
/* 基础样式保持不变... */
.eeg-visualizer {
  font-family: 'Inter', 'Arial', sans-serif;
  max-width: 100vw;
  margin: 0;
  padding: 0;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  min-height: 100vh;
  color: #333;
}

.header {
  background: rgba(255, 255, 255, 0.95);
  backdrop-filter: blur(10px);
  padding: 1rem 2rem;
  box-shadow: 0 2px 20px rgba(0, 0, 0, 0.1);
  display: flex;
  justify-content: space-between;
  align-items: center;
  border-bottom: 1px solid rgba(255, 255, 255, 0.2);
}

.header h1 {
  margin: 0;
  font-size: 1.8rem;
  font-weight: 700;
  background: linear-gradient(45deg, #667eea, #764ba2);
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

.status-info {
  display: flex;
  gap: 1rem;
  align-items: center;
}

.stream-info {
  font-size: 0.9rem;
  color: #666;
  background: rgba(0, 0, 0, 0.05);
  padding: 0.3rem 0.8rem;
  border-radius: 15px;
}

.connection-status {
  padding: 0.4rem 1rem;
  border-radius: 20px;
  font-weight: 600;
  font-size: 0.85rem;
}

.connection-status.connected {
  background: #d4edda;
  color: #155724;
  border: 1px solid #c3e6cb;
}

.connection-status.disconnected {
  background: #f8d7da;
  color: #721c24;
  border: 1px solid #f5c6cb;
}

.control-panel {
  background: rgba(255, 255, 255, 0.9);
  padding: 1.5rem 2rem;
  display: flex;
  gap: 2rem;
  align-items: center;
  flex-wrap: wrap;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.1);
}

.control-group {
  display: flex;
  gap: 0.8rem;
  align-items: center;
}

.btn {
  padding: 0.6rem 1.2rem;
  border: none;
  border-radius: 8px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.3s ease;
  font-size: 0.9rem;
}

.btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-primary {
  background: linear-gradient(45deg, #667eea, #764ba2);
  color: white;
}

.btn-primary:hover:not(:disabled) {
  transform: translateY(-2px);
  box-shadow: 0 4px 15px rgba(102, 126, 234, 0.4);
}

.btn-success {
  background: linear-gradient(45deg, #56ab2f, #a8e6cf);
  color: white;
}

.btn-danger {
  background: linear-gradient(45deg, #ff416c, #ff4b2b);
  color: white;
}

.filename-input {
  padding: 0.6rem 1rem;
  border: 2px solid #e0e0e0;
  border-radius: 8px;
  font-size: 0.9rem;
  transition: border-color 0.3s ease;
}

.filename-input:focus {
  outline: none;
  border-color: #667eea;
}

.stream-select {
  padding: 0.6rem 1rem;
  border: 2px solid #e0e0e0;
  border-radius: 8px;
  font-size: 0.9rem;
  background: white;
  cursor: pointer;
  transition: border-color 0.3s ease;
  min-width: 200px;
}

.stream-select:focus {
  outline: none;
  border-color: #667eea;
}

.stream-select:disabled {
  background: #f5f5f5;
  cursor: not-allowed;
  opacity: 0.6;
}

.recording-indicator {
  color: #dc3545;
  font-weight: 600;
  animation: pulse 1.5s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

.channel-help {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.help-text {
  font-size: 0.8rem;
  color: #666;
  font-style: italic;
}

.visualization-area {
  padding: 2rem;
  background: rgba(255, 255, 255, 0.95);
  margin: 0 2rem 2rem;
  border-radius: 12px;
  box-shadow: 0 4px 25px rgba(0, 0, 0, 0.1);
  backdrop-filter: blur(10px);
  min-height: 500px;
}

/* ✅ 新增：架构信息展示 */
.connection-prompt {
  text-align: center;
  padding: 3rem 0;
}

.architecture-info {
  margin-top: 2rem;
  padding: 1.5rem;
  background: linear-gradient(135deg, #f8f9fa 0%, #e9ecef 100%);
  border-radius: 12px;
  border-left: 4px solid #667eea;
}

.architecture-info h4 {
  color: #495057;
  margin-bottom: 1rem;
}

.architecture-info ul {
  text-align: left;
  display: inline-block;
  margin: 0;
  padding-left: 1.5rem;
}

.architecture-info li {
  margin: 0.5rem 0;
  color: #666;
}

.dual-canvas-layout {
  display: flex;
  gap: 2rem;
  height: 500px;
  align-items: stretch; /* ✅ 确保子元素完全拉伸 */
  /* ✅ 确保是真正的flex布局 */
  width: 100%;
}

/* ✅ 修正：确保两个组件都参与flex布局 */
.dual-canvas-layout > * {
  display: flex;
  flex-direction: column;
  /* ✅ 移除任何可能的绝对定位 */
  position: relative;
}

/* 时域组件占用更大空间 */
.dual-canvas-layout > :first-child {
  flex: 2; /* 时域占2/3 */
  min-width: 0; /* ✅ 防止flex收缩问题 */
}

/* 频域组件占用较小空间但高度对齐 */
.dual-canvas-layout > :last-child {
  flex: 1; /* 频域占1/3 */
  min-width: 0; /* ✅ 防止flex收缩问题 */
}


/* 响应式设计 */
@media (max-width: 1200px) {
  .dual-canvas-layout {
    flex-direction: column;
    height: auto;
    gap: 1.5rem;
  }
  
  /* 在垂直布局时重置flex */
  .dual-canvas-layout > * {
    flex: none;
    min-width: auto;
  }
}

@media (max-width: 768px) {
  .header {
    flex-direction: column;
    gap: 1rem;
    text-align: center;
  }
  
  .control-panel {
    flex-direction: column;
    align-items: stretch;
  }
  
  .visualization-area {
    margin: 0 1rem 1rem;
    padding: 1rem;
  }
  
  .info-panel {
    flex-direction: column;
    gap: 0.5rem;
    margin: 0 1rem 1rem;
  }
}
</style>