<!-- filepath: src/components/TimeDomainCanvas.vue -->
<template>
  <div class="time-domain-panel">
    <h3>实时EEG波形 ({{ channelsCount }}通道, 事件驱动WebGL渲染)</h3>
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
    
    <!-- 性能统计 -->
    <div class="performance-stats" v-if="showDebugInfo">
      <span>帧率: {{ renderRate.toFixed(1) }}Hz</span>
      <span>延迟: {{ avgLatency.toFixed(1) }}ms</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch, nextTick } from 'vue';
import { WebglPlot, WebglLine, ColorRGBA } from 'webgl-plot';
import { listen } from '@tauri-apps/api/event';

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

// ✅ 大幅简化：移除时间窗口概念，改为循环缓冲区
let DISPLAY_POINTS = 1250; // 5秒 × 250Hz = 1250个显示点

// 线条管理
const channelLines: WebglLine[] = [];
const channelColors = ['#FF6B6B', '#4ECDC4', '#45B7D1', '#96CEB4', '#FFEAA7', '#DDA0DD', '#98D8E8', '#F7DC6F'];

// ✅ 波前管理：简化为纯索引
let currentIndex = 0;
const waveFrontPosition = ref(0);

// ✅ 完全移除环形缓冲区！
// ❌ 删除: class OptimizedRingBuffer
// ❌ 删除: let ringBuffer: OptimizedRingBuffer | null = null;
// ❌ 删除: let renderLoopId = 0;

// ✅ 性能监控：事件驱动模式
//TODO: 升级成多线程离屏画布
let frameCount = 0;
let lastFrameTime = 0;
const renderRate = ref(0);
const avgLatency = ref(0);
const showDebugInfo = ref(false);

// ✅ 缓存优化
let cachedChannelOffsets: number[] = [];
let cachedChannelScale = 0;
let lastChannelsCount = 0;

// ✅ 延迟统计
let latencyHistory: number[] = [];
const MAX_LATENCY_SAMPLES = 10;

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
    
    console.log(`📺 事件驱动WebGL Canvas: ${canvas.width}x${canvas.height}`);
    
    // 初始化WebGLplot
    wglp = new WebglPlot(canvas);
    
    console.log('✅ 事件驱动WebGL初始化成功');
    
    // 初始化线条
    initChannelLines();
    
  } catch (error) {
    console.error('❌ WebGL初始化失败:', error);
  }
}

// 预计算缓存值
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
    
    const line = new WebglLine(color, DISPLAY_POINTS);
    
    // 初始化X轴：从-1到1，等间距分布
    line.lineSpaceX(-1, 2 / DISPLAY_POINTS);
    
    // 使用缓存的偏移值初始化
    const channelOffset = cachedChannelOffsets[ch];
    for (let i = 0; i < DISPLAY_POINTS; i++) {
      line.setY(i, channelOffset);
    }
    
    wglp.addLine(line);
    channelLines.push(line);
  }
  
  // 重置波前
  currentIndex = 0;
  waveFrontPosition.value = 0;
  
  console.log(`✅ 创建了 ${channelLines.length} 条时域线条，每条${DISPLAY_POINTS}点`);
}

// 计算函数保持不变
function calculateChannelOffset(channelIndex: number): number {
  if (props.channelsCount <= 1) return 0;
  const channelHeight = 2 / props.channelsCount;
  const centerY = 1 - (channelIndex + 0.5) * channelHeight;
  return centerY;
}

function calculateChannelScale(): number {
  if (props.channelsCount <= 1) return 0.4;
  const maxChannelHeight = (2 / props.channelsCount) * 0.8;
  return maxChannelHeight / 200; // 信号范围 [-100, 100]
}

function hexToColorRGBA(hex: string): ColorRGBA {
  const r = parseInt(hex.slice(1, 3), 16) / 255;
  const g = parseInt(hex.slice(3, 5), 16) / 255;
  const b = parseInt(hex.slice(5, 7), 16) / 255;
  return new ColorRGBA(r, g, b, 1.0);
}

// ✅ 核心功能：事件驱动的直接渲染
function handleFrameUpdate(event: any) {
  const startTime = performance.now();
  
  if (!wglp || channelLines.length === 0) {
    return;
  }
  
  const { time_domain } = event.payload;
  if (!time_domain || !time_domain.samples) {
    return;
  }
  
  // ✅ 直接处理后端批处理好的样本
  const samples = time_domain.samples;
  console.log(`📦 直接处理 ${samples.length} 个样本`);
  
  // 更新每个样本
  for (const sample of samples) {
    updateSingleSample(sample);
  }
  
  // ✅ 一次性WebGL更新
  try {
    wglp.update();
  } catch (error) {
    console.error('WebGL更新错误:', error);
    return;
  }
  
  // 性能统计
  const endTime = performance.now();
  updatePerformanceStats(startTime, endTime);
  
  // 更新波前位置
  waveFrontPosition.value = currentIndex / DISPLAY_POINTS;
  emit('update-wave-front', waveFrontPosition.value);
}

// ✅ 单样本更新：最核心的渲染逻辑
function updateSingleSample(sample: any) {
  if (!sample || !sample.channels) {
    return;
  }
  
  // 为每个通道更新一个数据点
  for (let ch = 0; ch < props.channelsCount; ch++) {
    const line = channelLines[ch];
    if (!line) continue;
    
    // 处理可见性
    if (!props.channelVisibility[ch]) {
      // 不可见通道：设置为基线
      line.setY(currentIndex, cachedChannelOffsets[ch]);
      continue;
    }
    
    // 更新线条颜色（选中状态）
    updateLineColor(line, ch);
    
    // 计算并设置Y值
    const amplitude = sample.channels[ch] || 0;
    const y = cachedChannelOffsets[ch] + amplitude * cachedChannelScale;
    line.setY(currentIndex, y);
  }
  
  // 波前前进
  currentIndex = (currentIndex + 1) % DISPLAY_POINTS;
}

