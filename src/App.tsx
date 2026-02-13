import { useCallback, useRef, useState } from 'react';
import { Routes, Route } from 'react-router-dom';
import TitleBar from './components/layout/TitleBar';
import Sidebar from './components/layout/Sidebar';
import UpdateChecker from './components/UpdateChecker';
import Dashboard from './pages/Dashboard';
import Logs from './pages/Logs';
import Settings from './pages/Settings';
import { useAgentProcess } from './hooks/useAgentProcess';
import type { AgentConfig } from './types';
import { DEFAULT_CONFIG } from './types';

function isValidPrivateKey(key: string): boolean {
  return key.startsWith('0x') && key.length === 66;
}

export default function App() {
  const { status, metrics, health, logs, start, stop, addLog, clearLogs } = useAgentProcess();
  const configRef = useRef<AgentConfig>(DEFAULT_CONFIG);
  const [hasPrivateKey, setHasPrivateKey] = useState(false);

  const handleStart = useCallback(() => {
    start(configRef.current);
  }, [start]);

  const handleConfigChange = useCallback((config: AgentConfig) => {
    configRef.current = config;
    setHasPrivateKey(isValidPrivateKey(config.privateKey));
  }, []);

  const handleClearLogs = useCallback(() => {
    clearLogs();
  }, [clearLogs]);

  // Stop agent before closing the app window
  const handleBeforeClose = useCallback(async () => {
    if (status === 'running' || status === 'starting') {
      await stop();
    }
  }, [status, stop]);

  return (
    <div className="flex flex-col h-full rounded-xl overflow-hidden border border-[var(--border-divider)]">
      <TitleBar onBeforeClose={handleBeforeClose} />
      <UpdateChecker />
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
                hasPrivateKey={hasPrivateKey}
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
