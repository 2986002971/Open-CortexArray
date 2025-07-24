# Open CortexArray 说明文档

---

## 简介

**Open CortexArray** 是一款高性能、现代化的多通道脑电（EEG）可视化与录制系统，支持 LSL 流实时采集、时域/频域双画布独立渲染、极致性能优化的数据管道，以及一键录制 EDF 文件。系统采用 Rust + Vue3 + Tauri 技术栈，兼顾了性能、可维护性与用户体验。

---

## 架构总览

- **App.vue**：负责 UI 管理、连接/录制控制、通道可见性等交互，不直接处理样本流。
- **TimeDomainCanvas.vue**：独立监听 `binary-frame-update` 事件，专注高性能时域波形渲染。
- **FrequencyDomainCanvas.vue**：独立监听 `frequency-update` 事件，专注频谱分析与显示。
- **后端（Rust）**：LSL采集 → 数据分发 → 录制线程/时域收集器/FFT处理 → 前端事件推送。

---

## 主要功能

- **LSL流自动发现与连接**，支持多通道高采样率EEG数据。
- **时域波形实时渲染**，支持通道选择、隐藏、选中高亮、波前指示。
- **频域分析（FFT）**，支持多通道频谱显示，最大频率可自定义。
- **高性能数据管道**，后端采用经过优化的数据布局+SIMD优化，前端用ArrayBuffer零拷贝解析。
- **一键录制**，支持EDF格式文件保存。
- **UI交互友好**，支持通道多选、hover、录制状态提示等。
- **性能监控**，各画布可点击显示独立渲染速率。

---

## 数据流与事件接口

### 1. 时域数据（高性能二进制）

- **事件名**：`binary-frame-update`
- **内容**：`Uint8Array`（ArrayBuffer）
- **格式说明**：

| 字段                | 类型         | 长度      | 说明                   |
|---------------------|--------------|-----------|------------------------|
| batch_id            | u64          | 8 bytes   | 批次编号               |
| timestamp           | f64          | 8 bytes   | 时间戳（秒）           |
| channels_count      | u32          | 4 bytes   | 通道数                 |
| samples_per_channel | u32          | 4 bytes   | 每通道样本数           |
| sample_rate         | f64          | 8 bytes   | 采样率                 |
| ...每个通道         |              |           |                        |
| channel_index       | u32          | 4 bytes   | 通道索引               |
| samples             | f32[]        | 4*N bytes | 连续样本数据           |

- **前端解析建议**：  
  使用 DataView/Float32Array 解析，参考 binaryParser.ts。

### 2. 频域数据（JSON）

- **事件名**：`frequency-update`
- **内容**：`FreqData[]`
- **格式说明**：

```typescript
interface FreqData {
  channel_index: number;
  spectrum: number[];        // 功率谱
  frequency_bins: number[];  // 频率轴
}
```

---

## 典型使用流程

1. **发现LSL流**  
   点击“发现LSL流”按钮，选择目标流。

2. **连接到流**  
   点击“连接到流”，App.vue 会自动获取流参数并初始化画布。

3. **实时可视化**  
   - 时域波形：TimeDomainCanvas 独立监听 `binary-frame-update`，高性能渲染。
   - 频域分析：FrequencyDomainCanvas 独立监听 `frequency-update`，实时显示频谱。

4. **通道操作**  
   支持通道隐藏、选中、hover高亮，操作直接反映在画布上。

5. **录制数据**  
   输入文件名，点击“开始录制”，可保存为标准EDF文件。

6. **性能监控**  
   点击任一画布可显示当前帧率、延迟等性能信息。

---

## 性能优化亮点

- **Rust后端**：cache友好的数据布局，SIMD加速，极致吞吐。
- **前端**：ArrayBuffer零拷贝解析，WebGL批量渲染，画布独立监听，不卡主线程。
- **事件解耦**：时域/频域分离，互不干扰，便于扩展和维护。

---

## 代码结构简述

- App.vue：UI与连接/录制/通道控制
- TimeDomainCanvas.vue：时域波形渲染与事件监听
- FrequencyDomainCanvas.vue：频域分析与事件监听
- binaryParser.ts：二进制数据解析工具
- data_types.rs：核心数据结构与二进制格式定义
- eeg_processor.rs：数据管道、事件推送、录制与FFT处理

---

## 二进制帧解析示例（前端）

```typescript
import { BatchedBinaryParser } from '@/utils/binaryParser';

const parser = new BatchedBinaryParser();

listen('binary-frame-update', (event) => {
  const binaryArray = event.payload as number[];
  const buffer = new Uint8Array(binaryArray).buffer;
  const parsed = parser.parseForTimeRendering(buffer);
  if (parsed) {
    // parsed.channelData: Array<{ channel_index, samples: Float32Array }>
    // parsed.metadata: { batch_id, timestamp, ... }
    // 可直接批量渲染
  }
});
```

---

## TODO

- **通道高亮与隐藏的优化**  
  - 优化时域和频域画布的通道高亮、隐藏逻辑，提升交互体验和渲染效率。
  - 支持更灵活的通道高亮与隐藏、批量操作和视觉反馈。

- **添加更多自动化测试**  
  - 增加前端二进制解析、渲染流程的单元测试和集成测试。
  - 后端 Rust 端补充数据管道、格式转换、异常处理等测试用例，提升系统健壮性。

- **比例尺的手动控制**  
  - 支持用户在时域/频域画布上手动调整纵轴比例尺（缩放/拖拽），适应不同信号幅值和分析需求。
  - UI上增加比例尺调节控件，支持自动与手动切换

---

## FAQ

**Q: 如何扩展新的可视化？**  
A: 新增组件独立监听自己的事件即可，无需修改App.vue。

**Q: 录制线程是否受优化影响？**  
A: 不受影响，录制始终基于原始数据流，保证数据完整性。

---

## 致谢

感谢所有贡献者和测试者！  
如有问题或建议，欢迎在 GitHub 提 Issue 或 PR。

---

**ISI Lab**  
2025年7月