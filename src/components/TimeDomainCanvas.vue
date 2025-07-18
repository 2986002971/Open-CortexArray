<template>
  <div class="time-domain-panel">
    <h3>实时EEG波形 ({{ channelsCount }}通道, 波前式渲染)</h3>
    <canvas 
      ref="canvasRef" 
      class="eeg-canvas"
      :style="{ width: '100%', height: '400px' }"
      @click="handleCanvasClick"
      @mousemove="handleCanvasMouseMove"
      @mouseleave="handleCanvasMouseLeave"
    ></canvas>
    <div 
      class="wave-front-indicator" 
      :style="{ 
        left: ((waveFrontX - CHANNEL_LABEL_WIDTH) / WAVEFORM_WIDTH * 100) + '%', 
        marginLeft: (CHANNEL_LABEL_WIDTH / CANVAS_WIDTH * 100) + '%' 
      }"
    ></div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch, nextTick } from 'vue';

// Props
interface Props {
  channelsCount: number;
  sampleRate: number;
  channelVisibility: boolean[];
  selectedChannels: Set<number>;
  hoveredChannel: number;
  isConnected: boolean;
}

const props = defineProps<Props>();

// Emits
interface Emits {
  (e: 'toggle-channel', channelIndex: number): void;
  (e: 'select-channel', channelIndex: number, isMultiSelect: boolean): void;
  (e: 'hover-channel', channelIndex: number): void;
  (e: 'update-render-rate', rate: number): void;
  (e: 'update-wave-front', position: number): void;
}

const emit = defineEmits<Emits>();

// 渲染常量
const CANVAS_WIDTH = 1000;
const CANVAS_HEIGHT = 600;
const CHANNEL_LABEL_WIDTH = 80;
const WAVEFORM_WIDTH = CANVAS_WIDTH - CHANNEL_LABEL_WIDTH;
const TIME_WINDOW = 10;

// Canvas相关
const canvasRef = ref<HTMLCanvasElement | null>(null);
let ctx: CanvasRenderingContext2D | null = null;

// 渲染状态
const waveFrontX = ref(CHANNEL_LABEL_WIDTH);
let lastPoints: { x: number; y: number }[] = [];
let renderLoopId = 0;
let bufferSize = 0;
let pixelsPerSample = 0;

// 性能监控
let timedomainFrameCount = 0;
let lastTimedomainRender = 0;

// 通道颜色
const channelColors = ['#FF6B6B', '#4ECDC4', '#45B7D1', '#96CEB4', '#FFEAA7', '#DDA0DD', '#98D8E8', '#F7DC6F'];

// 环形缓冲区类
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

  addBatch(samples: any[]) {
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

  getCapacity(): number {
    return this.capacity;
  }
}

// 数据缓冲区
let ringBuffer: RingBuffer | null = null;

// 初始化函数
function initDataBuffer() {
  if (props.channelsCount <= 0) {
    console.warn('Invalid channel count:', props.channelsCount);
    return;
  }
  
  bufferSize = Math.ceil(props.sampleRate * TIME_WINDOW);
  ringBuffer = new RingBuffer(props.channelsCount, bufferSize);
  pixelsPerSample = WAVEFORM_WIDTH / bufferSize;
  
  // 初始化最后绘制点
  lastPoints = Array(props.channelsCount).fill(null).map(() => ({ 
    x: CHANNEL_LABEL_WIDTH, 
    y: 0 
  }));
  
  console.log(`Time domain buffer: ${props.channelsCount} channels, ${bufferSize} samples, ${pixelsPerSample} pixels/sample`);
}

