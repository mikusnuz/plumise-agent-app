import { useState, useEffect } from 'react';
import type { SystemInfo } from '../types';

let invoke: any = null;

// Dynamically import Tauri API if available
if (typeof window !== 'undefined' && '__TAURI__' in window) {
  import('@tauri-apps/api/core').then((mod) => {
    invoke = mod.invoke;
  });
}

const POLL_INTERVAL = 2000; // 2 seconds

export function useSystemInfo(enabled: boolean) {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);

  useEffect(() => {
    if (!enabled || !invoke) {
      setSystemInfo(null);
      return;
    }

    let isMounted = true;

    const fetchSystemInfo = async () => {
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
