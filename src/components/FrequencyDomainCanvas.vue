<!-- filepath: src/components/FrequencyDomainCanvas.vue -->
<template>
  <div class="frequency-panel">
    <h3>实时频谱分析 (1-{{ maxFreq }}Hz) - 事件驱动WebGL</h3>
    <canvas 
      ref="spectrumCanvasRef" 
      class="spectrum-canvas"
      @click="handleCanvasClick"
    ></canvas>
    
    <!-- ✅ 性能统计：只在调试时显示，位置绝对定位不占用布局空间 -->
    <div class="performance-stats" v-if="showDebugInfo">
      <span>{{ Math.round(updateRate) }}Hz</span>
      <span>{{ avgLatency.toFixed(1) }}ms</span>
      <span>WebGL: {{ webglStatus }}</span>
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
  maxFreq?: number;
}

const props = withDefaults(defineProps<Props>(), {
  maxFreq: 50
});

// Emits
interface Emits {
  (e: 'update-frequency-rate', rate: number): void;
}

const emit = defineEmits<Emits>();

// ✅ 频域数据接口定义
interface FreqData {
  channel_index: number;
  spectrum: number[];
  frequency_bins: number[];
}

// Canvas相关
const spectrumCanvasRef = ref<HTMLCanvasElement | null>(null);
let wglp: WebglPlot | null = null;

// WebGL状态
const webglStatus = ref<string>('初始化中...');
const updateRate = ref(0);
const showDebugInfo = ref(false);

// 线条管理
const channelLines: WebglLine[] = [];
const FREQ_BINS = 50;
const MAX_AMPLITUDE = 100;

// ✅ 性能监控：事件驱动模式
//TODO: 升级成多线程离屏画布
let frameCount = 0;
let lastFrameTime = 0;
const avgLatency = ref(0);
let latencyHistory: number[] = [];
const MAX_LATENCY_SAMPLES = 10;

// 通道颜色配置
const channelColors = [
  new ColorRGBA(1.0, 0.42, 0.42, 1.0),  // #FF6B6B
  new ColorRGBA(0.31, 0.8, 0.77, 1.0),  // #4ECDC4
  new ColorRGBA(0.27, 0.72, 0.82, 1.0), // #45B7D1
  new ColorRGBA(0.59, 0.81, 0.71, 1.0), // #96CEB4
  new ColorRGBA(1.0, 0.92, 0.65, 1.0),  // #FFEAA7
  new ColorRGBA(0.87, 0.63, 0.87, 1.0), // #DDA0DD
  new ColorRGBA(0.6, 0.85, 0.91, 1.0),  // #98D8E8
  new ColorRGBA(0.97, 0.86, 0.44, 1.0), // #F7DC6F
];

// 初始化WebGL绘图器
function initWebGLPlot() {
  if (!spectrumCanvasRef.value) {
    console.warn('Canvas ref not available');
    return;
  }

  try {
    const canvas = spectrumCanvasRef.value;
    
    // 设置画布尺寸，考虑设备像素比
    const devicePixelRatio = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * devicePixelRatio;
    canvas.height = rect.height * devicePixelRatio;
    
    console.log(`📺 事件驱动频域WebGL Canvas: ${canvas.width}x${canvas.height}`);
    
    // 初始化WebGLplot
    wglp = new WebglPlot(canvas);
    
    webglStatus.value = '就绪';
    console.log('✅ 频域事件驱动WebGL初始化成功');
    
    // 初始化通道线条
    initChannelLines();
    
  } catch (error) {
    console.error('❌ 频域WebGL初始化失败:', error);
    webglStatus.value = '失败';
  }
}

