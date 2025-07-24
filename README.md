# Open CortexArray Documentation

---

## Introduction

**Open CortexArray** is a high-performance, modern multi-channel EEG visualization and recording system. It supports real-time LSL stream acquisition, independent time/frequency domain rendering canvases, an extremely optimized data pipeline, and one-click EDF file recording. The system is built with Rust + Vue3 + Tauri, balancing performance, maintainability, and user experience.

---

## Architecture Overview

- **App.vue**: Handles UI management, connection/recording controls, and channel visibility. Does not process sample streams directly.
- **TimeDomainCanvas.vue**: Independently listens to the `binary-frame-update` event, focusing on high-performance time-domain waveform rendering.
- **FrequencyDomainCanvas.vue**: Independently listens to the `frequency-update` event, focusing on frequency spectrum analysis and display.
- **Backend (Rust)**: LSL acquisition → data distribution → recording thread/time-domain collector/FFT processing → frontend event emission.

---

## Main Features

- **Automatic LSL stream discovery and connection**, supporting multi-channel, high-sampling-rate EEG data.
- **Real-time time-domain waveform rendering**, with channel selection, hiding, highlighting, and wavefront indication.
- **Frequency domain analysis (FFT)**, supporting multi-channel spectrum display and customizable max frequency.
- **High-performance data pipeline**, with optimized backend data layout + SIMD, and zero-copy ArrayBuffer parsing on the frontend.
- **One-click recording**, supporting EDF file saving.
- **User-friendly UI**, supporting multi-channel selection, hover, recording status indication, etc.
- **Performance monitoring**, each canvas can display its own rendering rate by clicking.

---

## Data Flow & Event Interfaces

### 1. Time Domain Data (High-Performance Binary)

- **Event name**: `binary-frame-update`
- **Content**: `Uint8Array` (ArrayBuffer)
- **Format**:

| Field                | Type         | Length    | Description            |
|----------------------|--------------|-----------|------------------------|
| batch_id             | u64          | 8 bytes   | Batch number           |
| timestamp            | f64          | 8 bytes   | Timestamp (seconds)    |
| channels_count       | u32          | 4 bytes   | Number of channels     |
| samples_per_channel  | u32          | 4 bytes   | Samples per channel    |
| sample_rate          | f64          | 8 bytes   | Sampling rate          |
| ...per channel       |              |           |                        |
| channel_index        | u32          | 4 bytes   | Channel index          |
| samples              | f32[]        | 4*N bytes | Continuous samples     |

- **Frontend parsing suggestion**:  
  Use DataView/Float32Array, see `binaryParser.ts`.

### 2. Frequency Domain Data (JSON)

- **Event name**: `frequency-update`
- **Content**: `FreqData[]`
- **Format**:

```typescript
interface FreqData {
  channel_index: number;
  spectrum: number[];        // Power spectrum
  frequency_bins: number[];  // Frequency axis
}
```

---

## Typical Usage Flow

1. **Discover LSL Streams**  
   Click "Discover LSL Streams" and select the target stream.

2. **Connect to Stream**  
   Click "Connect to Stream". App.vue will automatically fetch stream parameters and initialize canvases.

3. **Real-Time Visualization**  
   - Time domain: TimeDomainCanvas independently listens to `binary-frame-update` for high-performance rendering.
   - Frequency domain: FrequencyDomainCanvas independently listens to `frequency-update` for real-time spectrum display.

4. **Channel Operations**  
   Supports channel hiding, selection, and hover highlighting, directly reflected on the canvases.

5. **Record Data**  
   Enter a filename and click "Start Recording" to save as a standard EDF file.

6. **Performance Monitoring**  
   Click any canvas to display its current frame rate, latency, etc.

---

## Performance Highlights

- **Rust backend**: Cache-friendly data layout, SIMD acceleration, extreme throughput.
- **Frontend**: Zero-copy ArrayBuffer parsing, batch WebGL rendering, independent canvas listeners, no main thread blocking.
- **Event decoupling**: Time/frequency domain separation, no interference, easy to extend and maintain.

---

## Code Structure Overview

- App.vue: UI, connection/recording/channel control
- TimeDomainCanvas.vue: Time-domain rendering and event listening
- FrequencyDomainCanvas.vue: Frequency-domain analysis and event listening
- binaryParser.ts: Binary data parsing utilities
- data_types.rs: Core data structures and binary format definitions
- eeg_processor.rs: Data pipeline, event emission, recording, and FFT processing

---

## Binary Frame Parsing Example (Frontend)

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
    // Can be rendered in batch
  }
});
```

---

## TODO

- **Channel Highlighting and Hiding Optimization**  
  - Optimize channel highlighting and hiding logic in both time and frequency domain canvases for better interaction and rendering efficiency.
  - Support more flexible channel highlighting/hiding, batch operations, and visual feedback.

- **Add More Automated Tests**  
  - Add frontend unit and integration tests for binary parsing and rendering.
  - Add more backend Rust tests for the data pipeline, format conversion, and error handling to improve robustness.

- **Manual Scale Control**  
  - Allow users to manually adjust the vertical scale (zoom/drag) on time/frequency canvases to suit different signal amplitudes and analysis needs.
  - Add UI controls for scale adjustment, supporting both auto and manual modes.

---

## FAQ

**Q: How to extend new visualizations?**  
A: Add a new component that listens to its own event; no need to modify App.vue.

**Q: Is the recording thread affected by the optimizations?**  
A: No, recording is always based on the original data stream, ensuring data integrity.

---

## Acknowledgements

Thanks to all contributors and testers!  
For questions or suggestions, please submit an Issue or PR on GitHub.

---

**ISI Lab**  
July 2025