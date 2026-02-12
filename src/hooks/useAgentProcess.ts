import { useState, useCallback, useRef, useEffect } from 'react';
import type { AgentStatus, AgentConfig, LogEntry, AgentMetrics, AgentHealth } from '../types';

let invoke: any = null;
let listen: any = null;

// Dynamically import Tauri APIs if available (desktop mode)
if (typeof window !== 'undefined' && '__TAURI__' in window) {
  import('@tauri-apps/api/core').then((mod) => {
    invoke = mod.invoke;
  });
  import('@tauri-apps/api/event').then((mod) => {
    listen = mod.listen;
  });
}

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
  const pollRef = useRef<number | null>(null);

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
    if (pollRef.current !== null) clearInterval(pollRef.current);

    pollRef.current = window.setInterval(async () => {
      try {
        const [healthRes, metricsRes] = await Promise.all([
          fetch(`http://localhost:${port}/health`),
          fetch(`http://localhost:${port}/api/v1/metrics`),
        ]);

        if (healthRes.ok) {
          const h = await healthRes.json();
          setHealth(h);
          if (h.status === 'ok' || h.status === 'ready') {
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
    if (pollRef.current !== null) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  const start = useCallback(async (config: AgentConfig) => {
    if (status === 'running' || status === 'starting') return;

    setStatus('starting');
    setLogs([]);
    setMetrics(EMPTY_METRICS);
    setHealth(null);

    try {
      // Run pre-flight checks (Tauri only)
      if (invoke) {
        addLog('INFO', 'Running pre-flight checks...');
        const result = await invoke('preflight_check', { config }) as { passed: boolean; checks: Array<{ name: string; passed: boolean; message: string }> };

        for (const check of result.checks) {
          addLog(check.passed ? 'INFO' : 'ERROR', `[${check.name}] ${check.message}`);
        }

        if (!result.passed) {
          setStatus('error');
          addLog('ERROR', 'Pre-flight checks failed. Fix the issues above and try again.');
          return;
        }
        addLog('INFO', 'All pre-flight checks passed');
      }

      addLog('INFO', `Starting agent with model: ${config.model}`);
      addLog('INFO', `Device: ${config.device}, Mode: standalone`);

      if (invoke) {
        await invoke('start_agent', { config });
        addLog('INFO', 'Agent process launched');
      } else {
        addLog('INFO', 'Agent process launched (mock)');
        addLog('INFO', 'Waiting for model to load...');
      }

      // Start polling the agent's HTTP API (source of truth for metrics)
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
      if (invoke) {
        // Tauri desktop mode
        await invoke('stop_agent');
        addLog('INFO', 'Agent stopped via Tauri');
      } else {
        // Browser fallback mode
        addLog('INFO', 'Agent stopped (mock)');
      }
      setStatus('stopped');
      setHealth(null);
    } catch (err) {
      setStatus('error');
      addLog('ERROR', `Failed to stop agent: ${err}`);
    }
  }, [status, addLog, stopPolling]);

  useEffect(() => {
    let unlistenLog: (() => void) | null = null;
    let unlistenStatus: (() => void) | null = null;

    // Set up Tauri event listeners if available
    if (listen) {
      // Listen for agent-log events
      listen('agent-log', (event: any) => {
        const { level, message } = event.payload;
        addLog(level, message);
      }).then((unlisten: () => void) => {
        unlistenLog = unlisten;
      });

      // Listen for agent-status events (emitted by Rust backend)
      listen('agent-status', (event: any) => {
        const { status: newStatus } = event.payload;
        // Map Rust enum (PascalCase) to frontend lowercase
        const mapped = typeof newStatus === 'string'
          ? (newStatus.toLowerCase() as AgentStatus)
          : newStatus;
        setStatus(mapped);
      }).then((unlisten: () => void) => {
        unlistenStatus = unlisten;
      });
    }

    return () => {
      stopPolling();
      if (unlistenLog) unlistenLog();
      if (unlistenStatus) unlistenStatus();
    };
  }, [stopPolling, addLog]);

  return { status, metrics, health, logs, start, stop, addLog };
}
