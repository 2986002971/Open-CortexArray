<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

// 响应式状态
const isConnected = ref(false);
const isRecording = ref(false);
const isDiscovering = ref(false);
const streamInfo = ref<StreamInfo | null>(null);
const availableStreams = ref<LslStreamInfo[]>([]);
const selectedStream = ref<string>("");
const recordingFilename = ref("");

// Canvas相关
const canvasRef = ref<HTMLCanvasElement | null>(null);
const spectrumCanvasRef = ref<HTMLCanvasElement | null>(null);
let ctx: CanvasRenderingContext2D | null = null;
let spectrumCtx: CanvasRenderingContext2D | null = null;

// 渲染参数
const CANVAS_WIDTH = 1200;
const CANVAS_HEIGHT = 600;
const SPECTRUM_HEIGHT = 300;
const TIME_WINDOW = 10; // 10秒时间窗口
const CHANNELS_MAX = 8;
let SAMPLE_RATE = 250;
let CHANNELS_COUNT = 8;

// 数据缓冲区 - 使用普通数组避免Vue深度代理
let dataBuffer: number[][] = [];
let bufferSize = 0;
let bufferIndex = 0;
let pixelsPerSample = 0;

// 波前式渲染状态
const waveFrontX = ref(0);
let lastPoints: { x: number; y: number }[] = [];
let renderLoopId = 0;

// 通道显示控制
const channelVisibility = ref<boolean[]>([]);
const channelColors = ['#FF6B6B', '#4ECDC4', '#45B7D1', '#96CEB4', '#FFEAA7', '#DDA0DD', '#98D8E8', '#F7DC6F'];

// FFT Worker相关
let fftWorker: Worker | null = null;
const spectrumData = ref<number[][]>([]);

// Web Worker FFT初始化
function initFFTWorker() {
  fftWorker = new Worker('/fft-worker.js');
  
  fftWorker.onmessage = (e) => {
    const { type, data } = e.data;
    
    switch (type) {
      case 'init-complete':
        console.log('FFT Worker initialized');
        break;
        
      case 'spectrum':
        updateSpectrum(data.channelIndex, data.spectrum);
        break;
        
      case 'error':
        console.error('FFT Worker error:', data.message);
        break;
    }
  };
  
  // 初始化FFT (256点FFT)
  fftWorker.postMessage({
    type: 'init',
    data: { fftSize: 256 }
  });
}

// 初始化数据缓冲区
function initDataBuffer() {
  bufferSize = Math.ceil(SAMPLE_RATE * TIME_WINDOW);
  dataBuffer = Array(CHANNELS_COUNT).fill(null).map(() => new Array(bufferSize).fill(0));
  bufferIndex = 0;
  pixelsPerSample = CANVAS_WIDTH / bufferSize;
  
  // 初始化通道可见性
  channelVisibility.value = Array(CHANNELS_COUNT).fill(true);
  
  // 初始化最后绘制点
  lastPoints = Array(CHANNELS_COUNT).fill(null).map(() => ({ x: 0, y: 0 }));
  
  console.log(`Buffer initialized: ${bufferSize} samples, ${pixelsPerSample} pixels/sample`);
}

// 初始化Canvas
function initCanvas() {
  if (!canvasRef.value || !spectrumCanvasRef.value) return;
  
  const canvas = canvasRef.value;
  const spectrumCanvas = spectrumCanvasRef.value;
  
  canvas.width = CANVAS_WIDTH;
  canvas.height = CANVAS_HEIGHT;
  spectrumCanvas.width = CANVAS_WIDTH;
  spectrumCanvas.height = SPECTRUM_HEIGHT;
  
  ctx = canvas.getContext('2d');
  spectrumCtx = spectrumCanvas.getContext('2d');
  
  if (ctx) {
    ctx.strokeStyle = '#333';
    ctx.lineWidth = 1;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    
    // 绘制背景网格
    drawGrid();
  }
  
  if (spectrumCtx) {
    spectrumCtx.fillStyle = '#f0f0f0';
    spectrumCtx.fillRect(0, 0, CANVAS_WIDTH, SPECTRUM_HEIGHT);
  }
}

