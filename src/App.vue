<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import TimeDomainCanvas from "./components/TimeDomainCanvas.vue";
import FrequencyDomainCanvas from "./components/FrequencyDomainCanvas.vue";

// 类型定义
interface EegSample {
  timestamp: number;
  channels: number[];
  sample_id: number;
}

interface EegBatch {
  samples: EegSample[];
  batch_id: number;
  channels_count: number;
  sample_rate: number;
}

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
  time_domain: EegBatch;
  frequency_domain: FreqData[];
}

interface FreqData {
  channel_index: number;
  spectrum: number[];
  frequency_bins: number[];
}

// 响应式状态
const isConnected = ref(false);
const isRecording = ref(false);
const isDiscovering = ref(false);
const streamInfo = ref<StreamInfo | null>(null);
const availableStreams = ref<LslStreamInfo[]>([]);
const selectedStream = ref<string>("");
const recordingFilename = ref("");

// 数据状态
const spectrumData = ref<FreqData[]>([]);
let SAMPLE_RATE = 250;
let CHANNELS_COUNT = 0;

// 组件引用
const timeDomainCanvasRef = ref<InstanceType<typeof TimeDomainCanvas> | null>(null);
const frequencyDomainCanvasRef = ref<InstanceType<typeof FrequencyDomainCanvas> | null>(null);

// 通道控制状态
const channelVisibility = ref<boolean[]>([]);
const hoveredChannel = ref<number>(-1);
const selectedChannels = ref<Set<number>>(new Set());

// 性能监控
const backendDataRate = ref(0);
const frontendRenderRate = ref(0);
const timedomainRenderRate = ref(0);
const waveFrontX = ref(80); // 默认波前位置

let lastBackendDataTime = 0;

// 控制函数
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
    
    // 获取流信息
    const info = await invoke('get_stream_info') as StreamInfo | null;
    streamInfo.value = info;
    
    if (info) {
      CHANNELS_COUNT = info.channels_count;
      SAMPLE_RATE = info.sample_rate;
      
      // 初始化通道可见性
      channelVisibility.value = Array(CHANNELS_COUNT).fill(true);
      
      // 初始化画布组件
      timeDomainCanvasRef.value?.initDataBuffer();
      timeDomainCanvasRef.value?.initCanvas();
      
      // 启动渲染循环
      timeDomainCanvasRef.value?.startRenderLoop();
    }
  } catch (error) {
    console.error('Failed to connect to stream:', error);
  }
}

async function disconnectStream() {
  try {
    await invoke('disconnect_stream');
    isConnected.value = false;
    
    // 停止渲染循环
    timeDomainCanvasRef.value?.stopRenderLoop();
    
    streamInfo.value = null;
  } catch (error) {
    console.error('Failed to disconnect stream:', error);
  }
}

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

// 通道控制事件处理
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
    selectedChannels.value = new Set(selectedChannels.value);
  } else {
    selectedChannels.value.clear();
    selectedChannels.value.add(channelIndex);
    selectedChannels.value = new Set(selectedChannels.value);
  }
}

function hoverChannel(channelIndex: number) {
  hoveredChannel.value = channelIndex;
}

// 性能监控事件处理
function updateTimedomainRenderRate(rate: number) {
  timedomainRenderRate.value = rate;
}

function updateFrequencyRate(rate: number) {
  frontendRenderRate.value = rate;
}

function updateWaveFront(position: number) {
  waveFrontX.value = position;
}

// 数据处理
function processFramePayload(payload: FramePayload) {
  const now = Date.now();
  
  // 跟踪后端数据更新率
  if (lastBackendDataTime > 0) {
    const delta = now - lastBackendDataTime;
    backendDataRate.value = 1000 / delta;
  }
  lastBackendDataTime = now;
  
  // 处理时域数据
  const batch = payload.time_domain;
  SAMPLE_RATE = batch.sample_rate;
  CHANNELS_COUNT = batch.channels_count;
  
  // 如果通道数改变，重新初始化
  if (channelVisibility.value.length !== CHANNELS_COUNT) {
    channelVisibility.value = Array(CHANNELS_COUNT).fill(true);
    timeDomainCanvasRef.value?.initDataBuffer();
  }
  
  // 将样本添加到时域画布
  timeDomainCanvasRef.value?.addBatchData(batch.samples);
  
  // 处理频域数据
  if (payload.frequency_domain && payload.frequency_domain.length > 0) {
    spectrumData.value = payload.frequency_domain;
  }
}

// 生命周期
onMounted(async () => {
  // 监听合并的帧数据
  const unlisten = await listen('frame-update', (event) => {
    const payload = event.payload as FramePayload;
    processFramePayload(payload);
  });
  
  onUnmounted(() => {
    unlisten();
  });
});
</script>

