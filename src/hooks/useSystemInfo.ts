import { useState, useEffect } from 'react';
import type { SystemInfo } from '../types';

// --- Tauri API loading (awaitable) ---
let invokePromise: Promise<typeof import('@tauri-apps/api/core')['invoke']> | null = null;

if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
  invokePromise = import('@tauri-apps/api/core').then((mod) => mod.invoke);
}

async function getInvoke() {
  if (!invokePromise) return null;
  try { return await invokePromise; } catch { return null; }
}

const POLL_INTERVAL = 2000; // 2 seconds

export function useSystemInfo(enabled: boolean) {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);

  useEffect(() => {
    if (!enabled) {
      setSystemInfo(null);
      return;
    }

    let isMounted = true;

    const fetchSystemInfo = async () => {
      const invoke = await getInvoke();
      if (!invoke || !isMounted) return;

      try {
        const info = await invoke('get_system_info') as SystemInfo;
        if (isMounted) {
          setSystemInfo(info);
        }
      } catch (err) {
        console.error('Failed to fetch system info:', err);
        if (isMounted) {
          setSystemInfo(null);
        }
      }
    };

    // Initial fetch
    fetchSystemInfo();

    // Set up polling
    const interval = setInterval(fetchSystemInfo, POLL_INTERVAL);

    return () => {
      isMounted = false;
      clearInterval(interval);
    };
  }, [enabled]);

  return { systemInfo };
}
