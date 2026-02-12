import { useState, useCallback, useRef, useEffect } from 'react';
import type { AgentStatus, AgentConfig, LogEntry, AgentMetrics, AgentHealth } from '../types';

const EMPTY_METRICS: AgentMetrics = {
  totalRequests: 0,
  totalTokensProcessed: 0,
  avgLatencyMs: 0,
  tokensPerSecond: 0,
  uptimeSeconds: 0,
};

/**
 * Main hook for managing the agent process lifecycle.
 * In Tauri production, this will invoke Rust commands.
 * For now, it mocks the process with HTTP polling.
 */
export function useAgentProcess() {
  const [status, setStatus] = useState<AgentStatus>('stopped');
  const [metrics, setMetrics] = useState<AgentMetrics>(EMPTY_METRICS);
  const [health, setHealth] = useState<AgentHealth | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const logIdRef = useRef(0);
  const pollRef = useRef<ReturnType<typeof setInterval>>();

  const addLog = useCallback((level: LogEntry['level'], message: string) => {
    const entry: LogEntry = {
      id: ++logIdRef.current,
      timestamp: new Date().toISOString(),
      level,
      message,
    };
    setLogs((prev) => {
      const next = [...prev, entry];
      return next.length > 500 ? next.slice(-500) : next;
    });
  }, []);

  const startPolling = useCallback((port: number) => {
    if (pollRef.current) clearInterval(pollRef.current);

    pollRef.current = setInterval(async () => {
      try {
        const [healthRes, metricsRes] = await Promise.all([
          fetch(`http://localhost:${port}/health`),
          fetch(`http://localhost:${port}/api/v1/metrics`),
        ]);

        if (healthRes.ok) {
          const h = await healthRes.json();
          setHealth(h);
          if (h.status === 'ready') {
            setStatus('running');
          }
        }

        if (metricsRes.ok) {
          const m = await metricsRes.json();
          setMetrics({
            totalRequests: m.total_requests ?? 0,
            totalTokensProcessed: m.total_tokens_processed ?? 0,
            avgLatencyMs: m.avg_latency_ms ?? 0,
            tokensPerSecond: m.tokens_per_second ?? 0,
            uptimeSeconds: m.uptime_seconds ?? 0,
          });
        }
      } catch {
        // Agent not ready yet, keep polling
      }
    }, 3000);
  }, []);

  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = undefined;
    }
  }, []);

  const start = useCallback(async (config: AgentConfig) => {
    if (status === 'running' || status === 'starting') return;

    setStatus('starting');
    setLogs([]);
    setMetrics(EMPTY_METRICS);
    setHealth(null);
    addLog('INFO', `Starting agent with model: ${config.model}`);
    addLog('INFO', `Device: ${config.device}, Oracle: ${config.oracleUrl}`);

    try {
      // In production Tauri, this will be:
      // await invoke('start_agent', { config });
      // For now, simulate startup
      addLog('INFO', 'Agent process launched');
      addLog('INFO', 'Waiting for model to load...');

      // Start polling the agent's HTTP API
      startPolling(config.httpPort);
    } catch (err) {
      setStatus('error');
      addLog('ERROR', `Failed to start agent: ${err}`);
    }
  }, [status, addLog, startPolling]);

  const stop = useCallback(async () => {
    if (status !== 'running' && status !== 'starting') return;

    setStatus('stopping');
    addLog('INFO', 'Stopping agent...');
    stopPolling();

    try {
      // In production Tauri:
      // await invoke('stop_agent');
      addLog('INFO', 'Agent stopped');
      setStatus('stopped');
      setHealth(null);
    } catch (err) {
      setStatus('error');
      addLog('ERROR', `Failed to stop agent: ${err}`);
    }
  }, [status, addLog, stopPolling]);

  useEffect(() => {
    return () => stopPolling();
  }, [stopPolling]);

  return { status, metrics, health, logs, start, stop, addLog };
}
