<!-- filepath: src/components/TimeDomainCanvas.vue -->
<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch, nextTick } from 'vue';
import { WebglPlot, WebglLine, ColorRGBA } from 'webgl-plot';

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

// WebGL相关
const canvasRef = ref<HTMLCanvasElement | null>(null);
let wglp: WebglPlot | null = null;

// ✅ 优化：减少时间窗口和数据密度
const TIME_WINDOW = 5; // 从10秒减少到5秒
const BATCH_SIZE = 8; // 增加批处理大小，减少更新频率
let TOTAL_POINTS = TIME_WINDOW * (props.sampleRate || 250); // 总点数

// 线条管理
const channelLines: WebglLine[] = [];
const channelColors = ['#FF6B6B', '#4ECDC4', '#45B7D1', '#96CEB4', '#FFEAA7', '#DDA0DD', '#98D8E8', '#F7DC6F'];

// 波前管理
let currentIndex = 0; // 当前波前位置索引
const waveFrontPosition = ref(0); // 波前位置比例 [0, 1]

// ✅ 优化：简化的环形缓冲区
class OptimizedRingBuffer {
  private buffers: Float32Array[];
  private head: number = 0;
  private readonly capacity: number;
  private readonly channelCount: number;

  constructor(channels: number, capacity: number) {
    this.channelCount = channels;
    this.capacity = capacity;
    this.buffers = Array(channels).fill(null).map(() => new Float32Array(capacity));
  }

  // ✅ 优化：批量添加样本，减少函数调用开销
  addBatch(samples: any[]) {
    for (const sample of samples) {
      if (sample && sample.channels) {
        for (let ch = 0; ch < this.channelCount && ch < sample.channels.length; ch++) {
          this.buffers[ch][this.head] = sample.channels[ch] || 0;
        }
        this.head = (this.head + 1) % this.capacity;
      }
    }
  }

  // ✅ 优化：直接返回缓冲区引用，避免数据拷贝
  getLatestBatch(channel: number, count: number): { data: Float32Array; startIndex: number } {
    if (channel >= this.buffers.length) {
      return { data: new Float32Array(count), startIndex: 0 };
    }
    
    const startIndex = Math.max(0, this.head - count);
    return { 
      data: this.buffers[channel], 
      startIndex: startIndex 
    };
  }

  // 获取可用数据数量
  getAvailableCount(): number {
    return Math.min(this.head, this.capacity);
  }
}

// 数据缓冲区
let ringBuffer: OptimizedRingBuffer | null = null;
let renderLoopId = 0;

// ✅ 优化：性能监控和自适应帧率
let frameCount = 0;
let lastFrameTime = 0;

// ✅ 优化：缓存经常计算的值
let cachedChannelOffsets: number[] = [];
let cachedChannelScale = 0;
let lastChannelsCount = 0;

// 初始化WebGL
function initWebGL() {
  if (!canvasRef.value) {
    console.warn('Canvas ref not available for WebGL init');
    return;
  }

  try {
    const canvas = canvasRef.value;
    
    // 设置画布尺寸
    const devicePixelRatio = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * devicePixelRatio;
    canvas.height = rect.height * devicePixelRatio;
    
    console.log(`Time domain WebGL Canvas: ${canvas.width}x${canvas.height}, DPR: ${devicePixelRatio}`);
    
    // 初始化WebGLplot
    wglp = new WebglPlot(canvas);
    
    console.log('✅ 时域WebGL初始化成功');
    
    // 初始化线条
    initChannelLines();
    
  } catch (error) {
    console.error('❌ 时域WebGL初始化失败:', error);
  }
}

// 初始化数据缓冲区
function initDataBuffer() {
  if (props.channelsCount <= 0) {
    console.warn('Invalid channel count:', props.channelsCount);
    return;
  }
  
  // ✅ 优化：更新总点数
  TOTAL_POINTS = TIME_WINDOW * props.sampleRate;
  
  // ✅ 优化：缓存大小基于批处理大小
  const bufferSize = Math.max(BATCH_SIZE * 4, Math.ceil(props.sampleRate * 0.05)); // 最少50ms的数据
  ringBuffer = new OptimizedRingBuffer(props.channelsCount, bufferSize);
  
  // ✅ 优化：预计算并缓存通道偏移和缩放
  updateCachedValues();
  
  console.log(`📊 优化后时域缓冲区: ${props.channelsCount}通道, ${TOTAL_POINTS}点, 缓冲:${bufferSize}样本`);
}

