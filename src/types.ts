export type AgentStatus = 'stopped' | 'starting' | 'running' | 'stopping' | 'error';

export interface AgentMetrics {
  totalRequests: number;
  totalTokensProcessed: number;
  avgLatencyMs: number;
  tokensPerSecond: number;
  uptimeSeconds: number;
}

export interface AgentHealth {
  status: string;
  model: string;
  mode: string;
  address: string;
  uptime: number;
  layers?: { start: number; end: number; total: number };
}

export interface SystemInfo {
  cpuUsage: number;
  ramTotal: number;
  ramUsed: number;
  vramTotal: number;
  vramUsed: number;
  gpuName: string;
}

export interface LoadingProgress {
  percent: number;
  phase: string;
  downloadedBytes?: number;
  totalBytes?: number;
}

export interface AgentConfig {
  privateKey: string;
  model: string;
  modelFile: string;
  device: string;
  oracleUrl: string;
  chainRpc: string;
  httpPort: number;
  gpuLayers: number;
  ctxSize: number;
  parallelSlots: number;
  ramLimitGb: number;
}

export interface LogEntry {
  id: number;
  timestamp: string;
  level: 'DEBUG' | 'INFO' | 'WARNING' | 'ERROR';
  message: string;
}

export const DEFAULT_CONFIG: AgentConfig = {
  privateKey: '',
  model: 'ggml-org/gpt-oss-20b-GGUF',
  modelFile: 'gpt-oss-20b-mxfp4.gguf',
  device: 'auto',
  oracleUrl: 'https://node-1.plumise.com/oracle',
  chainRpc: 'https://node-1.plumise.com/rpc',
  httpPort: 18920,
  gpuLayers: 99,
  ctxSize: 8192,
  parallelSlots: 4,
  ramLimitGb: 0,
};
