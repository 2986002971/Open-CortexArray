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

// 更新类型定义 - 添加合并数据包接口
interface FramePayload {
  time_domain: EegBatch;
  frequency_domain: FreqData[];
}

interface FreqData {
  channel_index: number;
  spectrum: number[]; // 频域幅度数据
  frequency_bins: number[]; // 对应的频率值
}

// 响应式状态
const isConnected = ref(false);
const isRecording = ref(false);
const isDiscovering = ref(false);
const streamInfo = ref<StreamInfo | null>(null);
const availableStreams = ref<LslStreamInfo[]>([]);
const selectedStream = ref<string>("");
const recordingFilename = ref("");

// 添加频域数据状态 ⭐
const spectrumData = ref<FreqData[]>([]);

// Canvas相关
const canvasRef = ref<HTMLCanvasElement | null>(null);
const spectrumCanvasRef = ref<HTMLCanvasElement | null>(null);
let ctx: CanvasRenderingContext2D | null = null;
let spectrumCtx: CanvasRenderingContext2D | null = null;

// 渲染参数
const CANVAS_WIDTH = 1000;
const SPECTRUM_WIDTH = 400;
const CANVAS_HEIGHT = 600;
const SPECTRUM_HEIGHT = 600;
const CHANNEL_LABEL_WIDTH = 80;
const WAVEFORM_WIDTH = CANVAS_WIDTH - CHANNEL_LABEL_WIDTH;
const TIME_WINDOW = 10;
let SAMPLE_RATE = 250;
let CHANNELS_COUNT = 0;

// 添加缺失的变量声明 ⭐
let bufferSize = 0;
let pixelsPerSample = 0;

// 标准环形缓冲区实现（非响应式）
class RingBuffer {
  private buffer: Float32Array[];
  private head: number = 0;
  private readonly capacity: number;

  constructor(channels: number, capacity: number) {
    this.capacity = capacity;
    this.buffer = Array(channels).fill(null).map(() => new Float32Array(capacity));
  }

  addSample(channelData: number[]) {
    for (let ch = 0; ch < this.buffer.length && ch < channelData.length; ch++) {
      this.buffer[ch][this.head] = channelData[ch] || 0;
    }
    this.head = (this.head + 1) % this.capacity;
  }

  addBatch(samples: EegSample[]) {
    for (const sample of samples) {
      this.addSample(sample.channels);
    }
  }

  getChannelSamples(channel: number, count: number): Float32Array {
    if (channel >= this.buffer.length) {
      return new Float32Array(count);
    }
    
    const result = new Float32Array(count);
    for (let i = 0; i < count; i++) {
      const idx = (this.head - count + i + this.capacity) % this.capacity;
      result[i] = this.buffer[channel][idx];
    }
    return result;
  }

  getCurrentIndex(): number {
    return this.head;
  }

  getCapacity(): number {
    return this.capacity;
  }
}

// 数据缓冲区声明
let ringBuffer: RingBuffer | null = null;

// 波前式渲染状态
const waveFrontX = ref(0);
let lastPoints: { x: number; y: number }[] = [];
let renderLoopId = 0;

// 通道显示控制
const channelVisibility = ref<boolean[]>([]);
const channelColors = ['#FF6B6B', '#4ECDC4', '#45B7D1', '#96CEB4', '#FFEAA7', '#DDA0DD', '#98D8E8', '#F7DC6F'];
const hoveredChannel = ref<number>(-1);
const selectedChannels = ref<Set<number>>(new Set());

// 频域更新相关变量
const frequencyUpdateRate = ref(0);
let lastFrequencyUpdate = 0;
let lastFreqRenderTime = 0;
const FREQ_RENDER_INTERVAL = 1000 / 30; // 30Hz限制

// 添加独立的性能监控
const backendDataRate = ref(0);
const frontendRenderRate = ref(0);
const timedomainRenderRate = ref(0);

let lastBackendDataTime = 0;
let lastTimedomainRender = 0;
let timedomainFrameCount = 0;