// ✅ 优化：预计算并缓存经常使用的值
function updateCachedValues() {
  if (props.channelsCount !== lastChannelsCount) {
    cachedChannelOffsets = [];
    for (let ch = 0; ch < props.channelsCount; ch++) {
      cachedChannelOffsets[ch] = calculateChannelOffset(ch);
    }
    cachedChannelScale = calculateChannelScale();
    lastChannelsCount = props.channelsCount;
    
    console.log(`📊 缓存更新: ${props.channelsCount}通道, 缩放=${cachedChannelScale.toFixed(4)}`);
  }
}

// 初始化通道线条
function initChannelLines() {
  if (!wglp) return;
  
  console.log(`🎨 初始化 ${props.channelsCount} 个通道的时域线条`);
  
  // 清除现有线条
  wglp.removeAllLines();
  channelLines.length = 0;
  
  // 更新缓存值
  updateCachedValues();
  
  // 为每个通道创建线条
  for (let ch = 0; ch < props.channelsCount; ch++) {
    const colorHex = channelColors[ch % channelColors.length];
    const color = hexToColorRGBA(colorHex);
    
    const line = new WebglLine(color, TOTAL_POINTS);
    
    // 初始化X轴：从-1到1，等间距分布
    line.lineSpaceX(-1, 2 / TOTAL_POINTS);
    
    // ✅ 优化：使用缓存的偏移值
    const channelOffset = cachedChannelOffsets[ch];
    for (let i = 0; i < TOTAL_POINTS; i++) {
      line.setY(i, channelOffset);
    }
    
    // 添加到WebGL绘图器
    wglp.addLine(line);
    channelLines.push(line);
  }
  
  // 重置波前位置
  currentIndex = 0;
  waveFrontPosition.value = 0;
  
  console.log(`✅ 创建了 ${channelLines.length} 条时域线条`);
}

// 计算通道在Y轴上的偏移
function calculateChannelOffset(channelIndex: number): number {
  if (props.channelsCount <= 1) return 0;
  
  // 将整个Y轴范围 [-1, 1] 分配给所有通道
  const channelHeight = 2 / props.channelsCount;
  const centerY = 1 - (channelIndex + 0.5) * channelHeight;
  
  return centerY;
}

// 计算通道的缩放因子
function calculateChannelScale(): number {
  if (props.channelsCount <= 1) return 0.4;
  
  // 每个通道可用的最大高度（留出一些间距）
  const maxChannelHeight = (2 / props.channelsCount) * 0.8;
  
  return maxChannelHeight / 200; // 假设信号范围是 [-100, 100]
}

// 颜色转换函数
function hexToColorRGBA(hex: string): ColorRGBA {
  const r = parseInt(hex.slice(1, 3), 16) / 255;
  const g = parseInt(hex.slice(3, 5), 16) / 255;
  const b = parseInt(hex.slice(5, 7), 16) / 255;
  return new ColorRGBA(r, g, b, 1.0);
}

// ✅ 大幅优化：移除波前清除，简化更新逻辑
function updateWaveFrontData() {
  if (!wglp || channelLines.length === 0 || !ringBuffer) {
    return;
  }
  
  const availableData = ringBuffer.getAvailableCount();
  if (availableData < BATCH_SIZE) {
    return; // 没有足够的数据，跳过此帧
  }
  
  // ✅ 优化：只有在有新数据时才更新
  for (let ch = 0; ch < props.channelsCount; ch++) {
    // ✅ 优化：跳过不可见的通道
    if (!props.channelVisibility[ch]) continue;
    
    const line = channelLines[ch];
    const channelOffset = cachedChannelOffsets[ch];
    
    // ✅ 优化：减少颜色更新频率，只在选中状态变化时更新
    const isSelected = props.selectedChannels.has(ch);
    const baseColor = hexToColorRGBA(channelColors[ch % channelColors.length]);
    
    // ✅ 优化：缓存颜色计算
    if (isSelected) {
      line.color = new ColorRGBA(
        Math.min(baseColor.r * 1.3, 1.0),
        Math.min(baseColor.g * 1.3, 1.0),
        Math.min(baseColor.b * 1.3, 1.0),
        1.0
      );
    } else if (line.color.r !== baseColor.r) { // 只在颜色真正变化时更新
      line.color = baseColor;
    }
    
    // ✅ 核心优化：简化波前更新，移除清除逻辑
    const { data, startIndex } = ringBuffer.getLatestBatch(ch, BATCH_SIZE);
    
    for (let i = 0; i < BATCH_SIZE; i++) {
      const pointIndex = (currentIndex + i) % TOTAL_POINTS;
      const dataIndex = (startIndex + availableData - BATCH_SIZE + i) % data.length;
      
      // 计算Y坐标：基线 + 幅度 * 缩放
      const amplitude = data[dataIndex] || 0;
      const y = channelOffset + amplitude * cachedChannelScale;
      
      // ✅ 优化：只更新当前点，不清除前方点
      line.setY(pointIndex, y);
    }
  }
  
  // ✅ 优化：批量处理不可见通道
  for (let ch = 0; ch < channelLines.length; ch++) {
    if (!props.channelVisibility[ch]) {
      const line = channelLines[ch];
      const channelOffset = cachedChannelOffsets[ch];
      
      // 只将当前波前区域设置为基线
      for (let i = 0; i < BATCH_SIZE; i++) {
        const pointIndex = (currentIndex + i) % TOTAL_POINTS;
        line.setY(pointIndex, channelOffset);
      }
    }
  }
  
  // 更新波前位置
  currentIndex = (currentIndex + BATCH_SIZE) % TOTAL_POINTS;
  waveFrontPosition.value = currentIndex / TOTAL_POINTS;
  
  emit('update-wave-front', waveFrontPosition.value);
  
  // ✅ 优化：WebGL更新（这里是最大的性能瓶颈）
  try {
    wglp.update();
  } catch (error) {
    console.error('WebGL更新错误:', error);
  }
}

