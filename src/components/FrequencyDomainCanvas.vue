<!-- filepath: src/components/FrequencyDomainCanvas.vue -->
<template>
  <div class="frequency-panel">
    <h3>实时频谱分析 (1-{{ maxFreq }}Hz) - WebGL加速</h3>
    <canvas 
      ref="spectrumCanvasRef" 
      class="spectrum-canvas"
      :style="{ width: '100%', height: '400px' }"
    ></canvas>
    <div class="frequency-legend">
      <div class="freq-range">1Hz</div>
      <div class="freq-range">{{ Math.round(maxFreq / 2) }}Hz</div>
      <div class="freq-range">{{ maxFreq }}Hz</div>
    </div>
    <div class="frequency-status">
      <span class="update-rate">{{ Math.round(updateRate) }}Hz 更新</span>
      <span class="webgl-status">WebGL: {{ webglStatus }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch, nextTick } from 'vue';
// ✅ 修复：正确的类名大小写
import { WebglPlot, WebglLine, ColorRGBA } from 'webgl-plot';

// Props
interface Props {
  channelsCount: number;
  sampleRate: number;
  channelVisibility: boolean[];
  selectedChannels: Set<number>;
  spectrumData: FreqData[];
  maxFreq?: number;
}

interface FreqData {
  channel_index: number;
  spectrum: number[];
  frequency_bins: number[];
}

const props = withDefaults(defineProps<Props>(), {
  maxFreq: 50
});

// Emits
interface Emits {
  (e: 'update-frequency-rate', rate: number): void;
}

const emit = defineEmits<Emits>();

// Canvas相关
const spectrumCanvasRef = ref<HTMLCanvasElement | null>(null);
let wglp: WebglPlot | null = null;

// WebGL状态
const webglStatus = ref<string>('初始化中...');
const updateRate = ref(0);

// 线条管理
const channelLines: WebglLine[] = [];
const FREQ_BINS = 50;
const MAX_AMPLITUDE = 100;

// 性能监控
let lastFrequencyUpdate = 0;
let lastFrameTime = 0;
let frameCount = 0;

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
    
    console.log(`WebGL Canvas: ${canvas.width}x${canvas.height}, DPR: ${devicePixelRatio}`);
    
    // ✅ 修复实例化：WebglPlot（注意大小写）
    wglp = new WebglPlot(canvas);
    
    // 清空现有线条
    channelLines.length = 0;
    
    webglStatus.value = '就绪';
    console.log('✅ WebGL初始化成功');
    
    // 初始化通道线条
    initChannelLines();
    
  } catch (error) {
    console.error('❌ WebGL初始化失败:', error);
    webglStatus.value = '失败';
  }
}

