import { useState, useCallback, useRef, useEffect } from 'react';
import type { AgentStatus, AgentConfig, LogEntry, AgentMetrics, AgentHealth } from '../types';

// --- Tauri API loading (awaitable) ---
let invokePromise: Promise<typeof import('@tauri-apps/api/core')['invoke']> | null = null;
let listenPromise: Promise<typeof import('@tauri-apps/api/event')['listen']> | null = null;

if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
  invokePromise = import('@tauri-apps/api/core').then((mod) => mod.invoke);
  listenPromise = import('@tauri-apps/api/event').then((mod) => mod.listen);
}

async function getInvoke() {
  if (!invokePromise) return null;
  try { return await invokePromise; } catch { return null; }
}

async function getListen() {
  if (!listenPromise) return null;
  try { return await listenPromise; } catch { return null; }
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
 */
export function useAgentProcess() {
  const [status, setStatus] = useState<AgentStatus>('stopped');
  const [metrics, setMetrics] = useState<AgentMetrics>(EMPTY_METRICS);
  const [health, setHealth] = useState<AgentHealth | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const logIdRef = useRef(0);
  const pollRef = useRef<number | null>(null);
  const startTimeRef = useRef<number | null>(null);
  const elapsedRef = useRef<number | null>(null);

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

    let pollCount = 0;
    let ready_detected = false;

    pollRef.current = window.setInterval(async () => {
      pollCount++;

      // Show elapsed time every 10 polls (~30s)
      if (startTimeRef.current && pollCount % 10 === 0) {
        const elapsed = Math.round((Date.now() - startTimeRef.current) / 1000);
        elapsedRef.current = elapsed;
        const mins = Math.floor(elapsed / 60);
        const secs = elapsed % 60;
        addLog('INFO', `Still loading... (${mins}m ${secs}s elapsed)`);
      }

      try {
        const [healthRes, metricsRes] = await Promise.all([
          fetch(`http://localhost:${port}/health`),
          fetch(`http://localhost:${port}/api/v1/metrics`),
        ]);

        if (healthRes.ok) {
          const h = await healthRes.json();
          setHealth(h);
          if ((h.status === 'ok' || h.status === 'ready') && !ready_detected) {
            ready_detected = true;
            setStatus('running');
            startTimeRef.current = null;
            const modeLabel = h.mode === 'pipeline' ? 'pipeline (distributed)' : 'single (standalone)';
            addLog('INFO', `Mode: ${modeLabel}`);
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
  }, [addLog]);

  const stopPolling = useCallback(() => {
    if (pollRef.current !== null) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
    startTimeRef.current = null;
    elapsedRef.current = null;
  }, []);

  const start = useCallback(async (config: AgentConfig) => {
    if (status === 'running' || status === 'starting') return;

    // Frontend-level validation
    if (!config.privateKey || config.privateKey.trim() === '') {
      setStatus('error');
      addLog('ERROR', 'Private key is required. Go to Settings to configure it.');
      return;
    }
    if (!config.privateKey.startsWith('0x') || config.privateKey.length !== 66) {
      setStatus('error');
      addLog('ERROR', 'Invalid private key format. Must be 0x-prefixed hex (66 chars).');
      return;
    }

    setStatus('starting');
    setLogs([]);
    setMetrics(EMPTY_METRICS);
    setHealth(null);
    startTimeRef.current = Date.now();

    // Await Tauri invoke (resolves immediately if already loaded)
    const invoke = await getInvoke();

    try {
      if (invoke) {
        // --- Tauri desktop mode ---
        addLog('INFO', 'Running pre-flight checks...');
        const result = await invoke('preflight_check', { config }) as {
          passed: boolean;
          checks: Array<{ name: string; passed: boolean; message: string }>;
        };

        for (const check of result.checks) {
          addLog(check.passed ? 'INFO' : 'ERROR', `[${check.name}] ${check.message}`);
        }

        if (!result.passed) {
          setStatus('error');
          addLog('ERROR', 'Pre-flight checks failed. Fix the issues above and try again.');
          startTimeRef.current = null;
          return;
        }
        addLog('INFO', 'All pre-flight checks passed');

        addLog('INFO', `Starting agent with model: ${config.model}`);
        addLog('INFO', `Device: ${config.device}`);

        await invoke('start_agent', { config });
        addLog('INFO', 'Agent process launched — loading model (this may take several minutes)...');
      } else {
        // --- Browser fallback (no Tauri) ---
        addLog('WARNING', 'Tauri API not available — running in browser mock mode');
        addLog('INFO', `Starting agent with model: ${config.model}`);
        addLog('INFO', `Device: ${config.device}`);
        addLog('INFO', 'Waiting for agent HTTP server...');
      }

      // Start polling the agent's HTTP API
      startPolling(config.httpPort);
    } catch (err) {
      setStatus('error');
      addLog('ERROR', `Failed to start agent: ${err}`);
      startTimeRef.current = null;
    }
  }, [status, addLog, startPolling]);

  const stop = useCallback(async () => {
    if (status !== 'running' && status !== 'starting') return;

    setStatus('stopping');
    addLog('INFO', 'Stopping agent...');
    stopPolling();

    const invoke = await getInvoke();

    try {
      if (invoke) {
        await invoke('stop_agent');
        addLog('INFO', 'Agent stopped');
      } else {
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

    // Set up Tauri event listeners
    getListen().then((listen) => {
      if (!listen) return;

      listen('agent-log', (event: any) => {
        const { level, message } = event.payload;
        addLog(level, message);
      }).then((unlisten: () => void) => {
        unlistenLog = unlisten;
      });

      listen('agent-status', (event: any) => {
        const { status: newStatus } = event.payload;
        const mapped = typeof newStatus === 'string'
          ? (newStatus.toLowerCase() as AgentStatus)
          : newStatus;
        setStatus(mapped);
      }).then((unlisten: () => void) => {
        unlistenStatus = unlisten;
      });
    });

    return () => {
      stopPolling();
      if (unlistenLog) unlistenLog();
      if (unlistenStatus) unlistenStatus();
    };
  }, [stopPolling, addLog]);

  const clearLogs = useCallback(() => {
    setLogs([]);
    logIdRef.current = 0;
  }, []);

  return { status, metrics, health, logs, start, stop, addLog, clearLogs };
}