// ✅ 简化：直接的渲染循环，移除自适应逻辑
function renderLoop() {
  const now = Date.now();
  frameCount++;
  
  // 只保留性能监控，移除自适应控制
  if (now - lastFrameTime >= 1000) {
    const currentFPS = frameCount;
    emit('update-render-rate', currentFPS);
    
    // ✅ 简化：只记录性能，不做任何自适应调整
    frameCount = 0;
    lastFrameTime = now;
  }
  
  // ✅ 直接更新，不跳帧
  updateWaveFrontData();
  
  renderLoopId = requestAnimationFrame(renderLoop);
}

// 事件处理函数保持不变...
function handleChannelClick(channelIndex: number, event: MouseEvent) {
  if (event.ctrlKey || event.metaKey) {
    emit('select-channel', channelIndex, true);
  } else {
    emit('toggle-channel', channelIndex);
  }
}

function handleChannelHover(channelIndex: number) {
  emit('hover-channel', channelIndex);
}

function handleChannelLeave() {
  emit('hover-channel', -1);
}

function handleCanvasClick(event: MouseEvent) {
  // WebGL画布点击事件
}

function handleCanvasMouseMove(event: MouseEvent) {
  // WebGL画布鼠标移动事件
}

function handleCanvasMouseLeave() {
  emit('hover-channel', -1);
}

// 公共方法
function addBatchData(samples: any[]) {
  if (ringBuffer) {
    ringBuffer.addBatch(samples);
  }
}

function startRenderLoop() {
  if (!renderLoopId) {
    console.log('🚀 启动优化后的WebGL时域渲染循环');
    renderLoop();
  }
}

function stopRenderLoop() {
  if (renderLoopId) {
    console.log('⏹️ 停止WebGL时域渲染循环');
    cancelAnimationFrame(renderLoopId);
    renderLoopId = 0;
  }
}

function initCanvas() {
  initWebGL();
}

// 窗口大小变化处理
function handleResize() {
  if (canvasRef.value && wglp) {
    const canvas = canvasRef.value;
    const devicePixelRatio = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    
    canvas.width = rect.width * devicePixelRatio;
    canvas.height = rect.height * devicePixelRatio;
    
    initWebGL();
  }
}

// 监听器
watch(() => props.channelsCount, () => {
  console.log(`📊 时域通道数变化: ${props.channelsCount}`);
  initDataBuffer();
  if (wglp && props.channelsCount > 0) {
    initChannelLines();
  }
}, { immediate: true });

watch(() => props.sampleRate, () => {
  console.log(`📊 时域采样率变化: ${props.sampleRate}`);
  initDataBuffer();
  if (wglp && props.channelsCount > 0) {
    initChannelLines();
  }
});

// 生命周期
onMounted(async () => {
  await nextTick();
  initDataBuffer();
  initWebGL();
  window.addEventListener('resize', handleResize);
});

