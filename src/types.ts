export type AgentStatus = 'stopped' | 'starting' | 'running' | 'stopping' | 'error';

export type NodeMode = 'standalone' | 'rpc-server' | 'coordinator';

export interface ClusterAssignment {
  mode: NodeMode;
  clusterId: string | null;
  rpcPort: number;
  rpcPeers: string[] | null; // coordinator only
}

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
  distributedMode: 'auto' | 'standalone' | 'disabled';
  rpcPort: number;
}

export interface LogEntry {
  id: number;
  timestamp: string;
  level: 'DEBUG' | 'INFO' | 'WARNING' | 'ERROR';
  message: string;
}

export const DEFAULT_CONFIG: AgentConfig = {
  privateKey: '',
  model: 'Qwen/Qwen3-32B-GGUF',
  modelFile: 'Qwen3-32B-Q4_K_M.gguf',
  device: 'auto',
  oracleUrl: 'https://plug.plumise.com/oracle',
  chainRpc: 'https://plug.plumise.com/rpc/plug_live_6VuDzRY1lNoA2noX0lSPGQlm9itOF9td4Jvvd4eAMzE',
  httpPort: 18920,
  gpuLayers: 99,
  ctxSize: 8192,
  parallelSlots: 1,
  ramLimitGb: 0,
  distributedMode: 'auto',
  rpcPort: 50052,
};