<template>
  <div class="eeg-visualizer">
    <!-- 标题栏 -->
    <header class="header">
      <h1>Open CortexArray - EEG可视化系统 V2.5</h1>
      <div class="status-info">
        <span v-if="streamInfo" class="stream-info">
          {{ streamInfo.name }} ({{ streamInfo.stream_type }}) | {{ streamInfo.channels_count }}通道 | {{ streamInfo.sample_rate }}Hz | {{ streamInfo.source_id }}
        </span>
        <span v-else-if="availableStreams.length > 0" class="stream-info">
          发现 {{ availableStreams.length }} 个LSL流
        </span>
        <span :class="['connection-status', isConnected ? 'connected' : 'disconnected']">
          {{ isConnected ? '已连接' : '未连接' }}
        </span>
      </div>
    </header>

    <!-- 控制面板 -->
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
          class="btn btn-warning"
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

    <!-- 主要可视化区域 -->
    <div class="visualization-area">
      <!-- 连接提示 -->
      <div v-if="!isConnected" class="connection-prompt">
        <h3>请先连接到LSL流</h3>
        <p>点击"发现LSL流"按钮开始搜索可用的数据流，然后选择并连接。</p>
      </div>

      <!-- 双画布布局 -->
      <div v-else class="dual-canvas-layout">
        <!-- 时域波形组件 -->
        <TimeDomainCanvas
          ref="timeDomainCanvasRef"
          :channels-count="CHANNELS_COUNT"
          :sample-rate="SAMPLE_RATE"
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

        <!-- 频域分析组件 -->
        <FrequencyDomainCanvas
          ref="frequencyDomainCanvasRef"
          :channels-count="CHANNELS_COUNT"
          :sample-rate="SAMPLE_RATE"
          :channel-visibility="channelVisibility"
          :selected-channels="selectedChannels"
          :spectrum-data="spectrumData"
          :max-freq="60"
          @update-frequency-rate="updateFrequencyRate"
        />
      </div>
    </div>

    <!-- 信息面板 -->
    <div class="info-panel">
      <div class="info-item">
        <strong>渲染模式:</strong> 组件化双画布实时渲染
      </div>
      <div class="info-item">
        <strong>波前位置:</strong> {{ Math.round(waveFrontX) }}px
      </div>
      <div class="info-item">
        <strong>频域更新:</strong> {{ Math.round(frontendRenderRate) }}Hz
      </div>
      <div class="info-item">
        <strong>后端数据率:</strong> {{ Math.round(backendDataRate) }}Hz
      </div>
      <div class="info-item">
        <strong>时域渲染率:</strong> {{ Math.round(timedomainRenderRate) }}帧/秒
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 保留原有样式，但简化布局相关代码 */
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

/* 双画布布局 */
.dual-canvas-layout {
  display: flex;
  gap: 2rem;
  height: 500px;
}

.time-domain-panel {
  flex: 1; /* 占据剩余空间，约66% */
  display: flex;
  flex-direction: column;
  position: relative;
}

.frequency-panel {
  flex: 0 0 33%; /* 固定33%宽度 */
  display: flex;
  flex-direction: column;
  background: #f8f9fa;
  border-radius: 8px;
  padding: 1rem;
  border: 2px solid #e9ecef;
}

.frequency-panel h3,
.time-domain-panel h3 {
  margin: 0 0 1rem 0;
  font-size: 1.1rem;
  font-weight: 600;
  color: #495057;
  text-align: center;
}

/* 时域画布样式 */
.eeg-canvas {
  flex: 1;
  border: 2px solid #e0e0e0;
  border-radius: 8px;
  background: #fafafa;
  display: block;
  box-shadow: inset 0 2px 8px rgba(0, 0, 0, 0.1);
  cursor: default;
}

/* 频域画布样式 */
.spectrum-canvas {
  flex: 1;
  border: 2px solid #dee2e6;
  border-radius: 6px;
  background: #ffffff;
  display: block;
  box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.05);
}

/* 频域图例 */
.frequency-legend {
  display: flex;
  justify-content: space-between;
  margin-top: 0.5rem;
  padding: 0 0.5rem;
}

.freq-range {
  font-size: 0.8rem;
  color: #6c757d;
  font-weight: 500;
}

/* 波前指示器调整 */
.wave-front-indicator {
  position: absolute;
  bottom: 0;
  width: 2px;
  height: 20px;
  background: linear-gradient(to bottom, #ff6b6b, transparent);
  border-radius: 1px;
  box-shadow: 0 0 4px rgba(255, 107, 107, 0.5);
  animation: pulse-glow 1s ease-in-out infinite alternate;
}

@keyframes pulse-glow {
  from { box-shadow: 0 0 4px rgba(255, 107, 107, 0.5); }
  to { box-shadow: 0 0 8px rgba(255, 107, 107, 0.8); }
}

/* 响应式设计 */
@media (max-width: 1200px) {
  .dual-canvas-layout {
    flex-direction: column;
    height: auto;
    gap: 1.5rem;
  }
  
  .time-domain-panel {
    flex: none;
    height: 400px;
  }
  
  .frequency-panel {
    flex: none;
    height: 300px;
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
  
  .dual-canvas-layout {
    gap: 1rem;
  }
  
  .info-panel {
    flex-direction: column;
    gap: 0.5rem;
    margin: 0 1rem 1rem;
  }
}
</style>