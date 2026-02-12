import { useCallback, useRef } from 'react';
import { Routes, Route } from 'react-router-dom';
import TitleBar from './components/layout/TitleBar';
import Sidebar from './components/layout/Sidebar';
import Dashboard from './pages/Dashboard';
import Logs from './pages/Logs';
import Settings from './pages/Settings';
import { useAgentProcess } from './hooks/useAgentProcess';
import type { AgentConfig } from './types';
import { DEFAULT_CONFIG } from './types';

export default function App() {
  const { status, metrics, health, logs, start, stop, addLog } = useAgentProcess();
  const configRef = useRef<AgentConfig>(DEFAULT_CONFIG);

  const handleStart = useCallback(() => {
    start(configRef.current);
  }, [start]);

  const handleConfigChange = useCallback((config: AgentConfig) => {
    configRef.current = config;
  }, []);

  const handleClearLogs = useCallback(() => {
    addLog('INFO', '--- Logs cleared ---');
  }, [addLog]);

  return (
    <div className="flex flex-col h-full rounded-xl overflow-hidden border border-[var(--border-divider)]">
      <TitleBar />
      <div className="flex flex-1 overflow-hidden">
        <Sidebar status={status} />
        <Routes>
          <Route
            path="/"
            element={
              <Dashboard
                status={status}
                metrics={metrics}
                health={health}
                logs={logs}
                onStart={handleStart}
                onStop={stop}
              />
            }
          />
          <Route
            path="/logs"
            element={<Logs logs={logs} onClear={handleClearLogs} />}
          />
          <Route
            path="/settings"
            element={
              <Settings
                status={status}
                onConfigChange={handleConfigChange}
              />
            }
          />
        </Routes>
      </div>
    </div>
  );
}
