import { getCurrentWindow } from '@tauri-apps/api/window';
import { Minus, Square, X } from 'lucide-react';
import logo from '../../assets/logo.png';

const appWindow = getCurrentWindow();

export default function TitleBar() {
  return (
    <div
      data-tauri-drag-region
      className="flex items-center justify-between h-8 bg-[var(--bg-sidebar)] border-b border-[var(--border-divider)] select-none shrink-0"
    >
      <div data-tauri-drag-region className="flex items-center gap-2 pl-4">
        <img src={logo} alt="Plumise" className="w-4 h-4" />
        <span className="text-xs font-semibold text-[var(--text-secondary)] tracking-wide">
          Plumise Agent
        </span>
      </div>

      <div className="flex">
        <button
          className="titlebar-btn"
          onClick={() => appWindow.minimize()}
        >
          <Minus size={14} className="text-[var(--text-muted)]" />
        </button>
        <button
          className="titlebar-btn"
          onClick={async () => {
            const maximized = await appWindow.isMaximized();
            maximized ? appWindow.unmaximize() : appWindow.maximize();
          }}
        >
          <Square size={11} className="text-[var(--text-muted)]" />
        </button>
        <button
          className="titlebar-btn titlebar-btn-close"
          onClick={() => appWindow.close()}
        >
          <X size={14} className="text-[var(--text-muted)]" />
        </button>
      </div>
    </div>
  );
}
