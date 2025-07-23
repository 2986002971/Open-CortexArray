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
    <div class="top-bar">
      <div class="logo-title">
        <h1>Open CortexArray -- EEG示波器</h1>
      </div>
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
      </div>
    </div>

    <div class="main-canvas-area">
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
    </div>
  </div>
</template>

<!-- 样式保持不变 -->
<style scoped>
/* 🔧 CSS重置 - 移除浏览器默认的margin和padding */
:global(html), :global(body) {
  margin: 0;
  padding: 0;
  height: 100%;
}

.eeg-visualizer {
  display: flex;
  flex-direction: column;
  height: 100vh;
  min-height: 0;
  background: linear-gradient(135deg, #181c24 0%, #23293a 100%);
  color: #eaf6fb;
}

.top-bar {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 2rem;
  background: rgba(24, 28, 36, 0.95);
  box-shadow: 0 2px 10px rgba(0,0,0,0.25);
  border-bottom: 1px solid #23293a;
  min-height: 64px;
  z-index: 10;
}

.logo-title h1 {
  margin: 0;
  font-size: 1.8rem;
  font-weight: 700;
  background: linear-gradient(90deg, #7fdaff, #a18fff, #6fffb0, #ffd6a5);
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

.stream-info {
  font-size: 0.9rem;
  color: #b3c2d6;
  background: rgba(32, 39, 58, 0.6);
  padding: 0.3rem 0.8rem;
  border-radius: 15px;
}

.connection-status.connected {
  background: #213e2e;
  color: #7fffd4;
  border: 1px solid #2ec4b6;
}

.connection-status.disconnected {
  background: #3a2329;
  color: #ff7f7f;
  border: 1px solid #ff4b2b;
}

.control-panel {
  display: flex;
  gap: 1rem;
  align-items: center;
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
  box-shadow: 0 2px 8px rgba(0,0,0,0.15);
}

.btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-primary {
  background: linear-gradient(90deg, #7fdaff 0%, #a18fff 100%);
  color: #181c24;
}

.btn-primary:hover:not(:disabled) {
  transform: translateY(-2px);
  box-shadow: 0 4px 15px rgba(127, 218, 255, 0.4);
}

.btn-success {
  background: linear-gradient(90deg, #6fffb0 0%, #2ec4b6 100%);
  color: #181c24;
}

.btn-danger {
  background: linear-gradient(90deg, #ff7f7f 0%, #ff4b2b 100%);
  color: #181c24;
}

.filename-input {
  padding: 0.6rem 1rem;
  border: 2px solid #2ec4b6;
  border-radius: 8px;
  font-size: 0.9rem;
  background: #23293a;
  color: #eaf6fb;
  transition: border-color 0.3s ease;
}

.stream-select {
  padding: 0.6rem 1rem;
  background-color: #181c24 !important;
  color: #eaf6fb !important;
  border: 2px solid #7fdaff !important;
  box-shadow: 0 2px 8px rgba(127, 218, 255, 0.08);
  font-weight: 600;
  text-shadow: 0 1px 2px #23293a;
  appearance: none; /* 去除系统默认样式，部分浏览器支持 */
}

.stream-select option {
  background: #23293a;
  color: #eaf6fb;
}

.filename-input:focus,
.stream-select:focus,
.btn:focus {
  outline: none;
  background: #23293a;
  border-color: #7fdaff; /* 极光蓝色 */
  box-shadow: 0 0 0 2px #7fdaff44; /* 极光色外发光 */
}

.stream-select:disabled {
  background: #23293a;
  cursor: not-allowed;
  opacity: 0.6;
}

.recording-indicator {
  color: #ff7f7f;
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
  color: #b3c2d6;
  font-style: italic;
}

.visualization-area {
  flex: 1;
  padding: 2rem;
  background: rgba(32, 39, 58, 0.95);
  margin: 0 2rem 2rem;
  border-radius: 12px;
  box-shadow: 0 4px 25px rgba(0, 0, 0, 0.25);
  backdrop-filter: blur(10px);
  min-height: 500px;
}

/* 架构信息展示 */
.connection-prompt {
  text-align: center;
  padding: 3rem 0;
}

.architecture-info {
  margin-top: 2rem;
  padding: 1.5rem;
  background: linear-gradient(135deg, #23293a 0%, #181c24 100%);
  border-radius: 12px;
  border-left: 4px solid #7fdaff;
}

.architecture-info h4 {
  color: #7fdaff;
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
  color: #b3c2d6;
}

.dual-canvas-layout {
  flex: 1;
  display: flex;
  gap: 2rem;
  width: 100%;
  height: 100%;
  align-items: stretch;
}

.dual-canvas-layout > :first-child {
  flex: 2;
  min-width: 0;
  height: 100%;
}
.dual-canvas-layout > :last-child {
  flex: 1;
  min-width: 0;
  height: 100%;
}

/* 响应式设计 */
@media (max-width: 1200px) {
  .dual-canvas-layout {
    flex-direction: column;
    height: auto;
    gap: 1.5rem;
  }
  .dual-canvas-layout > * {
    flex: none;
    min-width: auto;
  }
}

@media (max-width: 768px) {
  .top-bar {
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

.main-canvas-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
</style>