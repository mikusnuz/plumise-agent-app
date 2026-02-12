import { useRef, useEffect, useState, useMemo } from 'react';
import { ArrowDown, Trash2, Search } from 'lucide-react';
import type { LogEntry } from '../types';

interface LogsProps {
  logs: LogEntry[];
  onClear: () => void;
}

type LogLevel = 'ALL' | 'DEBUG' | 'INFO' | 'WARNING' | 'ERROR';

const LEVEL_COLORS: Record<string, string> = {
  DEBUG: '#64748b',
  INFO: '#06b6d4',
  WARNING: '#fb923c',
  ERROR: '#ef4444',
};

export default function Logs({ logs, onClear }: LogsProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [filter, setFilter] = useState<LogLevel>('ALL');
  const [search, setSearch] = useState('');

  const filteredLogs = useMemo(() => {
    return logs.filter((log) => {
      if (filter !== 'ALL' && log.level !== filter) return false;
      if (search && !log.message.toLowerCase().includes(search.toLowerCase())) return false;
      return true;
    });
  }, [logs, filter, search]);

  useEffect(() => {
    if (autoScroll && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [filteredLogs, autoScroll]);

  const handleScroll = () => {
    if (!containerRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = containerRef.current;
    setAutoScroll(scrollHeight - scrollTop - clientHeight < 50);
  };

  return (
    <div className="flex-1 flex flex-col overflow-hidden p-6 gap-4">
      {/* Toolbar */}
      <div className="flex items-center gap-3">
        <div className="relative flex-1 max-w-sm">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-dim)]" />
          <input
            type="text"
            placeholder="Filter logs..."
            className="input-field pl-9"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>

        <div className="flex gap-1">
          {(['ALL', 'DEBUG', 'INFO', 'WARNING', 'ERROR'] as LogLevel[]).map((level) => (
            <button
              key={level}
              onClick={() => setFilter(level)}
              className={`px-3 py-1.5 rounded text-xs font-medium transition-colors ${
                filter === level
                  ? 'bg-[var(--accent)] text-white'
                  : 'bg-[var(--bg-elevated)] text-[var(--text-muted)] hover:text-[var(--text-secondary)]'
              }`}
            >
              {level}
            </button>
          ))}
        </div>

        <div className="flex items-center gap-1 ml-auto">
          <span className="text-[10px] text-[var(--text-dim)]">
            {filteredLogs.length} / {logs.length}
          </span>
          <button
            className="p-1.5 rounded hover:bg-[var(--bg-elevated)] text-[var(--text-muted)]"
            onClick={onClear}
            title="Clear logs"
          >
            <Trash2 size={14} />
          </button>
        </div>
      </div>

      {/* Log Container */}
      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="flex-1 glass-card p-4 overflow-y-auto"
      >
        {filteredLogs.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[var(--text-dim)] text-sm">
            {logs.length === 0 ? 'No logs yet. Start the agent to see output.' : 'No logs match the current filter.'}
          </div>
        ) : (
          <div className="space-y-0.5">
            {filteredLogs.map((log) => (
              <div key={log.id} className="log-line flex gap-3 py-0.5 hover:bg-[var(--bg-hover)] rounded px-2 -mx-2">
                <span className="text-[var(--text-dim)] shrink-0 w-20">
                  {new Date(log.timestamp).toLocaleTimeString([], {
                    hour: '2-digit',
                    minute: '2-digit',
                    second: '2-digit',
                  })}
                </span>
                <span
                  className="shrink-0 w-14 text-center"
                  style={{ color: LEVEL_COLORS[log.level] }}
                >
                  {log.level}
                </span>
                <span className="text-[var(--text-secondary)]">
                  {log.message}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Auto-scroll indicator */}
      {!autoScroll && (
        <button
          onClick={() => {
            setAutoScroll(true);
            if (containerRef.current) {
              containerRef.current.scrollTop = containerRef.current.scrollHeight;
            }
          }}
          className="fixed bottom-8 right-8 btn-primary flex items-center gap-2 shadow-lg"
        >
          <ArrowDown size={14} />
          <span>Scroll to bottom</span>
        </button>
      )}
    </div>
  );
}
