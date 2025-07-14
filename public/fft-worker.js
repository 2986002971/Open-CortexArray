/**
 * Web Worker for FFT computation
 * 在独立线程中处理FFT计算，避免阻塞UI主线程
 */

// 简单的FFT实现（可以后续替换为更高效的库）
class FFT {
    constructor(size) {
        this.size = size;
        this.log2Size = Math.log2(size);
        if (this.log2Size % 1 !== 0) {
            throw new Error('FFT size must be a power of 2');
        }
        
        // 预计算旋转因子
        this.w = new Array(size / 2);
        for (let i = 0; i < size / 2; i++) {
            const angle = -2 * Math.PI * i / size;
            this.w[i] = { real: Math.cos(angle), imag: Math.sin(angle) };
        }
    }
    
    // 简化的FFT实现（仅用于演示，实际项目中应使用优化版本）
    compute(realData) {
        const size = this.size;
        const complexData = realData.map((real, i) => ({ real, imag: 0 }));
        
        // 位逆序排列
        this.bitReversePermute(complexData);
        
        // 蝶形运算
        for (let len = 2; len <= size; len *= 2) {
            for (let i = 0; i < size; i += len) {
                for (let j = 0; j < len / 2; j++) {
                    const u = complexData[i + j];
                    const v = this.complexMultiply(
                        complexData[i + j + len / 2],
                        this.w[j * size / len]
                    );
                    
                    complexData[i + j] = this.complexAdd(u, v);
                    complexData[i + j + len / 2] = this.complexSubtract(u, v);
                }
            }
        }
        
        // 返回幅度谱
        return complexData.map(c => Math.sqrt(c.real * c.real + c.imag * c.imag));
    }
    
    bitReversePermute(data) {
        const size = data.length;
        for (let i = 1, j = 0; i < size; i++) {
            let bit = size >> 1;
            for (; j & bit; bit >>= 1) {
                j ^= bit;
            }
            j ^= bit;
            
            if (i < j) {
                [data[i], data[j]] = [data[j], data[i]];
            }
        }
    }
    
    complexMultiply(a, b) {
        return {
            real: a.real * b.real - a.imag * b.imag,
            imag: a.real * b.imag + a.imag * b.real
        };
    }
    
    complexAdd(a, b) {
        return {
            real: a.real + b.real,
            imag: a.imag + b.imag
        };
    }
    
    complexSubtract(a, b) {
        return {
            real: a.real - b.real,
            imag: a.imag - b.imag
        };
    }
}

let fft = null;

self.onmessage = function(e) {
    const { type, data } = e.data;
    
    switch (type) {
        case 'init':
            const { fftSize } = data;
            fft = new FFT(fftSize);
            self.postMessage({ type: 'init-complete' });
            break;
            
        case 'compute':
            if (!fft) {
                self.postMessage({ 
                    type: 'error', 
                    message: 'FFT not initialized' 
                });
                return;
            }
            
            const { channelData, channelIndex, timestamp } = data;
            
            try {
                const spectrum = fft.compute(channelData);
                self.postMessage({
                    type: 'spectrum',
                    data: {
                        channelIndex,
                        spectrum,
                        timestamp
                    }
                });
            } catch (error) {
                self.postMessage({
                    type: 'error',
                    message: error.message
                });
            }
            break;
            
        default:
            self.postMessage({
                type: 'error',
                message: `Unknown message type: ${type}`
            });
    }
};