// 初始化数据缓冲区
function initDataBuffer() {
  if (CHANNELS_COUNT <= 0) {
    console.warn('Invalid channel count:', CHANNELS_COUNT);
    return;
  }
  
  bufferSize = Math.ceil(SAMPLE_RATE * TIME_WINDOW);
  ringBuffer = new RingBuffer(CHANNELS_COUNT, bufferSize);
  pixelsPerSample = WAVEFORM_WIDTH / bufferSize;
  
  // 初始化通道可见性
  channelVisibility.value = Array(CHANNELS_COUNT).fill(true);
  
  // 初始化最后绘制点
  lastPoints = Array(CHANNELS_COUNT).fill(null).map(() => ({ x: CHANNEL_LABEL_WIDTH, y: 0 }));
  
  console.log(`Buffer initialized: ${CHANNELS_COUNT} channels, ${bufferSize} samples, ${pixelsPerSample} pixels/sample`);
}

// 初始化Canvas
function initCanvas() {
  if (!canvasRef.value || !spectrumCanvasRef.value) return;
  
  const canvas = canvasRef.value;
  const spectrumCanvas = spectrumCanvasRef.value;
  
  canvas.width = CANVAS_WIDTH;
  canvas.height = CANVAS_HEIGHT;
  spectrumCanvas.width = SPECTRUM_WIDTH;
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
    // 初始化频域画布
    initSpectrumCanvas();
  }
}

// 新增频域画布初始化
function initSpectrumCanvas() {
  if (!spectrumCtx) return;
  
  spectrumCtx.fillStyle = '#ffffff';
  spectrumCtx.fillRect(0, 0, SPECTRUM_WIDTH, SPECTRUM_HEIGHT);
  
  // 绘制频域背景网格
  drawSpectrumGrid();
}

// 新增频域网格绘制
function drawSpectrumGrid() {
  if (!spectrumCtx) return;
  
  spectrumCtx.save();
  
  // 清除画布
  spectrumCtx.fillStyle = '#ffffff';
  spectrumCtx.fillRect(0, 0, SPECTRUM_WIDTH, SPECTRUM_HEIGHT);
  
  // 绘制频率网格线
  spectrumCtx.strokeStyle = '#e0e0e0';
  spectrumCtx.lineWidth = 0.5;
  
  // 垂直线（频率刻度）
  for (let i = 0; i <= 10; i++) {
    const x = (SPECTRUM_WIDTH / 10) * i;
    spectrumCtx.beginPath();
    spectrumCtx.moveTo(x, 0);
    spectrumCtx.lineTo(x, SPECTRUM_HEIGHT);
    spectrumCtx.stroke();
  }
  
  // 水平线（幅度刻度）
  if (CHANNELS_COUNT > 0) {
    const channelHeight = SPECTRUM_HEIGHT / CHANNELS_COUNT;
    for (let ch = 0; ch <= CHANNELS_COUNT; ch++) {
      const y = channelHeight * ch;
      spectrumCtx.beginPath();
      spectrumCtx.moveTo(0, y);
      spectrumCtx.lineTo(SPECTRUM_WIDTH, y);
      spectrumCtx.stroke();
    }
  }
  
  spectrumCtx.restore();
}

// 绘制背景网格和通道标签
function drawGrid() {
  if (!ctx) return;
  
  ctx.save();
  
  // 清除整个画布
  ctx.clearRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
  
  // 绘制左侧通道标签区域背景
  ctx.fillStyle = '#f8f9fa';
  ctx.fillRect(0, 0, CHANNEL_LABEL_WIDTH, CANVAS_HEIGHT);
  
  // 绘制波形区域背景
  ctx.fillStyle = '#ffffff';
  ctx.fillRect(CHANNEL_LABEL_WIDTH, 0, WAVEFORM_WIDTH, CANVAS_HEIGHT);
  
  // 绘制分隔线
  ctx.strokeStyle = '#dee2e6';
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(CHANNEL_LABEL_WIDTH, 0);
  ctx.lineTo(CHANNEL_LABEL_WIDTH, CANVAS_HEIGHT);
  ctx.stroke();
  
  // 绘制网格线
  ctx.strokeStyle = '#e0e0e0';
  ctx.lineWidth = 0.5;
  
  // 垂直网格线 (时间) - 只在波形区域
  const timeStep = WAVEFORM_WIDTH / 10;
  for (let i = 1; i <= 10; i++) {
    const x = CHANNEL_LABEL_WIDTH + i * timeStep;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, CANVAS_HEIGHT);
    ctx.stroke();
  }
  
  // 水平网格线和通道标签
  if (CHANNELS_COUNT > 0) {
    const channelHeight = CANVAS_HEIGHT / CHANNELS_COUNT;
    
    for (let ch = 0; ch < CHANNELS_COUNT; ch++) {
      const y = channelHeight * (ch + 1);
      
      // 绘制水平分隔线
      ctx.strokeStyle = '#e0e0e0';
      ctx.lineWidth = 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(CANVAS_WIDTH, y);
      ctx.stroke();
      
      // 绘制通道标签
      drawChannelLabel(ch, channelHeight);
    }
  }
  
  ctx.restore();
}

