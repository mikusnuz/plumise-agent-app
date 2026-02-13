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
}

export interface AgentConfig {
  privateKey: string;
  model: string;
  device: string;
  oracleUrl: string;
  chainRpc: string;
  httpPort: number;
  grpcPort: number;
  ramLimitMb: number;
}

export interface LogEntry {
  id: number;
  timestamp: string;
  level: 'DEBUG' | 'INFO' | 'WARNING' | 'ERROR';
  message: string;
}

export const DEFAULT_CONFIG: AgentConfig = {
  privateKey: '',
  model: 'openai/gpt-oss-20b',
  device: 'auto',
  oracleUrl: 'https://node-1.plumise.com/oracle',
  chainRpc: 'https://node-1.plumise.com/rpc',
  httpPort: 18920,
  grpcPort: 0, // standalone mode: gRPC disabled
  ramLimitMb: 0,
};