// 初始化通道线条
function initChannelLines() {
  if (!wglp) return;
  
  console.log(`🎨 初始化 ${props.channelsCount} 个通道的频域线条`);
  
  // 清除现有线条
  wglp.removeAllLines();
  channelLines.length = 0;
  
  // 为每个通道创建线条
  for (let ch = 0; ch < props.channelsCount; ch++) {
    const color = channelColors[ch % channelColors.length];
    const line = new WebglLine(color, FREQ_BINS);
    
    // 初始化X轴：频率轴从-1到1，对应1-50Hz
    line.lineSpaceX(-1, 2 / FREQ_BINS);
    
    // 初始化Y轴：每个通道占用不同的Y区间
    const channelOffset = calculateChannelOffset(ch);
    
    // 初始化为基线（每个通道的底部）
    for (let i = 0; i < FREQ_BINS; i++) {
      line.setY(i, channelOffset);
    }
    
    wglp.addLine(line);
    channelLines.push(line);
  }
  
  console.log(`✅ 创建了 ${channelLines.length} 条频域线条`);
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
  if (props.channelsCount <= 1) return 0.8;
  
  const maxChannelHeight = (2 / props.channelsCount) * 0.8;
  return maxChannelHeight / 2;
}

// ✅ 核心功能：事件驱动的频域渲染
function handleFrameUpdate(event: any) {
  const startTime = performance.now();
  
  if (!wglp || channelLines.length === 0) {
    return;
  }
  
  const { frequency_domain } = event.payload;
  if (!frequency_domain || frequency_domain.length === 0) {
    return;
  }
  
  console.log(`🎵 直接处理 ${frequency_domain.length} 个通道的频域数据`);
  
  // ✅ 直接处理后端的频域数据
  updateSpectrumDirect(frequency_domain);
  
  // ✅ 一次性WebGL更新
  try {
    wglp.update();
  } catch (error) {
    console.error('频域WebGL更新错误:', error);
    return;
  }
  
  // 性能统计
  const endTime = performance.now();
  updatePerformanceStats(startTime, endTime);
}

// ✅ 直接更新频谱：核心渲染逻辑
function updateSpectrumDirect(spectrumData: FreqData[]) {
  const channelScale = calculateChannelScale();
  
  // 更新每个通道的频谱线条
  for (const freqData of spectrumData) {
    const ch = freqData.channel_index;
    
    // 检查通道索引有效性和可见性
    if (ch >= channelLines.length || ch >= props.channelsCount) {
      continue;
    }
    
    const line = channelLines[ch];
    const channelOffset = calculateChannelOffset(ch);
    const spectrum = freqData.spectrum;
    
    // 处理可见性
    if (!props.channelVisibility[ch]) {
      // 不可见通道：设置为基线
      for (let i = 0; i < FREQ_BINS; i++) {
        line.setY(i, channelOffset);
      }
      continue;
    }
    
    // 更新线条颜色（选中状态）
    updateLineColor(line, ch);
    
    // 更新频谱数据点
    const dataLength = Math.min(spectrum.length, FREQ_BINS);
    
    for (let i = 0; i < FREQ_BINS; i++) {
      let magnitude = 0;
      
      if (i < dataLength) {
        magnitude = Math.min(spectrum[i] / MAX_AMPLITUDE, 1.0);
        magnitude = Math.max(magnitude, 0.0);
      }
      
      const y = channelOffset + magnitude * channelScale;
      line.setY(i, y);
    }
  }
}

// ✅ 颜色更新优化
function updateLineColor(line: WebglLine, channelIndex: number) {
  const isSelected = props.selectedChannels.has(channelIndex);
  const baseColor = channelColors[channelIndex % channelColors.length];
  
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
    updateRate.value = frameCount;
    avgLatency.value = latencyHistory.reduce((a, b) => a + b, 0) / latencyHistory.length;
    
    emit('update-frequency-rate', updateRate.value);
    
    console.log(`📊 频域统计: ${updateRate.value}Hz, 平均延迟: ${avgLatency.value.toFixed(1)}ms`);
    
    frameCount = 0;
    lastFrameTime = now;
  }
}

// 清除所有频谱数据
function clearSpectrum() {
  if (!wglp || channelLines.length === 0) return;
  
  for (let ch = 0; ch < channelLines.length; ch++) {
    const line = channelLines[ch];
    const channelOffset = calculateChannelOffset(ch);
    
    for (let i = 0; i < FREQ_BINS; i++) {
      line.setY(i, channelOffset);
    }
  }
  
  wglp.update();
}

