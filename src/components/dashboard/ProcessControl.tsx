import { useState, useEffect, useRef } from 'react';
import { Play, Square, Loader2, AlertTriangle } from 'lucide-react';
import type { AgentStatus } from '../../types';

interface ProcessControlProps {
  status: AgentStatus;
  hasPrivateKey: boolean;
  loadingProgress?: { percent: number; phase: string } | null;
  onStart: () => void;
  onStop: () => void;
}

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

// Estimated loading steps with time-based progress
function getProgressInfo(elapsed: number): { percent: number; step: string } {
  if (elapsed < 3) return { percent: 5, step: 'Running pre-flight checks...' };
  if (elapsed < 8) return { percent: 10, step: 'Launching agent process...' };
  if (elapsed < 15) return { percent: 15, step: 'Initializing model runtime...' };
  if (elapsed < 30) return { percent: 20 + Math.min((elapsed - 15) * 2, 30), step: 'Downloading model weights...' };
  if (elapsed < 120) return { percent: 50 + Math.min((elapsed - 30) * 0.4, 40), step: 'Loading model into memory...' };
  return { percent: Math.min(90 + (elapsed - 120) * 0.05, 98), step: 'Finalizing model setup...' };
}

export default function ProcessControl({ status, hasPrivateKey, loadingProgress, onStart, onStop }: ProcessControlProps) {
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

  const progress = status === 'starting'
    ? (loadingProgress
        ? {
            percent: loadingProgress.phase === 'downloading'
              ? 10 + loadingProgress.percent * 0.4
              : 50 + loadingProgress.percent * 0.45,
            step: loadingProgress.phase === 'downloading'
              ? `Downloading model weights... (${Math.round(loadingProgress.percent)}%)`
              : `Loading model into memory... (${Math.round(loadingProgress.percent)}%)`,
          }
        : getProgressInfo(elapsed))
    : null;

  return (
    <div className="glass-card p-5">
      <div className="flex items-center justify-between">
        <div className="flex-1 min-w-0">
          <h3 className="text-sm font-semibold text-[var(--text-primary)]">Agent Process</h3>
          <p className="text-xs text-[var(--text-muted)] mt-0.5">
            {status === 'running' && 'Agent is running and processing requests'}
            {status === 'starting' && progress && (
              <span className="text-cyan-400">
                {progress.step} ({formatElapsed(elapsed)})
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

        <div className="flex items-center gap-2 shrink-0">
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

      {/* Progress bar during starting */}
      {status === 'starting' && progress && (
        <div className="mt-3">
          <div className="flex items-center justify-between text-[10px] text-[var(--text-dim)] mb-1">
            <span>{progress.step}</span>
            <span>{Math.round(progress.percent)}%</span>
          </div>
          <div className="h-1.5 rounded-full bg-[var(--bg-elevated)] overflow-hidden">
            <div
              className="h-full rounded-full bg-gradient-to-r from-cyan-500 to-blue-500"
              style={{
                width: `${progress.percent}%`,
                transition: 'width 1s ease-out',
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