onUnmounted(() => {
  stopRenderLoop();
  
  if (wglp) {
    wglp.removeAllLines();
    channelLines.length = 0;
    wglp = null;
  }
  
  window.removeEventListener('resize', handleResize);
  console.log('🧹 WebGL时域画布已清理');
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

<!-- 模板和样式保持不变 -->
<template>
  <div class="time-domain-panel">
    <h3>实时EEG波形 ({{ channelsCount }}通道, 优化WebGL波前式渲染)</h3>
    <canvas 
      ref="canvasRef" 
      class="eeg-canvas"
      :style="{ width: '100%', height: '400px' }"
      @click="handleCanvasClick"
      @mousemove="handleCanvasMouseMove"
      @mouseleave="handleCanvasMouseLeave"
    ></canvas>
    
    <!-- 通道标签叠加层 -->
    <div class="channel-labels-overlay">
      <div 
        v-for="(_, ch) in channelsCount"
        :key="ch"
        class="channel-label"
        :class="{
          'selected': selectedChannels.has(ch),
          'hovered': hoveredChannel === ch,
          'hidden': !channelVisibility[ch]
        }"
        :style="{ 
          top: `${(ch / channelsCount) * 100}%`,
          height: `${(100 / channelsCount)}%`,
          borderColor: channelColors[ch % channelColors.length],
          color: channelVisibility[ch] ? channelColors[ch % channelColors.length] : '#ccc'
        }"
        @click="handleChannelClick(ch, $event)"
        @mouseenter="handleChannelHover(ch)"
        @mouseleave="handleChannelLeave()"
      >
        <div class="channel-indicator" 
             :style="{ backgroundColor: channelVisibility[ch] ? channelColors[ch % channelColors.length] : '#ccc' }">
        </div>
        <span class="channel-text">CH{{ ch + 1 }}</span>
      </div>
    </div>
    
    <!-- 波前指示器 -->
    <div 
      class="wave-front-indicator" 
      :style="{ 
        left: `${(waveFrontPosition * 100)}%`
      }"
    ></div>
  </div>
</template>

<!-- 样式保持不变 -->
<style scoped>
/* 所有CSS样式保持不变 */
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
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

.eeg-canvas {
  flex: 1;
  border: 2px solid #dee2e6;
  border-radius: 6px;
  background: #000000;
  display: block;
  box-shadow: 
    inset 0 2px 4px rgba(0, 0, 0, 0.1),
    0 0 20px rgba(102, 126, 234, 0.1);
  transition: box-shadow 0.3s ease;
}

.eeg-canvas:hover {
  box-shadow: 
    inset 0 2px 4px rgba(0, 0, 0, 0.1),
    0 0 25px rgba(102, 126, 234, 0.2);
}

.channel-labels-overlay {
  position: absolute;
  left: 0;
  top: 3rem;
  bottom: 0;
  width: 80px;
  pointer-events: none;
}

.channel-label {
  position: absolute;
  width: 100%;
  display: flex;
  align-items: center;
  padding: 0.2rem 0.5rem;
  font-size: 0.75rem;
  font-weight: 600;
  background: rgba(255, 255, 255, 0.9);
  border-right: 2px solid;
  border-radius: 0 4px 4px 0;
  cursor: pointer;
  pointer-events: auto;
  transition: all 0.2s ease;
  z-index: 10;
}

.channel-label:hover {
  background: rgba(255, 255, 255, 0.95);
  transform: translateX(2px);
  box-shadow: 2px 0 8px rgba(0, 0, 0, 0.1);
}

.channel-label.selected {
  background: rgba(255, 255, 255, 1);
  transform: translateX(4px);
  box-shadow: 4px 0 12px rgba(0, 0, 0, 0.15);
  font-weight: 700;
}

.channel-label.hidden {
  opacity: 0.5;
  background: rgba(240, 240, 240, 0.8);
}

.channel-indicator {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-right: 0.3rem;
  flex-shrink: 0;
}

.channel-text {
  font-family: 'Monaco', 'Menlo', 'Consolas', monospace;
}

.wave-front-indicator {
  position: absolute;
  top: 3rem;
  bottom: 0;
  width: 2px;
  background: linear-gradient(to bottom, #ff6b6b, transparent);
  border-radius: 1px;
  box-shadow: 0 0 6px rgba(255, 107, 107, 0.8);
  animation: webgl-pulse 2s ease-in-out infinite alternate;
  z-index: 5;
}

@keyframes webgl-pulse {
  0%, 100% { 
    box-shadow: 0 0 6px rgba(255, 107, 107, 0.6); 
  }
  50% { 
    box-shadow: 0 0 12px rgba(255, 107, 107, 1); 
  }
}

@media (max-width: 768px) {
  .channel-labels-overlay {
    width: 60px;
  }
  
  .channel-label {
    font-size: 0.7rem;
    padding: 0.1rem 0.3rem;
  }
  
  .channel-indicator {
    width: 6px;
    height: 6px;
    margin-right: 0.2rem;
  }
}
</style>