// 绘制背景网格
function drawGrid() {
  if (!ctx) return;
  
  ctx.save();
  ctx.strokeStyle = '#e0e0e0';
  ctx.lineWidth = 0.5;
  
  // 垂直网格线 (时间)
  const timeStep = CANVAS_WIDTH / 10; // 10个时间分割
  for (let x = 0; x <= CANVAS_WIDTH; x += timeStep) {
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, CANVAS_HEIGHT);
    ctx.stroke();
  }
  
  // 水平网格线 (通道分隔)
  const channelHeight = CANVAS_HEIGHT / CHANNELS_COUNT;
  for (let y = 0; y <= CANVAS_HEIGHT; y += channelHeight) {
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(CANVAS_WIDTH, y);
    ctx.stroke();
  }
  
  ctx.restore();
}

// 处理接收到的EEG数据
function processEegBatch(batch: EegBatch) {
  SAMPLE_RATE = batch.sample_rate;
  CHANNELS_COUNT = batch.channels_count;
  
  // 如果通道数改变，重新初始化
  if (dataBuffer.length !== CHANNELS_COUNT) {
    initDataBuffer();
  }
  
  // 将样本添加到缓冲区
  for (const sample of batch.samples) {
    for (let ch = 0; ch < CHANNELS_COUNT; ch++) {
      if (ch < sample.channels.length) {
        dataBuffer[ch][bufferIndex] = sample.channels[ch];
      }
    }
    
    bufferIndex = (bufferIndex + 1) % bufferSize;
  }
  
  // 触发FFT计算（每隔一定样本数）
  if (batch.batch_id % 10 === 0 && fftWorker) {
    for (let ch = 0; ch < CHANNELS_COUNT; ch++) {
      if (channelVisibility.value[ch]) {
        // 获取最近256个样本用于FFT
        const fftSamples = [];
        for (let i = 0; i < 256; i++) {
          const idx = (bufferIndex - 256 + i + bufferSize) % bufferSize;
          fftSamples.push(dataBuffer[ch][idx]);
        }
        
        fftWorker.postMessage({
          type: 'compute',
          data: {
            channelData: fftSamples,
            channelIndex: ch,
            timestamp: Date.now()
          }
        });
      }
    }
  }
}

// 波前式渲染主循环
function renderLoop() {
  if (!ctx) return;
  
  const pointsToProcess = 4; // 每帧处理的点数
  
  // 1. 擦除波前区域
  const clearWidth = pointsToProcess * pixelsPerSample + 10;
  ctx.clearRect(waveFrontX.value, 0, clearWidth, CANVAS_HEIGHT);
  
  // 重绘背景网格（仅在擦除区域）
  ctx.save();
  ctx.strokeStyle = '#e0e0e0';
  ctx.lineWidth = 0.5;
  ctx.beginPath();
  
  // 垂直网格线
  const timeStep = CANVAS_WIDTH / 10;
  for (let x = 0; x <= CANVAS_WIDTH; x += timeStep) {
    if (x >= waveFrontX.value && x <= waveFrontX.value + clearWidth) {
      ctx.moveTo(x, 0);
      ctx.lineTo(x, CANVAS_HEIGHT);
    }
  }
  
  // 水平网格线
  const channelHeight = CANVAS_HEIGHT / CHANNELS_COUNT;
  for (let y = 0; y <= CANVAS_HEIGHT; y += channelHeight) {
    ctx.moveTo(waveFrontX.value, y);
    ctx.lineTo(waveFrontX.value + clearWidth, y);
  }
  ctx.stroke();
  ctx.restore();
  
  // 2. 绘制新的波形数据
  for (let ch = 0; ch < CHANNELS_COUNT; ch++) {
    if (!channelVisibility.value[ch]) continue;
    
    ctx.strokeStyle = channelColors[ch % channelColors.length];
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    
    // 从上一帧的最后点开始
    ctx.moveTo(lastPoints[ch].x, lastPoints[ch].y);
    
    // 绘制新的数据点
    for (let i = 0; i < pointsToProcess; i++) {
      const dataIndex = (bufferIndex - pointsToProcess + i + bufferSize) % bufferSize;
      const x = waveFrontX.value + i * pixelsPerSample;
      
      // 计算Y坐标（每个通道占用canvas高度的1/CHANNELS_COUNT）
      const channelHeight = CANVAS_HEIGHT / CHANNELS_COUNT;
      const channelCenter = channelHeight * (ch + 0.5);
      const amplitude = dataBuffer[ch][dataIndex];
      const scale = channelHeight * 0.4 / 100; // 假设±100μV的范围
      const y = channelCenter - amplitude * scale;
      
      ctx.lineTo(x, y);
      
      // 更新最后点位置
      if (i === pointsToProcess - 1) {
        lastPoints[ch] = { x, y };
      }
    }
    
    ctx.stroke();
  }
  
  // 3. 更新波前位置
  waveFrontX.value += pointsToProcess * pixelsPerSample;
  if (waveFrontX.value >= CANVAS_WIDTH) {
    waveFrontX.value = 0;
    // 重置最后点的X坐标
    lastPoints.forEach(point => {
      point.x = 0;
    });
  }
  
  renderLoopId = requestAnimationFrame(renderLoop);
}

