import { useMemo } from 'react';
import {
  Zap, Hash, Clock, Gauge, Cpu, HardDrive,
} from 'lucide-react';
import {
  AreaChart, Area, XAxis, YAxis, Tooltip, ResponsiveContainer,
} from 'recharts';
import StatCard from '../components/dashboard/StatCard';
import GaugeRing from '../components/dashboard/GaugeRing';
import ProcessControl from '../components/dashboard/ProcessControl';
import type { AgentStatus, AgentMetrics, AgentHealth, LogEntry } from '../types';
import { useSystemInfo } from '../hooks/useSystemInfo';

interface DashboardProps {
  status: AgentStatus;
  metrics: AgentMetrics;
  health: AgentHealth | null;
  logs: LogEntry[];
  hasPrivateKey: boolean;
  onStart: () => void;
  onStop: () => void;
}

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return `${h}h ${m}m`;
}

function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

// Mock throughput data for chart — in production, this will be real-time
function generateMockChartData() {
  const now = Date.now();
  return Array.from({ length: 20 }, (_, i) => ({
    time: new Date(now - (19 - i) * 5000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }),
    tps: 0,
  }));
}

export default function Dashboard({ status, metrics, health, logs, hasPrivateKey, onStart, onStop }: DashboardProps) {
  const chartData = useMemo(generateMockChartData, []);

  // Fetch system info when agent is running
  const { systemInfo } = useSystemInfo(status === 'running' || status === 'starting');

  // Calculate resource usage percentages
  const ramPercent = systemInfo && systemInfo.ramTotal > 0
    ? Math.round((systemInfo.ramUsed / systemInfo.ramTotal) * 100)
    : 0;
  const vramPercent = systemInfo && systemInfo.vramTotal > 0
    ? Math.round((systemInfo.vramUsed / systemInfo.vramTotal) * 100)
    : 0;

  const recentLogs = logs.slice(-8);

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-5">
      {/* Process Control */}
      <ProcessControl status={status} hasPrivateKey={hasPrivateKey} onStart={onStart} onStop={onStop} />

      {/* Stats Grid */}
      <div className="grid grid-cols-4 gap-4">
        <StatCard
          icon={Hash}
          label="Total Requests"
          value={formatNumber(metrics.totalRequests)}
          color="#06b6d4"
        />
        <StatCard
          icon={Zap}
          label="Tokens Processed"
          value={formatNumber(metrics.totalTokensProcessed)}
          color="#8b5cf6"
        />
        <StatCard
          icon={Gauge}
          label="Avg Latency"
          value={`${metrics.avgLatencyMs.toFixed(0)}ms`}
          color="#fb923c"
        />
        <StatCard
          icon={Clock}
          label="Uptime"
          value={formatUptime(metrics.uptimeSeconds)}
          color="#4ade80"
        />
      </div>

      {/* Middle Row: Chart + Gauges */}
      <div className="grid grid-cols-3 gap-4">
        {/* Throughput Chart */}
        <div className="col-span-2 glass-card p-4">
          <h3 className="text-sm font-semibold text-[var(--text-primary)] mb-3">
            Throughput
          </h3>
          <div className="h-48">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={chartData}>
                <defs>
                  <linearGradient id="tpsGrad" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="#06b6d4" stopOpacity={0.3} />
                    <stop offset="100%" stopColor="#06b6d4" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <XAxis
                  dataKey="time"
                  tick={{ fill: '#64748b', fontSize: 10 }}
                  axisLine={{ stroke: '#334155' }}
                  tickLine={false}
                />
                <YAxis
                  tick={{ fill: '#64748b', fontSize: 10 }}
                  axisLine={false}
                  tickLine={false}
                  width={35}
                />
                <Tooltip
                  contentStyle={{
                    background: 'rgba(15, 23, 42, 0.9)',
                    border: '1px solid rgba(148, 163, 184, 0.2)',
                    borderRadius: 8,
                    fontSize: 12,
                    color: '#f1f5f9',
                  }}
                />
                <Area
                  type="monotone"
                  dataKey="tps"
                  stroke="#06b6d4"
                  strokeWidth={2}
                  fill="url(#tpsGrad)"
                  name="tok/s"
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        </div>

        {/* System Gauges */}
        <div className="glass-card p-4 flex flex-col items-center justify-center gap-4">
          <h3 className="text-sm font-semibold text-[var(--text-primary)] self-start">
            System Resources
          </h3>
          <div className="flex gap-6">
            <GaugeRing
              value={ramPercent}
              label="RAM"
              detail={status === 'running' ? (health?.layers ? `L${health.layers.start}-${health.layers.end}` : 'All') : 'Idle'}
              color="#06b6d4"
              size={90}
            />
            <GaugeRing
              value={vramPercent}
              label="VRAM"
              detail={status === 'running' ? 'GPU' : 'Idle'}
              color="#8b5cf6"
              size={90}
            />
          </div>
          <div className="flex items-center gap-4 text-[10px] text-[var(--text-dim)]">
            <span className="flex items-center gap-1">
              <Cpu size={10} />
              {health?.mode ?? 'N/A'}
            </span>
            <span className="flex items-center gap-1">
              <HardDrive size={10} />
              {health?.model?.split('/').pop() ?? 'N/A'}
            </span>
          </div>
        </div>
      </div>

      {/* Recent Logs Preview */}
      <div className="glass-card p-4">
        <h3 className="text-sm font-semibold text-[var(--text-primary)] mb-3">
          Recent Activity
        </h3>
        <div className="space-y-1 min-h-[120px]">
          {recentLogs.length === 0 ? (
            <p className="text-xs text-[var(--text-dim)] py-8 text-center">
              No activity yet. Start the agent to see logs.
            </p>
          ) : (
            recentLogs.map((log) => (
              <div key={log.id} className="log-line flex gap-3">
                <span className="text-[var(--text-dim)] shrink-0 w-20">
                  {new Date(log.timestamp).toLocaleTimeString()}
                </span>
                <span
                  className="shrink-0 w-12"
                  style={{
                    color:
                      log.level === 'ERROR' ? '#ef4444' :
                      log.level === 'WARNING' ? '#fb923c' :
                      log.level === 'DEBUG' ? '#64748b' :
                      '#06b6d4',
                  }}
                >
                  {log.level}
                </span>
                <span className="text-[var(--text-secondary)] truncate">
                  {log.message}
                </span>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
