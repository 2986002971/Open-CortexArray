<template>
  <div class="frequency-panel">
    <h3>实时频谱分析 (1-{{ maxFreq }}Hz)</h3>
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
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, watch, nextTick } from 'vue';

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

// 渲染常量
const SPECTRUM_WIDTH = 400;
const SPECTRUM_HEIGHT = 600;

// Canvas相关
const spectrumCanvasRef = ref<HTMLCanvasElement | null>(null);
let spectrumCtx: CanvasRenderingContext2D | null = null;

// 性能监控
const updateRate = ref(0);
let lastFrequencyUpdate = 0;
let lastFreqRenderTime = 0;
const FREQ_RENDER_INTERVAL = 1000 / 30; // 30Hz限制

// 通道颜色
const channelColors = ['#FF6B6B', '#4ECDC4', '#45B7D1', '#96CEB4', '#FFEAA7', '#DDA0DD', '#98D8E8', '#F7DC6F'];

// 初始化函数
function initSpectrumCanvas() {
  if (!spectrumCanvasRef.value) return;
  
  const canvas = spectrumCanvasRef.value;
  canvas.width = SPECTRUM_WIDTH;
  canvas.height = SPECTRUM_HEIGHT;
  
  spectrumCtx = canvas.getContext('2d');
  
  if (spectrumCtx) {
    drawSpectrumGrid();
  }
}

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
  if (props.channelsCount > 0) {
    const channelHeight = SPECTRUM_HEIGHT / props.channelsCount;
    for (let ch = 0; ch <= props.channelsCount; ch++) {
      const y = channelHeight * ch;
      spectrumCtx.beginPath();
      spectrumCtx.moveTo(0, y);
      spectrumCtx.lineTo(SPECTRUM_WIDTH, y);
      spectrumCtx.stroke();
    }
  }
  
  spectrumCtx.restore();
}

function drawSpectrumFromBackend() {
  const now = Date.now();
  
  // 频域更新节流控制
  if (now - lastFreqRenderTime < FREQ_RENDER_INTERVAL) {
    return;
  }
  lastFreqRenderTime = now;
  
  if (!spectrumCtx || props.channelsCount <= 0 || props.spectrumData.length === 0) return;
  
  // 更新频域更新率显示
  const deltaTime = now - lastFrequencyUpdate;
  if (deltaTime > 0) {
    updateRate.value = 1000 / deltaTime;
    emit('update-frequency-rate', updateRate.value);
  }
  lastFrequencyUpdate = now;
  
  // 重绘背景
  drawSpectrumGrid();
  
  const channelHeight = SPECTRUM_HEIGHT / props.channelsCount;
  
  // 绘制每个通道的频谱
  for (const freqData of props.spectrumData) {
    const ch = freqData.channel_index;
    
    if (ch >= props.channelsCount || !props.channelVisibility[ch]) continue;
    
    const channelY = ch * channelHeight;
    const isSelected = props.selectedChannels.has(ch);
    
    spectrumCtx.strokeStyle = channelColors[ch % channelColors.length];
    spectrumCtx.lineWidth = isSelected ? 2.5 : 1.5;
    spectrumCtx.fillStyle = channelColors[ch % channelColors.length] + '20';
    
    const spectrum = freqData.spectrum;
    const freqBinWidth = SPECTRUM_WIDTH / spectrum.length;
    
    spectrumCtx.beginPath();
    spectrumCtx.moveTo(0, channelY + channelHeight);
    
    // 绘制频谱曲线
    for (let i = 0; i < spectrum.length; i++) {
      const magnitude = Math.min(spectrum[i] / 50, 1);
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

// 监听器
watch(() => props.spectrumData, () => {
  drawSpectrumFromBackend();
}, { deep: true });

watch(() => props.channelsCount, () => {
  initSpectrumCanvas();
}, { immediate: true });

watch(() => props.channelVisibility, () => {
  drawSpectrumFromBackend();
}, { deep: true });

watch(() => props.selectedChannels, () => {
  drawSpectrumFromBackend();
}, { deep: true });

// 生命周期
onMounted(async () => {
  await nextTick();
  initSpectrumCanvas();
});

// 暴露方法给父组件
defineExpose({
  drawSpectrumFromBackend,
  initSpectrumCanvas
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
}

.frequency-panel h3 {
  margin: 0 0 1rem 0;
  font-size: 1.1rem;
  font-weight: 600;
  color: #495057;
  text-align: center;
}

.spectrum-canvas {
  flex: 1;
  border: 2px solid #dee2e6;
  border-radius: 6px;
  background: #ffffff;
  display: block;
  box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.05);
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
}

.frequency-status {
  text-align: center;
  margin-top: 0.5rem;
}

.update-rate {
  font-size: 0.8rem;
  color: #28a745;
  font-weight: 600;
  background: rgba(40, 167, 69, 0.1);
  padding: 0.2rem 0.5rem;
  border-radius: 12px;
}
</style>