// 事件处理
function handleCanvasClick() {
  showDebugInfo.value = !showDebugInfo.value;
}

// ✅ 简化的公共方法
function initCanvas() {
  initWebGLPlot();
}

// 窗口大小变化处理
function handleResize() {
  if (spectrumCanvasRef.value && wglp) {
    const canvas = spectrumCanvasRef.value;
    const devicePixelRatio = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    
    canvas.width = rect.width * devicePixelRatio;
    canvas.height = rect.height * devicePixelRatio;
    
    initWebGLPlot();
  }
}

// 监听器
watch(() => props.channelsCount, () => {
  console.log(`📊 频域通道数变化: ${props.channelsCount}`);
  if (wglp && props.channelsCount > 0) {
    initChannelLines();
  }
}, { immediate: true });

watch(() => props.channelVisibility, () => {
  // 可见性变化时无需重新渲染，下次数据到达时自然处理
}, { deep: true });

watch(() => props.selectedChannels, () => {
  // 选中状态变化时无需重新渲染，下次数据到达时自然处理
}, { deep: true });

// ✅ 生命周期：事件驱动模式
onMounted(async () => {
  await nextTick();
  initWebGLPlot();
  
  // ✅ 关键：监听后端frame-update事件，专注频域数据
  const unlistenFrameUpdate = await listen('frame-update', handleFrameUpdate);
  
  // 保存取消监听的函数
  onUnmounted(() => {
    unlistenFrameUpdate();
  });
  
  window.addEventListener('resize', handleResize);
  console.log('🎧 频域事件监听器已设置，等待后端频域数据...');
});

onUnmounted(() => {
  // 清理WebGL资源
  if (wglp) {
    wglp.removeAllLines();
    channelLines.length = 0;
    wglp = null;
  }
  
  window.removeEventListener('resize', handleResize);
  console.log('🧹 事件驱动频域WebGL画布已清理');
});

// ✅ 大幅简化的暴露方法
defineExpose({
  initCanvas,
  clearSpectrum
});
</script>

<style scoped>
.frequency-panel {
  /* ✅ 移除圆角矩形样式，与时域保持一致 */
  height: 100%;
  width: 100%;
  display: flex;
  flex-direction: column;
  position: relative;
  
  background: #f8f9fa;
  border-radius: 8px;
  padding: 1rem;
  border: 2px solid #e9ecef;
  box-shadow: 0 4px 15px rgba(0, 0, 0, 0.1);
 
  
  /* ✅ 保留基本布局属性 */
  box-sizing: border-box;
  overflow: hidden;
}

.frequency-panel h3 {
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

.spectrum-canvas {
  flex: 1;
  width: 100%;
  border: 2px solid #dee2e6;
  border-radius: 6px;
  background: #000000;
  display: block;
  box-shadow: 
    inset 0 2px 4px rgba(0, 0, 0, 0.1),
    0 0 20px rgba(102, 126, 234, 0.1);
  transition: box-shadow 0.3s ease;
  cursor: pointer;
  box-sizing: border-box;
}

.spectrum-canvas:hover {
  box-shadow: 
    inset 0 2px 4px rgba(0, 0, 0, 0.1),
    0 0 25px rgba(102, 126, 234, 0.2);
}

/* 性能统计：绝对定位，不影响布局 */
.performance-stats {
  position: absolute;
  top: 3rem;
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
  pointer-events: none;
}

/* WebGL动画保持不变 */
@keyframes webgl-pulse {
  0%, 100% { 
    box-shadow: 0 0 20px rgba(102, 126, 234, 0.1); 
  }
  50% { 
    box-shadow: 0 0 30px rgba(102, 126, 234, 0.3); 
  }
}

.spectrum-canvas {
  animation: webgl-pulse 3s ease-in-out infinite;
}

/* 响应式调整 */
@media (max-width: 1200px) {
  .frequency-panel {
    height: auto;
    min-height: 400px;
  }
}

@media (max-width: 768px) {
  .frequency-panel {
    height: auto;
    min-height: 350px;
  }
}
</style>