// 绘制通道标签
function drawChannelLabel(channelIndex: number, channelHeight: number) {
  if (!ctx) return;
  
  const isVisible = channelVisibility.value[channelIndex];
  const isHovered = hoveredChannel.value === channelIndex;
  const isSelected = selectedChannels.value.has(channelIndex);
  const channelColor = channelColors[channelIndex % channelColors.length];
  
  const centerY = channelHeight * (channelIndex + 0.5);
  const labelRect = {
    x: 5,
    y: centerY - 15,
    width: CHANNEL_LABEL_WIDTH - 10,
    height: 30
  };
  
  ctx.save();
  
  // 绘制标签背景
  if (isHovered || isSelected) {
    ctx.fillStyle = isSelected ? channelColor + '30' : '#f0f0f0';
    ctx.fillRect(labelRect.x, labelRect.y, labelRect.width, labelRect.height);
  }
  
  // 绘制边框
  ctx.strokeStyle = isVisible ? channelColor : '#ccc';
  ctx.lineWidth = isSelected ? 2 : 1;
  ctx.strokeRect(labelRect.x, labelRect.y, labelRect.width, labelRect.height);
  
  // 绘制颜色指示器
  ctx.fillStyle = isVisible ? channelColor : '#ccc';
  ctx.fillRect(labelRect.x + 5, centerY - 3, 6, 6);
  
  // 绘制通道文本
  ctx.fillStyle = isVisible ? '#333' : '#999';
  ctx.font = '12px Inter, Arial';
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(`CH${channelIndex + 1}`, labelRect.x + 18, centerY);
  
  ctx.restore();
}