// ✅ 颜色更新优化
function updateLineColor(line: WebglLine, channelIndex: number) {
  const isSelected = props.selectedChannels.has(channelIndex);
  const baseColor = hexToColorRGBA(channelColors[channelIndex % channelColors.length]);
  
  if (isSelected) {
    // 选中状态：增强亮度
    line.color = new ColorRGBA(
      Math.min(baseColor.r * 1.3, 1.0),
      Math.min(baseColor.g * 1.3, 1.0),
      Math.min(baseColor.b * 1.3, 1.0),
      1.0
    );
  } else {
    // 普通状态
    line.color = baseColor;
  }
}

// ✅ 性能统计
function updatePerformanceStats(startTime: number, endTime: number) {
  const now = Date.now();
  frameCount++;
  
  // 计算延迟
  const latency = endTime - startTime;
  latencyHistory.push(latency);
  if (latencyHistory.length > MAX_LATENCY_SAMPLES) {
    latencyHistory.shift();
  }
  
  // 每秒更新一次统计
  if (now - lastFrameTime >= 1000) {
    renderRate.value = frameCount;
    avgLatency.value = latencyHistory.reduce((a, b) => a + b, 0) / latencyHistory.length;
    
    emit('update-render-rate', renderRate.value);
    
    console.log(`📊 渲染统计: ${renderRate.value}Hz, 平均延迟: ${avgLatency.value.toFixed(1)}ms`);
    
    frameCount = 0;
    lastFrameTime = now;
  }
}

// 事件处理函数
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
  // 点击切换调试信息显示
  showDebugInfo.value = !showDebugInfo.value;
}

function handleCanvasMouseMove(event: MouseEvent) {
  // 鼠标移动事件
}

function handleCanvasMouseLeave() {
  emit('hover-channel', -1);
}

// ✅ 大幅简化的公共方法
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
  if (wglp && props.channelsCount > 0) {
    initChannelLines();
  }
}, { immediate: true });

watch(() => props.sampleRate, () => {
  console.log(`📊 时域采样率变化: ${props.sampleRate}`);
  // 重新计算显示点数
  DISPLAY_POINTS = 5 * props.sampleRate; // 5秒显示窗口
  if (wglp && props.channelsCount > 0) {
    initChannelLines();
  }
});

watch(() => props.channelVisibility, () => {
  // 可见性变化时无需重新渲染，下次数据到达时自然处理
}, { deep: true });

watch(() => props.selectedChannels, () => {
  // 选中状态变化时无需重新渲染，下次数据到达时自然处理
}, { deep: true });

// ✅ 生命周期：事件驱动模式
onMounted(async () => {
  await nextTick();
  initWebGL();
  
  // ✅ 关键：监听后端数据事件而不是启动渲染循环
  const unlistenFrameUpdate = await listen('frame-update', handleFrameUpdate);
  
  // 保存取消监听的函数
  onUnmounted(() => {
    unlistenFrameUpdate();
  });
  
  window.addEventListener('resize', handleResize);
  console.log('🎧 事件监听器已设置，等待后端数据...');
});

onUnmounted(() => {
  // ✅ 无需停止渲染循环
  
  if (wglp) {
    wglp.removeAllLines();
    channelLines.length = 0;
    wglp = null;
  }
  
  window.removeEventListener('resize', handleResize);
  console.log('🧹 事件驱动WebGL画布已清理');
});

// ✅ 大幅简化的暴露方法
defineExpose({
  initCanvas
  // ✅ 移除了大量不再需要的方法
});
</script>

<style scoped>
.time-domain-panel {
  /* ✅ 确保参与flex布局 */
  height: 100%;
  width: 100%; /* ✅ 明确宽度 */
  display: flex;
  flex-direction: column;
  position: relative;
  /* ✅ 移除任何背景或边框，确保与频域一致 */
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
  height: 2rem;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.eeg-canvas {
  flex: 1;
  width: 100%; /* ✅ 确保Canvas占满容器宽度 */
  border: 2px solid #dee2e6;
  border-radius: 6px;
  background: #000000;
  display: block;
  box-shadow: 
    inset 0 2px 4px rgba(0, 0, 0, 0.1),
    0 0 20px rgba(102, 126, 234, 0.1);
  transition: box-shadow 0.3s ease;
  cursor: pointer;
}

/* 通道标签叠加层需要绝对定位，但要确保在正确的容器内 */
.channel-labels-overlay {
  position: absolute;
  left: 0;
  top: 3rem;
  bottom: 0;
  width: 80px;
  pointer-events: none;
  /* ✅ 确保z-index正确 */
  z-index: 10;
}

/* 性能统计面板调整位置避免遮挡 */
.performance-stats {
  position: absolute;
  top: 3rem; /* ✅ 调整位置避开标题 */
  right: 1rem;
  background: rgba(0, 0, 0, 0.8);
  color: #00ff00;
  padding: 0.5rem;
  border-radius: 4px;
  font-family: 'Monaco', 'Menlo', 'Consolas', monospace;
  font-size: 0.7rem;
  z-index: 15;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}

/* 通道标签样式保持不变 */
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