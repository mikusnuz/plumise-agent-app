import { useState, useEffect, useRef } from 'react';
import { Play, Square, Loader2, AlertTriangle } from 'lucide-react';
import type { AgentStatus } from '../../types';

interface ProcessControlProps {
  status: AgentStatus;
  hasPrivateKey: boolean;
  onStart: () => void;
  onStop: () => void;
}

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

export default function ProcessControl({ status, hasPrivateKey, onStart, onStop }: ProcessControlProps) {
  const canStart = (status === 'stopped' || status === 'error') && hasPrivateKey;
  const [elapsed, setElapsed] = useState(0);
  const intervalRef = useRef<number | null>(null);

  // Elapsed timer while starting
  useEffect(() => {
    if (status === 'starting') {
      setElapsed(0);
      intervalRef.current = window.setInterval(() => {
        setElapsed((prev) => prev + 1);
      }, 1000);
    } else {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
      setElapsed(0);
    }
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [status]);

  return (
    <div className="glass-card p-5">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold text-[var(--text-primary)]">Agent Process</h3>
          <p className="text-xs text-[var(--text-muted)] mt-0.5">
            {status === 'running' && 'Agent is running and processing requests'}
            {status === 'starting' && (
              <span className="text-cyan-400">
                Loading model... {formatElapsed(elapsed)}
                {elapsed > 30 && ' — first load can take several minutes'}
              </span>
            )}
            {status === 'stopping' && 'Gracefully shutting down...'}
            {status === 'stopped' && !hasPrivateKey && (
              <span className="text-amber-400 flex items-center gap-1">
                <AlertTriangle size={12} />
                Set your private key in Settings before starting
              </span>
            )}
            {status === 'stopped' && hasPrivateKey && 'Agent is not running'}
            {status === 'error' && 'Agent encountered an error'}
          </p>
        </div>

        <div className="flex items-center gap-2">
          {status === 'stopped' || status === 'error' ? (
            <button
              className="btn-primary flex items-center gap-2"
              onClick={onStart}
              disabled={!canStart}
              title={!hasPrivateKey ? 'Private key required' : ''}
            >
              <Play size={14} />
              <span>Start</span>
            </button>
          ) : status === 'running' ? (
            <button className="btn-danger flex items-center gap-2" onClick={onStop}>
              <Square size={14} />
              <span>Stop</span>
            </button>
          ) : (
            <button className="btn-secondary flex items-center gap-2" disabled>
              <Loader2 size={14} style={{ animation: 'spin-slow 1s linear infinite' }} />
              <span>{status === 'starting' ? 'Starting...' : 'Stopping...'}</span>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
