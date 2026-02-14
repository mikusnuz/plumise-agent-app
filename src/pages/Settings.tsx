import { useState, useEffect } from 'react';
import { Save, Eye, EyeOff, RotateCcw, ChevronDown, ChevronRight } from 'lucide-react';
import type { AgentConfig, AgentStatus } from '../types';
import { DEFAULT_CONFIG } from '../types';

const STORAGE_KEY = 'plumise-agent-config';

// --- Tauri API loading (awaitable) ---
let invokePromise: Promise<typeof import('@tauri-apps/api/core')['invoke']> | null = null;

if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
  invokePromise = import('@tauri-apps/api/core').then((mod) => mod.invoke);
}

async function getInvoke() {
  if (!invokePromise) return null;
  try { return await invokePromise; } catch { return null; }
}

async function loadConfig(): Promise<AgentConfig> {
  const invoke = await getInvoke();
  if (invoke) {
    try {
      const config = await invoke('load_config') as AgentConfig;
      return { ...DEFAULT_CONFIG, ...config };
    } catch {
      return { ...DEFAULT_CONFIG };
    }
  }

  // Browser fallback: localStorage (privateKey is never stored here)
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      delete parsed.privateKey; // Ensure privateKey is never loaded from localStorage
      return { ...DEFAULT_CONFIG, ...parsed };
    }
  } catch { /* ignore */ }
  return { ...DEFAULT_CONFIG };
}

async function saveConfig(config: AgentConfig) {
  const invoke = await getInvoke();
  if (invoke) {
    try {
      await invoke('save_config', { config });
    } catch (err) {
      console.error('Failed to save config via Tauri:', err);
    }
  }

  // Save to localStorage as fallback, but NEVER store privateKey
  try {
    const { privateKey: _, ...safeConfig } = config;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(safeConfig));
  } catch (err) {
    console.error('Failed to save config to localStorage:', err);
  }
}

interface SettingsProps {
  status: AgentStatus;
  onConfigChange: (config: AgentConfig) => void;
}

