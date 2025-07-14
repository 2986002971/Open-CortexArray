<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// 基本状态
const isConnected = ref(false);
const isRecording = ref(false);
const streams = ref<any[]>([]);
const selectedStream = ref("");
const recordingFilename = ref("");

// 基本功能函数
async function discoverStreams() {
  try {
    console.log("Discovering LSL streams...");
    const result = await invoke('discover_lsl_streams');
    streams.value = result as any[];
    console.log("Found streams:", streams.value);
  } catch (error) {
    console.error('Failed to discover streams:', error);
  }
}

async function connectToStream() {
  if (!selectedStream.value) {
    alert("Please select a stream first");
    return;
  }
  
  try {
    console.log("Connecting to stream:", selectedStream.value);
    await invoke('connect_to_stream', { streamName: selectedStream.value });
    isConnected.value = true;
    console.log("Connected successfully");
  } catch (error) {
    console.error('Failed to connect to stream:', error);
  }
}

async function disconnect() {
  try {
    await invoke('disconnect_stream');
    isConnected.value = false;
    console.log("Disconnected");
  } catch (error) {
    console.error('Failed to disconnect:', error);
  }
}

async function startRecording() {
  if (!recordingFilename.value) {
    recordingFilename.value = `eeg_recording_${Date.now()}.edf`;
  }
  
  try {
    await invoke('start_recording', { filename: recordingFilename.value });
    isRecording.value = true;
    console.log("Recording started");
  } catch (error) {
    console.error('Failed to start recording:', error);
  }
}

async function stopRecording() {
  try {
    await invoke('stop_recording');
    isRecording.value = false;
    console.log("Recording stopped");
  } catch (error) {
    console.error('Failed to stop recording:', error);
  }
}

onMounted(async () => {
  console.log("App mounted");
  
  // 监听EEG数据
  const unlisten = await listen('eeg-data', (event) => {
    console.log("Received EEG data:", event.payload);
  });
});
</script>

<template>
  <div class="app">
    <h1>Open CortexArray - EEG可视化系统 (测试版)</h1>
    
    <div class="controls">
      <div class="section">
        <h3>LSL流发现</h3>
        <button @click="discoverStreams">发现LSL流</button>
        <p>找到 {{ streams.length }} 个流</p>
        
        <div v-if="streams.length > 0">
          <label>选择流:</label>
          <select v-model="selectedStream">
            <option value="">-- 请选择流 --</option>
            <option v-for="stream in streams" :key="stream.source_id" :value="stream.name">
              {{ stream.name }} ({{ stream.channels_count }}通道, {{ stream.sample_rate }}Hz)
            </option>
          </select>
        </div>
      </div>
      
      <div class="section">
        <h3>连接控制</h3>
        <button @click="connectToStream" :disabled="!selectedStream || isConnected">
          {{ isConnected ? '已连接' : '连接' }}
        </button>
        <button @click="disconnect" :disabled="!isConnected">断开连接</button>
        <p>状态: {{ isConnected ? '已连接' : '未连接' }}</p>
      </div>
      
      <div class="section">
        <h3>录制控制</h3>
        <input v-model="recordingFilename" placeholder="录制文件名.edf" :disabled="isRecording">
        <br>
        <button @click="startRecording" :disabled="!isConnected || isRecording">开始录制</button>
        <button @click="stopRecording" :disabled="!isRecording">停止录制</button>
        <p>录制状态: {{ isRecording ? '录制中...' : '未录制' }}</p>
      </div>
    </div>
    
    <div class="info">
      <h3>调试信息</h3>
      <p>请打开浏览器开发者工具查看控制台输出</p>
    </div>
  </div>
</template>

<style>
.app {
  font-family: Arial, sans-serif;
  max-width: 800px;
  margin: 0 auto;
  padding: 20px;
}

.controls {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.section {
  border: 1px solid #ccc;
  padding: 15px;
  border-radius: 5px;
}

button {
  padding: 8px 16px;
  margin: 5px;
  background: #007bff;
  color: white;
  border: none;
  border-radius: 4px;
  cursor: pointer;
}

button:disabled {
  background: #ccc;
  cursor: not-allowed;
}

input, select {
  padding: 5px;
  margin: 5px;
  border: 1px solid #ccc;
  border-radius: 3px;
}

.info {
  margin-top: 30px;
  padding: 15px;
  background: #f5f5f5;
  border-radius: 5px;
}
</style>