function initCanvas() {
  if (!canvasRef.value) return;
  
  const canvas = canvasRef.value;
  canvas.width = CANVAS_WIDTH;
  canvas.height = CANVAS_HEIGHT;
  
  ctx = canvas.getContext('2d');
  
  if (ctx) {
    ctx.strokeStyle = '#333';
    ctx.lineWidth = 1;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    
    drawGrid();
  }
}

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
  
  // 垂直网格线 (时间)
  const timeStep = WAVEFORM_WIDTH / 10;
  for (let i = 1; i <= 10; i++) {
    const x = CHANNEL_LABEL_WIDTH + i * timeStep;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, CANVAS_HEIGHT);
    ctx.stroke();
  }
  
  // 水平网格线和通道标签
  if (props.channelsCount > 0) {
    const channelHeight = CANVAS_HEIGHT / props.channelsCount;
    
    for (let ch = 0; ch < props.channelsCount; ch++) {
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

function drawChannelLabel(channelIndex: number, channelHeight: number) {
  if (!ctx) return;
  
  const isVisible = props.channelVisibility[channelIndex];
  const isHovered = props.hoveredChannel === channelIndex;
  const isSelected = props.selectedChannels.has(channelIndex);
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

// 事件处理
function handleCanvasClick(event: MouseEvent) {
  if (!canvasRef.value || props.channelsCount <= 0) return;
  
  const rect = canvasRef.value.getBoundingClientRect();
  const x = (event.clientX - rect.left) * (CANVAS_WIDTH / rect.width);
  const y = (event.clientY - rect.top) * (CANVAS_HEIGHT / rect.height);
  
  // 只处理标签区域的点击
  if (x <= CHANNEL_LABEL_WIDTH) {
    const channelHeight = CANVAS_HEIGHT / props.channelsCount;
    const clickedChannel = Math.floor(y / channelHeight);
    
    if (clickedChannel >= 0 && clickedChannel < props.channelsCount) {
      if (event.ctrlKey || event.metaKey) {
        // Ctrl+点击：多选高亮
        emit('select-channel', clickedChannel, true);
      } else {
        // 普通点击：切换可见性
        emit('toggle-channel', clickedChannel);
      }
      
      // 重绘标签区域
      drawGrid();
    }
  }
}

function handleCanvasMouseMove(event: MouseEvent) {
  if (!canvasRef.value || props.channelsCount <= 0) return;
  
  const rect = canvasRef.value.getBoundingClientRect();
  const x = (event.clientX - rect.left) * (CANVAS_WIDTH / rect.width);
  const y = (event.clientY - rect.top) * (CANVAS_HEIGHT / rect.height);
  
  if (x <= CHANNEL_LABEL_WIDTH) {
    const channelHeight = CANVAS_HEIGHT / props.channelsCount;
    const hoveredCh = Math.floor(y / channelHeight);
    
    if (hoveredCh >= 0 && hoveredCh < props.channelsCount) {
      emit('hover-channel', hoveredCh);
      
      if (canvasRef.value) {
        canvasRef.value.style.cursor = 'pointer';
      }
    }
  } else {
    emit('hover-channel', -1);
    
    if (canvasRef.value) {
      canvasRef.value.style.cursor = 'default';
    }
  }
}

function handleCanvasMouseLeave() {
  emit('hover-channel', -1);
  
  if (canvasRef.value) {
    canvasRef.value.style.cursor = 'default';
  }
}

// 渲染循环
function renderLoop() {
  const now = Date.now();
  timedomainFrameCount++;
  
  if (now - lastTimedomainRender >= 1000) {
    emit('update-render-rate', timedomainFrameCount);
    timedomainFrameCount = 0;
    lastTimedomainRender = now;
  }
  
  if (!ctx || props.channelsCount <= 0 || !ringBuffer) {
    renderLoopId = requestAnimationFrame(renderLoop);
    return;
  }
  
  const pointsToProcess = 4;
  
  // 计算波前移动量
  const waveAdvance = pointsToProcess * pixelsPerSample;
  const nextWaveFrontX = waveFrontX.value + waveAdvance;
  
  // 局部擦除策略（支持循环）
  if (nextWaveFrontX >= CANVAS_WIDTH) {
    const remainingWidth = CANVAS_WIDTH - waveFrontX.value;
    ctx.clearRect(waveFrontX.value, 0, remainingWidth, CANVAS_HEIGHT);
    
    const wrapAroundWidth = nextWaveFrontX - CANVAS_WIDTH;
    ctx.clearRect(CHANNEL_LABEL_WIDTH, 0, wrapAroundWidth, CANVAS_HEIGHT);
  } else {
    const clearWidth = Math.ceil(waveAdvance) + 2;
    ctx.clearRect(waveFrontX.value, 0, clearWidth, CANVAS_HEIGHT);
  }
  
  // 重绘背景网格
  redrawGridInRegion(waveFrontX.value, waveAdvance, nextWaveFrontX >= CANVAS_WIDTH);
  
  // 绘制新的波形数据
  for (let ch = 0; ch < props.channelsCount; ch++) {
    if (!props.channelVisibility[ch]) continue;
    
    const isSelected = props.selectedChannels.has(ch);
    ctx.strokeStyle = channelColors[ch % channelColors.length];
    ctx.lineWidth = isSelected ? 2.5 : 1.5;
    ctx.beginPath();
    
    ctx.moveTo(lastPoints[ch].x, lastPoints[ch].y);
    
    const samples = ringBuffer.getChannelSamples(ch, pointsToProcess);
    
    for (let i = 0; i < pointsToProcess; i++) {
      let x = waveFrontX.value + i * pixelsPerSample;
      
      if (x >= CANVAS_WIDTH) {
        x = CHANNEL_LABEL_WIDTH + (x - CANVAS_WIDTH);
      }
      
      if (x < CHANNEL_LABEL_WIDTH) continue;
      
      const channelHeight = CANVAS_HEIGHT / props.channelsCount;
      const channelCenter = channelHeight * (ch + 0.5);
      const amplitude = samples[i];
      const scale = channelHeight * 0.4 / 100;
      const y = channelCenter - amplitude * scale;
      
      ctx.lineTo(x, y);
      
      if (i === pointsToProcess - 1) {
        lastPoints[ch] = { x, y };
      }
    }
    
    ctx.stroke();
  }
  
  // 更新波前位置
  waveFrontX.value = nextWaveFrontX % CANVAS_WIDTH;
  if (waveFrontX.value < CHANNEL_LABEL_WIDTH) {
    waveFrontX.value = CHANNEL_LABEL_WIDTH;
  }
  
  emit('update-wave-front', waveFrontX.value);
  
  renderLoopId = requestAnimationFrame(renderLoop);
}

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
  const channelHeight = CANVAS_HEIGHT / props.channelsCount;
  for (let ch = 0; ch <= props.channelsCount; ch++) {
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

// 公共方法
function addBatchData(samples: any[]) {
  if (ringBuffer) {
    ringBuffer.addBatch(samples);
  }
}

function startRenderLoop() {
  if (!renderLoopId) {
    renderLoop();
  }
}

function stopRenderLoop() {
  if (renderLoopId) {
    cancelAnimationFrame(renderLoopId);
    renderLoopId = 0;
  }
}

// 监听器
watch(() => props.channelsCount, () => {
  initDataBuffer();
  initCanvas();
}, { immediate: true });

watch(() => props.hoveredChannel, () => {
  drawGrid();
});

watch(() => props.selectedChannels, () => {
  drawGrid();
}, { deep: true });

// 生命周期
onMounted(async () => {
  await nextTick();
  initCanvas();
});

onUnmounted(() => {
  stopRenderLoop();
});

// 暴露方法给父组件
defineExpose({
  addBatchData,
  startRenderLoop,
  stopRenderLoop,
  initDataBuffer,
  initCanvas
});
</script>

<style scoped>
.time-domain-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  position: relative;
}

.time-domain-panel h3 {
  margin: 0 0 1rem 0;
  font-size: 1.1rem;
  font-weight: 600;
  color: #495057;
  text-align: center;
}

.eeg-canvas {
  flex: 1;
  border: 2px solid #e0e0e0;
  border-radius: 8px;
  background: #fafafa;
  display: block;
  box-shadow: inset 0 2px 8px rgba(0, 0, 0, 0.1);
  cursor: default;
}

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
</style>