export default function Settings({ status, onConfigChange }: SettingsProps) {
  const [config, setConfig] = useState<AgentConfig>(DEFAULT_CONFIG);
  const [showKey, setShowKey] = useState(false);
  const [saved, setSaved] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const isRunning = status === 'running' || status === 'starting';

  // Load config on mount
  useEffect(() => {
    loadConfig().then((loaded) => {
      setConfig(loaded);
      onConfigChange(loaded);
      setIsLoading(false);
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Auto-save on config changes (debounced) and propagate to parent
  useEffect(() => {
    if (isLoading) return;

    const timer = setTimeout(() => {
      saveConfig(config);
      onConfigChange(config);
    }, 500);

    return () => clearTimeout(timer);
  }, [config, isLoading]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleSave = () => {
    saveConfig(config);
    onConfigChange(config);
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  const handleReset = () => {
    setConfig({ ...DEFAULT_CONFIG });
  };

  const update = <K extends keyof AgentConfig>(key: K, value: AgentConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl space-y-6">
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold">Settings</h2>
          <div className="flex items-center gap-2">
            <button
              className="btn-secondary flex items-center gap-1.5"
              onClick={handleReset}
              disabled={isRunning}
            >
              <RotateCcw size={13} />
              Reset
            </button>
            <button
              className="btn-primary flex items-center gap-1.5"
              onClick={handleSave}
              disabled={isRunning}
            >
              <Save size={13} />
              {saved ? 'Saved!' : 'Save'}
            </button>
          </div>
        </div>

        {isRunning && (
          <div className="badge-warning text-xs">
            Stop the agent to modify settings
          </div>
        )}

        {/* Wallet */}
        <section className="glass-card p-5 space-y-4">
          <h3 className="text-sm font-semibold text-[var(--text-primary)]">Wallet</h3>

          <div>
            <label className="block text-xs text-[var(--text-muted)] mb-1.5">
              Private Key
            </label>
            <div className="relative">
              <input
                type={showKey ? 'text' : 'password'}
                className="input-field pr-10 font-mono text-xs"
                placeholder="0x..."
                value={config.privateKey}
                onChange={(e) => {
                  let val = e.target.value.trim();
                  // Auto-add 0x prefix if user pastes a raw hex key
                  if (val.length === 64 && /^[0-9a-fA-F]+$/.test(val)) {
                    val = '0x' + val;
                  }
                  update('privateKey', val);
                }}
                disabled={isRunning}
              />
              <button
                className="absolute right-2 top-1/2 -translate-y-1/2 text-[var(--text-dim)] hover:text-[var(--text-secondary)]"
                onClick={() => setShowKey(!showKey)}
              >
                {showKey ? <EyeOff size={15} /> : <Eye size={15} />}
              </button>
            </div>
            <p className="text-[10px] text-[var(--text-dim)] mt-1">
              Used for on-chain agent registration and reward claiming
            </p>
          </div>
        </section>

        {/* Model */}
        <section className="glass-card p-5 space-y-4">
          <h3 className="text-sm font-semibold text-[var(--text-primary)]">Model</h3>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-xs text-[var(--text-muted)] mb-1.5">
                Model Repository
              </label>
              <select
                className="input-field"
                value={config.model}
                onChange={(e) => update('model', e.target.value)}
                disabled={isRunning}
              >
                <option value="ggml-org/gpt-oss-20b-GGUF">gpt-oss-20b GGUF (12GB)</option>
              </select>
            </div>

            <div>
              <label className="block text-xs text-[var(--text-muted)] mb-1.5">
                Device
              </label>
              <select
                className="input-field"
                value={config.device}
                onChange={(e) => update('device', e.target.value)}
                disabled={isRunning}
              >
                <option value="auto">Auto Detect</option>
                <option value="cuda">CUDA (GPU)</option>
                <option value="cpu">CPU Only</option>
              </select>
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-xs text-[var(--text-muted)] mb-1.5">
                GPU Layers
              </label>
              <input
                type="number"
                className="input-field w-32"
                value={config.gpuLayers}
                onChange={(e) => update('gpuLayers', parseInt(e.target.value) || 0)}
                disabled={isRunning}
                min={0}
                max={999}
              />
              <p className="text-[10px] text-[var(--text-dim)] mt-1">
                0 = CPU only, 99 = all layers on GPU
              </p>
            </div>

            <div>
              <label className="block text-xs text-[var(--text-muted)] mb-1.5">
                Context Size
              </label>
              <select
                className="input-field"
                value={config.ctxSize}
                onChange={(e) => update('ctxSize', parseInt(e.target.value))}
                disabled={isRunning}
              >
                <option value={2048}>2048</option>
                <option value={4096}>4096</option>
                <option value={8192}>8192</option>
                <option value={16384}>16384</option>
                <option value={32768}>32768</option>
              </select>
            </div>
          </div>
        </section>

        {/* Advanced */}
        <section className="glass-card p-5 space-y-4">
          <button
            className="flex items-center gap-2 text-sm font-semibold text-[var(--text-primary)] w-full"
            onClick={() => setShowAdvanced(!showAdvanced)}
          >
            {showAdvanced ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            Advanced
          </button>

          {showAdvanced && (
            <div className="space-y-4 pt-2">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-xs text-[var(--text-muted)] mb-1.5">
                    Oracle URL
                  </label>
                  <input
                    type="text"
                    className="input-field text-xs"
                    value={config.oracleUrl}
                    onChange={(e) => update('oracleUrl', e.target.value)}
                    disabled={isRunning}
                  />
                </div>

                <div>
                  <label className="block text-xs text-[var(--text-muted)] mb-1.5">
                    Chain RPC
                  </label>
                  <input
                    type="text"
                    className="input-field text-xs"
                    value={config.chainRpc}
                    onChange={(e) => update('chainRpc', e.target.value)}
                    disabled={isRunning}
                  />
                </div>
              </div>

              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-xs text-[var(--text-muted)] mb-1.5">
                    HTTP Port
                  </label>
                  <input
                    type="number"
                    className="input-field w-32"
                    value={config.httpPort}
                    onChange={(e) => update('httpPort', parseInt(e.target.value) || 18920)}
                    disabled={isRunning}
                  />
                  <p className="text-[10px] text-[var(--text-dim)] mt-1">
                    Local port for llama-server
                  </p>
                </div>

                <div>
                  <label className="block text-xs text-[var(--text-muted)] mb-1.5">
                    Parallel Slots
                  </label>
                  <input
                    type="number"
                    className="input-field w-32"
                    value={config.parallelSlots}
                    onChange={(e) => update('parallelSlots', parseInt(e.target.value) || 1)}
                    disabled={isRunning}
                    min={1}
                    max={32}
                  />
                  <p className="text-[10px] text-[var(--text-dim)] mt-1">
                    Concurrent inference slots
                  </p>
                </div>
              </div>
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