// 使用后端频域数据的绘制函数
function drawSpectrumFromBackend() {
  const now = Date.now();
  
  // 频域更新节流控制⭐
  if (now - lastFreqRenderTime < FREQ_RENDER_INTERVAL) {
    return; // 跳过本次渲染
  }
  lastFreqRenderTime = now;
  
  if (!spectrumCtx || CHANNELS_COUNT <= 0 || spectrumData.value.length === 0) return;
  
  // 更新频域更新率显示
  const deltaTime = now - lastFrequencyUpdate;
  if (deltaTime > 0) {
    frequencyUpdateRate.value = 1000 / deltaTime;
  }
  lastFrequencyUpdate = now;
  
  // 重绘背景
  drawSpectrumGrid();
  
  const channelHeight = SPECTRUM_HEIGHT / CHANNELS_COUNT;
  
  // 绘制每个通道的频谱
  for (const freqData of spectrumData.value) {
    const ch = freqData.channel_index;
    
    if (ch >= CHANNELS_COUNT || !channelVisibility.value[ch]) continue;
    
    const channelY = ch * channelHeight;
    const isSelected = selectedChannels.value.has(ch);
    
    spectrumCtx.strokeStyle = channelColors[ch % channelColors.length];
    spectrumCtx.lineWidth = isSelected ? 2.5 : 1.5;
    spectrumCtx.fillStyle = channelColors[ch % channelColors.length] + '20'; // 半透明填充
    
    const spectrum = freqData.spectrum;
    const freqBinWidth = SPECTRUM_WIDTH / spectrum.length;
    
    spectrumCtx.beginPath();
    spectrumCtx.moveTo(0, channelY + channelHeight);
    
    // 绘制频谱曲线
    for (let i = 0; i < spectrum.length; i++) {
      const magnitude = Math.min(spectrum[i] / 50, 1); // 归一化到0-1
      const x = i * freqBinWidth;
      const y = channelY + channelHeight - (magnitude * channelHeight * 0.8);
      
      if (i === 0) {
        spectrumCtx.moveTo(x, y);
      } else {
        spectrumCtx.lineTo(x, y);
      }
    }
    
    // 完成填充路径
    spectrumCtx.lineTo(spectrum.length * freqBinWidth, channelY + channelHeight);
    spectrumCtx.lineTo(0, channelY + channelHeight);
    spectrumCtx.closePath();
    
    // 填充和描边
    spectrumCtx.fill();
    spectrumCtx.stroke();
    
    // 绘制通道标签
    spectrumCtx.fillStyle = '#333';
    spectrumCtx.font = '12px Inter, Arial';
    spectrumCtx.textAlign = 'right';
    spectrumCtx.fillText(`CH${ch + 1}`, SPECTRUM_WIDTH - 5, channelY + 15);
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

// Canvas点击事件处理
function handleCanvasClick(event: MouseEvent) {
  if (!canvasRef.value || CHANNELS_COUNT <= 0) return;
  
  const rect = canvasRef.value.getBoundingClientRect();
  const x = (event.clientX - rect.left) * (CANVAS_WIDTH / rect.width);
  const y = (event.clientY - rect.top) * (CANVAS_HEIGHT / rect.height);
  
  // 只处理标签区域的点击
  if (x <= CHANNEL_LABEL_WIDTH) {
    const channelHeight = CANVAS_HEIGHT / CHANNELS_COUNT;
    const clickedChannel = Math.floor(y / channelHeight);
    
    if (clickedChannel >= 0 && clickedChannel < CHANNELS_COUNT) {
      if (event.ctrlKey || event.metaKey) {
        // Ctrl+点击：多选高亮
        if (selectedChannels.value.has(clickedChannel)) {
          selectedChannels.value.delete(clickedChannel);
        } else {
          selectedChannels.value.add(clickedChannel);
        }
        selectedChannels.value = new Set(selectedChannels.value); // 触发响应式更新
      } else {
        // 普通点击：切换可见性
        toggleChannel(clickedChannel);
      }
      
      // 重绘标签区域
      drawGrid();
    }
  }
}

// Canvas鼠标移动事件处理
function handleCanvasMouseMove(event: MouseEvent) {
  if (!canvasRef.value || CHANNELS_COUNT <= 0) return;
  
  const rect = canvasRef.value.getBoundingClientRect();
  const x = (event.clientX - rect.left) * (CANVAS_WIDTH / rect.width);
  const y = (event.clientY - rect.top) * (CANVAS_HEIGHT / rect.height);
  
  if (x <= CHANNEL_LABEL_WIDTH) {
    const channelHeight = CANVAS_HEIGHT / CHANNELS_COUNT;
    const hoveredCh = Math.floor(y / channelHeight);
    
    if (hoveredCh >= 0 && hoveredCh < CHANNELS_COUNT) {
      if (hoveredChannel.value !== hoveredCh) {
        hoveredChannel.value = hoveredCh;
        drawGrid(); // 重绘以显示悬停效果
      }
      
      if (canvasRef.value) {
        canvasRef.value.style.cursor = 'pointer';
      }
    } else {
      if (hoveredChannel.value !== -1) {
        hoveredChannel.value = -1;
        drawGrid();
      }
    }
  } else {
    if (hoveredChannel.value !== -1) {
      hoveredChannel.value = -1;
      drawGrid();
    }
    
    if (canvasRef.value) {
      canvasRef.value.style.cursor = 'default';
    }
  }
}

// Canvas鼠标离开事件处理
function handleCanvasMouseLeave() {
  if (hoveredChannel.value !== -1) {
    hoveredChannel.value = -1;
    drawGrid(); // 重绘以清除悬停效果
  }
  
  if (canvasRef.value) {
    canvasRef.value.style.cursor = 'default';
  }
}

// 生命周期
onMounted(async () => {
  await nextTick();
  initDataBuffer();
  initCanvas();
  
  // 统一监听合并的帧数据（时域+频域）⭐
  const unlisten = await listen('frame-update', (event) => {
    const payload = event.payload as FramePayload;
    processFramePayload(payload);
  });
  
  // 移除向后兼容的旧格式监听 ⭐
  // const unlistenLegacy = await listen('eeg-data', (event) => {
  //   const batch = event.payload as EegBatch;
  //   // 转换为新格式
  //   const payload: FramePayload = {
  //     time_domain: batch,
  //     frequency_domain: [] // 频域数据为空
  //   };
  //   processFramePayload(payload);
  // });
  
  // 在组件卸载时清理
  onUnmounted(() => {
    unlisten();
    // unlistenLegacy(); // 移除这行
    if (renderLoopId) {
      cancelAnimationFrame(renderLoopId);
    }
  });
});

// 处理接收到的EEG数据
function processFramePayload(payload: FramePayload) {
  const now = Date.now();
  
  // 跟踪后端数据更新率
  if (lastBackendDataTime > 0) {
    const delta = now - lastBackendDataTime;
    backendDataRate.value = 1000 / delta;
  }
  lastBackendDataTime = now;
  
  // 1. 处理时域数据
  const batch = payload.time_domain;
  SAMPLE_RATE = batch.sample_rate;
  CHANNELS_COUNT = batch.channels_count;
  
  // 如果通道数改变，重新初始化
  if (!ringBuffer || ringBuffer.getCapacity() !== Math.ceil(SAMPLE_RATE * TIME_WINDOW)) {
    initDataBuffer();
  }
  
  // 将样本添加到环形缓冲区⭐
  if (ringBuffer) {
    ringBuffer.addBatch(batch.samples);
  }
  
  // 2. 处理频域数据
  if (payload.frequency_domain && payload.frequency_domain.length > 0) {
    spectrumData.value = payload.frequency_domain;
    drawSpectrumFromBackend();
  }
}

// 波前式渲染主循环
function renderLoop() {
  const now = Date.now();
  timedomainFrameCount++;
  
  if (now - lastTimedomainRender >= 1000) {
    timedomainRenderRate.value = timedomainFrameCount;
    timedomainFrameCount = 0;
    lastTimedomainRender = now;
  }
  
  if (!ctx || CHANNELS_COUNT <= 0 || !ringBuffer) return;
  
  const pointsToProcess = 4;
  
  // 1. 计算波前移动量
  const waveAdvance = pointsToProcess * pixelsPerSample;
  const nextWaveFrontX = waveFrontX.value + waveAdvance;
  
  // 2. 局部擦除策略（支持循环）⭐
  if (nextWaveFrontX >= CANVAS_WIDTH) {
    // 情况A：波前需要循环到画布左端
    const remainingWidth = CANVAS_WIDTH - waveFrontX.value;
    ctx.clearRect(waveFrontX.value, 0, remainingWidth, CANVAS_HEIGHT);
    
    const wrapAroundWidth = nextWaveFrontX - CANVAS_WIDTH;
    ctx.clearRect(CHANNEL_LABEL_WIDTH, 0, wrapAroundWidth, CANVAS_HEIGHT);
  } else {
    // 情况B：普通前进
    const clearWidth = Math.ceil(waveAdvance) + 2; // 2像素抗锯齿余量
    ctx.clearRect(waveFrontX.value, 0, clearWidth, CANVAS_HEIGHT);
  }
  
  // 重绘背景网格（在擦除区域）
  redrawGridInRegion(waveFrontX.value, waveAdvance, nextWaveFrontX >= CANVAS_WIDTH);
  
  // 3. 绘制新的波形数据
  for (let ch = 0; ch < CHANNELS_COUNT; ch++) {
    if (!channelVisibility.value[ch]) continue;
    
    const isSelected = selectedChannels.value.has(ch);
    ctx.strokeStyle = channelColors[ch % channelColors.length];
    ctx.lineWidth = isSelected ? 2.5 : 1.5;
    ctx.beginPath();
    
    // 从上一帧的最后点开始
    ctx.moveTo(lastPoints[ch].x, lastPoints[ch].y);
    
    // 获取新数据点（从环形缓冲区）⭐
    const samples = ringBuffer.getChannelSamples(ch, pointsToProcess);
    
    // 绘制新的数据点
    for (let i = 0; i < pointsToProcess; i++) {
      let x = waveFrontX.value + i * pixelsPerSample;
      
      // 处理循环绘制⭐
      if (x >= CANVAS_WIDTH) {
        x = CHANNEL_LABEL_WIDTH + (x - CANVAS_WIDTH);
      }
      
      // 确保不超出波形区域左边界
      if (x < CHANNEL_LABEL_WIDTH) continue;
      
      const channelHeight = CANVAS_HEIGHT / CHANNELS_COUNT;
      const channelCenter = channelHeight * (ch + 0.5);
      const amplitude = samples[i];
      const scale = channelHeight * 0.4 / 100;
      const y = channelCenter - amplitude * scale;
      
      ctx.lineTo(x, y);
      
      // 记录最后一点供下一帧使用
      if (i === pointsToProcess - 1) {
        lastPoints[ch] = { x, y };
      }
    }
    
    ctx.stroke();
  }
  
  // 4. 更新波前位置（循环处理）⭐
  waveFrontX.value = nextWaveFrontX % CANVAS_WIDTH;
  if (waveFrontX.value < CHANNEL_LABEL_WIDTH) {
    waveFrontX.value = CHANNEL_LABEL_WIDTH;
  }
  
  renderLoopId = requestAnimationFrame(renderLoop);
}

// 新增：重绘网格辅助函数
function redrawGridInRegion(startX: number, width: number, isWrapped: boolean) {
  if (!ctx) return;
  
  ctx.save();
  
  // 绘制背景色
  ctx.fillStyle = '#ffffff';
  if (isWrapped) {
    const remainingWidth = CANVAS_WIDTH - startX;
    ctx.fillRect(startX, 0, remainingWidth, CANVAS_HEIGHT);
    ctx.fillRect(CHANNEL_LABEL_WIDTH, 0, width - remainingWidth, CANVAS_HEIGHT);
  } else {
    ctx.fillRect(startX, 0, width + 2, CANVAS_HEIGHT);
  }
  
  // 重绘网格线
  ctx.strokeStyle = '#e0e0e0';
  ctx.lineWidth = 0.5;
  ctx.beginPath();
  
  // 垂直网格线
  const timeStep = WAVEFORM_WIDTH / 10;
  for (let i = 1; i <= 10; i++) {
    const x = CHANNEL_LABEL_WIDTH + i * timeStep;
    if ((x >= startX && x <= startX + width) || 
        (isWrapped && x >= CHANNEL_LABEL_WIDTH && x <= CHANNEL_LABEL_WIDTH + (width - (CANVAS_WIDTH - startX)))) {
      ctx.moveTo(x, 0);
      ctx.lineTo(x, CANVAS_HEIGHT);
    }
  }
  
  // 水平网格线
  const channelHeight = CANVAS_HEIGHT / CHANNELS_COUNT;
  for (let ch = 0; ch <= CHANNELS_COUNT; ch++) {
    const y = channelHeight * ch;
    if (isWrapped) {
      ctx.moveTo(startX, y);
      ctx.lineTo(CANVAS_WIDTH, y);
      ctx.moveTo(CHANNEL_LABEL_WIDTH, y);
      ctx.lineTo(CHANNEL_LABEL_WIDTH + (width - (CANVAS_WIDTH - startX)), y);
    } else {
      ctx.moveTo(startX, y);
      ctx.lineTo(startX + width + 2, y);
    }
  }
  
  ctx.stroke();
  ctx.restore();
}
</script>

<template>
  <div class="eeg-visualizer">
    <!-- 标题栏 -->
    <header class="header">
      <h1>Open CortexArray - EEG可视化系统 V2.4</h1>
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
        <!-- 左侧时域波形 (66%宽度) -->
        <div class="time-domain-panel">
          <h3>实时EEG波形 ({{ CHANNELS_COUNT }}通道, 波前式渲染)</h3>
          <canvas 
            ref="canvasRef" 
            class="eeg-canvas"
            :style="{ width: '100%', height: '400px' }"
            @click="handleCanvasClick"
            @mousemove="handleCanvasMouseMove"
            @mouseleave="handleCanvasMouseLeave"
          ></canvas>
          <div class="wave-front-indicator" :style="{ left: ((waveFrontX - CHANNEL_LABEL_WIDTH) / WAVEFORM_WIDTH * 100) + '%', marginLeft: (CHANNEL_LABEL_WIDTH / CANVAS_WIDTH * 100) + '%' }"></div>
        </div>

        <!-- 右侧频域分析 (33%宽度) -->
        <div class="frequency-panel">
          <h3>实时频谱分析 (1-50Hz)</h3>
          <canvas 
            ref="spectrumCanvasRef" 
            class="spectrum-canvas"
            :style="{ width: '100%', height: '400px' }"
          ></canvas>
          <div class="frequency-legend">
            <div class="freq-range">1Hz</div>
            <div class="freq-range">25Hz</div>
            <div class="freq-range">50Hz</div>
          </div>
        </div>
      </div>
    </div>

    <!-- 信息面板 -->
    <div class="info-panel">
      <div class="info-item">
        <strong>渲染模式:</strong> 双画布实时渲染 (~30FPS)
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
      <div class="info-item">
        <strong>频域更新:</strong> {{ Math.round(frequencyUpdateRate) }}Hz
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