// 更新频谱显示
function updateSpectrum(channelIndex: number, spectrum: number[]) {
  if (!spectrumData.value[channelIndex]) {
    spectrumData.value[channelIndex] = [];
  }
  spectrumData.value[channelIndex] = spectrum.slice(0, 50); // 只显示前50个频率bin
  
  // 重绘频谱图
  drawSpectrum();
}

function drawSpectrum() {
  if (!spectrumCtx) return;
  
  spectrumCtx.fillStyle = '#f0f0f0';
  spectrumCtx.fillRect(0, 0, CANVAS_WIDTH, SPECTRUM_HEIGHT);
  
  const barWidth = CANVAS_WIDTH / 50; // 50个频率bin
  
  for (let ch = 0; ch < CHANNELS_COUNT; ch++) {
    if (!channelVisibility.value[ch] || !spectrumData.value[ch]) continue;
    
    spectrumCtx.fillStyle = channelColors[ch % channelColors.length];
    
    const spectrum = spectrumData.value[ch];
    for (let i = 0; i < spectrum.length; i++) {
      const magnitude = Math.min(spectrum[i] / 100, 1); // 归一化
      const barHeight = magnitude * SPECTRUM_HEIGHT;
      const x = i * barWidth;
      const y = SPECTRUM_HEIGHT - barHeight;
      
      spectrumCtx.fillRect(x, y, barWidth - 1, barHeight);
    }
  }
}

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
      initDataBuffer();
      initCanvas();
      renderLoop();
    }
  } catch (error) {
    console.error('Failed to connect to stream:', error);
  }
}