// 初始化通道线条
function initChannelLines() {
  if (!wglp) return;
  
  console.log(`🎨 初始化 ${props.channelsCount} 个通道的频域线条`);
  
  // ✅ 修复：使用 removeAllLines() 而不是 removeLine()
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
    
    // ✅ 使用 addLine() 方法（这是 addDataLine 的别名）
    wglp.addLine(line);
    channelLines.push(line);
    
    console.log(`📈 通道 ${ch + 1}: 颜色=${color.r.toFixed(2)},${color.g.toFixed(2)},${color.b.toFixed(2)}, 偏移=${channelOffset.toFixed(3)}`);
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

// 更新频谱数据
function updateSpectrumData() {
  if (!wglp || channelLines.length === 0 || props.spectrumData.length === 0) {
    return;
  }
  
  const now = Date.now();
  frameCount++;
  
  // 性能监控
  if (now - lastFrameTime >= 1000) {
    updateRate.value = frameCount;
    emit('update-frequency-rate', updateRate.value);
    frameCount = 0;
    lastFrameTime = now;
  }
  
  const channelScale = calculateChannelScale();
  
  // 更新每个通道的频谱线条
  for (const freqData of props.spectrumData) {
    const ch = freqData.channel_index;
    
    // 检查通道索引有效性和可见性
    if (ch >= channelLines.length || ch >= props.channelsCount || !props.channelVisibility[ch]) {
      continue;
    }
    
    const line = channelLines[ch];
    const channelOffset = calculateChannelOffset(ch);
    const spectrum = freqData.spectrum;
    
    // 更新线条颜色（如果选中则加强显示）
    const isSelected = props.selectedChannels.has(ch);
    const baseColor = channelColors[ch % channelColors.length];
    
    if (isSelected) {
      line.color = new ColorRGBA(
        Math.min(baseColor.r * 1.2, 1.0),
        Math.min(baseColor.g * 1.2, 1.0), 
        Math.min(baseColor.b * 1.2, 1.0),
        1.0
      );
    } else {
      line.color = baseColor;
    }
    
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
  
  // 处理不可见的通道
  for (let ch = 0; ch < channelLines.length; ch++) {
    if (!props.channelVisibility[ch]) {
      const line = channelLines[ch];
      const channelOffset = calculateChannelOffset(ch);
      
      for (let i = 0; i < FREQ_BINS; i++) {
        line.setY(i, channelOffset);
      }
    }
  }
  
  // 更新WebGL绘图
  try {
    wglp.update();
  } catch (error) {
    console.error('WebGL更新错误:', error);
    webglStatus.value = '更新错误';
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

// 监听器
watch(() => props.spectrumData, () => {
  updateSpectrumData();
}, { deep: true });

watch(() => props.channelsCount, () => {
  console.log(`📊 通道数变化: ${props.channelsCount}`);
  if (wglp && props.channelsCount > 0) {
    initChannelLines();
  }
}, { immediate: false });

watch(() => props.channelVisibility, () => {
  updateSpectrumData();
}, { deep: true });

watch(() => props.selectedChannels, () => {
  updateSpectrumData();
}, { deep: true });

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

// ✅ 修复生命周期问题
onMounted(async () => {
  await nextTick();
  initWebGLPlot();
  window.addEventListener('resize', handleResize);
});

onUnmounted(() => {
  // ✅ 修复：清理WebGL资源
  if (wglp) {
    wglp.removeAllLines();  // 使用正确的方法
    channelLines.length = 0;
    wglp = null;
  }
  
  window.removeEventListener('resize', handleResize);
  console.log('🧹 WebGL频域画布已清理');
});

// 暴露方法给父组件
defineExpose({
  updateSpectrumData,
  clearSpectrum,
  initWebGLPlot,
  initChannelLines
});
</script>

<style scoped>
.frequency-panel {
  flex: 0 0 33%;
  display: flex;
  flex-direction: column;
  background: #f8f9fa;
  border-radius: 8px;
  padding: 1rem;
  border: 2px solid #e9ecef;
  box-shadow: 0 4px 15px rgba(0, 0, 0, 0.1);
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
}

.spectrum-canvas {
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

.spectrum-canvas:hover {
  box-shadow: 
    inset 0 2px 4px rgba(0, 0, 0, 0.1),
    0 0 25px rgba(102, 126, 234, 0.2);
}

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
  background: rgba(108, 117, 125, 0.1);
  padding: 0.2rem 0.4rem;
  border-radius: 8px;
  border: 1px solid rgba(108, 117, 125, 0.2);
}

.frequency-status {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 0.5rem;
  padding: 0.3rem;
  background: rgba(255, 255, 255, 0.8);
  border-radius: 6px;
  font-size: 0.75rem;
}

.update-rate {
  color: #28a745;
  font-weight: 600;
  background: rgba(40, 167, 69, 0.1);
  padding: 0.2rem 0.5rem;
  border-radius: 12px;
  border: 1px solid rgba(40, 167, 69, 0.2);
}

.webgl-status {
  color: #007bff;
  font-weight: 600;
  background: rgba(0, 123, 255, 0.1);
  padding: 0.2rem 0.5rem;
  border-radius: 12px;
  border: 1px solid rgba(0, 123, 255, 0.2);
}

/* WebGL加速指示动画 */
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
    flex: 0 0 40%;
  }
}

@media (max-width: 768px) {
  .frequency-panel {
    flex: 1 1 100%;
    margin-top: 1rem;
  }
  
  .frequency-status {
    flex-direction: column;
    gap: 0.3rem;
  }
}
</style>