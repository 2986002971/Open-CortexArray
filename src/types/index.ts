export interface StreamInfo {
  name: string;
  stream_type: string;
  channels_count: number;
  sample_rate: number;
  is_connected: boolean;
  source_id: string;
}

export interface LslStreamInfo {
  name: string;
  stream_type: string;
  channels_count: number;
  sample_rate: number;
  source_id: string;
  hostname: string;
}

export interface FreqData {
  channel_index: number;
  spectrum: number[];
  frequency_bins: number[];
}