async function disconnectStream() {
  try {
    await invoke('disconnect_stream');
    isConnected.value = false;
    
    if (renderLoopId) {
      cancelAnimationFrame(renderLoopId);
      renderLoopId = 0;
    }
    
    // 清空画布
    if (ctx) {
      ctx.clearRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
      drawGrid();
    }
    
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

function toggleChannel(channelIndex: number) {
  channelVisibility.value[channelIndex] = !channelVisibility.value[channelIndex];
}

// 生命周期
onMounted(async () => {
  await nextTick();
  initDataBuffer();
  initCanvas();
  initFFTWorker();
  
  // 监听EEG数据
  const unlisten = await listen('eeg-data', (event) => {
    const batch = event.payload as EegBatch;
    processEegBatch(batch);
  });
  
  // 在组件卸载时清理
  onUnmounted(() => {
    unlisten();
    if (renderLoopId) {
      cancelAnimationFrame(renderLoopId);
    }
    if (fftWorker) {
      fftWorker.terminate();
    }
  });
});
</script>

<template>
  <div class="eeg-visualizer">
    <!-- 标题栏 -->
    <header class="header">
      <h1>Open CortexArray - EEG可视化系统</h1>
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

      <!-- 通道控制 -->
      <div class="channel-controls">
        <span class="control-label">通道显示:</span>
        <div class="channel-buttons">
          <button 
            v-for="(visible, index) in channelVisibility" 
            :key="index"
            @click="toggleChannel(index)"
            :class="['channel-btn', visible ? 'active' : 'inactive']"
            :style="{ backgroundColor: visible ? channelColors[index % channelColors.length] : '#ccc' }"
          >
            CH{{ index + 1 }}
          </button>
        </div>
      </div>
    </div>

    <!-- 主要可视化区域 -->
    <div class="visualization-area">
      <!-- 实时波形图 -->
      <div class="waveform-container">
        <h3>实时EEG波形 (波前式渲染)</h3>
        <canvas 
          ref="canvasRef" 
          class="eeg-canvas"
          :style="{ width: '100%', height: '400px' }"
        ></canvas>
        <div class="wave-front-indicator" :style="{ left: (waveFrontX / 1200 * 100) + '%' }"></div>
      </div>

      <!-- 实时频谱图 -->
      <div class="spectrum-container">
        <h3>实时频谱分析</h3>
        <canvas 
          ref="spectrumCanvasRef" 
          class="spectrum-canvas"
          :style="{ width: '100%', height: '200px' }"
        ></canvas>
      </div>
    </div>

    <!-- 信息面板 -->
    <div class="info-panel">
      <div class="info-item">
        <strong>渲染模式:</strong> 波前式实时渲染 (~60FPS)
      </div>
      <div class="info-item">
        <strong>时间窗口:</strong> {{ TIME_WINDOW }}秒
      </div>
      <div class="info-item">
        <strong>缓冲区大小:</strong> {{ bufferSize }}样本
      </div>
      <div class="info-item">
        <strong>波前位置:</strong> {{ Math.round(waveFrontX) }}px / {{ CANVAS_WIDTH }}px
      </div>
    </div>
  </div>
</template>

<style scoped>
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

.channel-controls {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.control-label {
  font-weight: 600;
  color: #555;
}

.channel-buttons {
  display: flex;
  gap: 0.5rem;
}

.channel-btn {
  padding: 0.4rem 0.8rem;
  border: none;
  border-radius: 6px;
  color: white;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.3s ease;
  font-size: 0.8rem;
}

.channel-btn:hover {
  transform: scale(1.05);
}

.channel-btn.inactive {
  opacity: 0.4;
}

.visualization-area {
  padding: 2rem;
  display: flex;
  flex-direction: column;
  gap: 2rem;
}

.waveform-container, .spectrum-container {
  background: rgba(255, 255, 255, 0.95);
  border-radius: 12px;
  padding: 1.5rem;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.1);
  backdrop-filter: blur(10px);
  position: relative;
}

.waveform-container h3, .spectrum-container h3 {
  margin: 0 0 1rem 0;
  color: #333;
  font-weight: 600;
}

.eeg-canvas, .spectrum-canvas {
  border: 2px solid #e0e0e0;
  border-radius: 8px;
  background: #fafafa;
  display: block;
  box-shadow: inset 0 2px 8px rgba(0, 0, 0, 0.1);
}

.wave-front-indicator {
  position: absolute;
  top: 3.5rem;
  width: 3px;
  height: calc(100% - 5rem);
  background: linear-gradient(to bottom, #ff416c, #ff4b2b);
  border-radius: 2px;
  pointer-events: none;
  z-index: 10;
  box-shadow: 0 0 10px rgba(255, 65, 108, 0.6);
  transition: left 0.1s ease-out;
}

.info-panel {
  background: rgba(255, 255, 255, 0.9);
  margin: 0 2rem 2rem 2rem;
  padding: 1rem 1.5rem;
  border-radius: 8px;
  display: flex;
  gap: 2rem;
  justify-content: space-around;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.1);
}

.info-item {
  font-size: 0.9rem;
  color: #666;
}

.info-item strong {
  color: #333;
}

/* 响应式设计 */
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
  
  .info-panel {
    flex-direction: column;
    gap: 0.5rem;
  }
}
</style>