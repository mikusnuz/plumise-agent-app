import { useEffect, useRef } from 'react';
import { Minus, Square, X } from 'lucide-react';
import logo from '../../assets/logo.png';

let appWindow: any = null;

// Lazy init to avoid crash in non-Tauri environments
async function getAppWindow() {
  if (appWindow) return appWindow;
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    appWindow = getCurrentWindow();
    return appWindow;
  } catch {
    return null;
  }
}

interface TitleBarProps {
  onBeforeClose?: () => Promise<void>;
}

export default function TitleBar({ onBeforeClose }: TitleBarProps) {
  const closingRef = useRef(false);

  // Listen for system close event (Alt+F4, taskbar close, etc.)
  useEffect(() => {
    let unlisten: (() => void) | null = null;

    getAppWindow().then((win) => {
      if (!win) return;
      win.onCloseRequested(async (event: any) => {
        if (closingRef.current) return;
        closingRef.current = true;
        event.preventDefault();
        if (onBeforeClose) {
          await onBeforeClose();
        }
        await win.destroy();
      }).then((fn: () => void) => {
        unlisten = fn;
      });
    });

    return () => { if (unlisten) unlisten(); };
  }, [onBeforeClose]);

  const handleClose = async () => {
    if (closingRef.current) return;
    closingRef.current = true;
    if (onBeforeClose) {
      await onBeforeClose();
    }
    const win = await getAppWindow();
    if (win) await win.destroy();
  };

  return (
    <div
      data-tauri-drag-region
      className="flex items-center justify-between h-8 bg-[var(--bg-sidebar)] border-b border-[var(--border-divider)] select-none shrink-0"
    >
      <div data-tauri-drag-region className="flex items-center gap-2 pl-4">
        <img src={logo} alt="Plumise" className="w-4 h-4 pointer-events-none" />
        <span className="text-xs font-semibold text-[var(--text-secondary)] tracking-wide pointer-events-none">
          Plumise Agent
        </span>
      </div>

      <div className="flex">
        <button
          className="titlebar-btn"
          onClick={async () => {
            const win = await getAppWindow();
            if (win) win.minimize();
          }}
        >
          <Minus size={14} className="text-[var(--text-muted)]" />
        </button>
        <button
          className="titlebar-btn"
          onClick={async () => {
            const win = await getAppWindow();
            if (!win) return;
            const maximized = await win.isMaximized();
            maximized ? win.unmaximize() : win.maximize();
          }}
        >
          <Square size={11} className="text-[var(--text-muted)]" />
        </button>
        <button
          className="titlebar-btn titlebar-btn-close"
          onClick={handleClose}
        >
          <X size={14} className="text-[var(--text-muted)]" />
        </button>
      </div>
    </div>
  );
}
