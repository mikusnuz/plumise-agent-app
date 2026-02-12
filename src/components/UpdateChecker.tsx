import { useEffect, useState } from 'react';
import { check as checkUpdate } from '@tauri-apps/plugin-updater';
import { Download, AlertCircle, X } from 'lucide-react';

interface UpdateInfo {
  version: string;
  date?: string;
  body?: string;
}

export default function UpdateChecker() {
  const [updateAvailable, setUpdateAvailable] = useState<UpdateInfo | null>(null);
  const [isDownloading, setIsDownloading] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
      return;
    }

    const performUpdateCheck = async () => {
      try {
        const update = await checkUpdate();

        if (update?.available) {
          setUpdateAvailable({
            version: update.version,
            date: update.date,
            body: update.body,
          });
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : 'Unknown error';
        console.error('[UpdateChecker] Failed to check for updates:', message);
        setError(message);
      }
    };

    performUpdateCheck();
  }, []);

  const handleInstall = async () => {
    if (!updateAvailable || isDownloading) return;

    try {
      setIsDownloading(true);
      setDownloadProgress(0);

      const update = await checkUpdate();
      if (!update?.available) {
        setError('No update available');
        return;
      }

      let totalBytes = 0;
      let downloadedBytes = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            totalBytes = (event.data as any).contentLength ?? 0;
            downloadedBytes = 0;
            setDownloadProgress(0);
            break;
          case 'Progress':
            downloadedBytes += event.data.chunkLength;
            if (totalBytes > 0) {
              setDownloadProgress((downloadedBytes / totalBytes) * 100);
            }
            break;
          case 'Finished':
            setDownloadProgress(100);
            break;
        }
      });

      // Relaunch after install
      const { relaunch: doRelaunch } = await import('@tauri-apps/plugin-process');
      await doRelaunch();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Installation failed';
      console.error('[UpdateChecker] Installation error:', message);
      setError(message);
      setIsDownloading(false);
    }
  };

  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    return null;
  }

  if (!updateAvailable || dismissed) {
    return null;
  }

  return (
    <div className="glass-card mx-4 mt-3 mb-0 p-3 border-[var(--border-accent)]">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-3 flex-1 min-w-0">
          {error ? (
            <>
              <AlertCircle size={18} className="text-[var(--warning)] flex-shrink-0" />
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-[var(--text-primary)]">
                  Update check failed
                </p>
                <p className="text-xs text-[var(--text-muted)] truncate">{error}</p>
              </div>
            </>
          ) : (
            <>
              <Download size={18} className="text-[var(--accent)] flex-shrink-0" />
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-[var(--text-primary)]">
                  Update available: v{updateAvailable.version}
                </p>
                {isDownloading && (
                  <div className="mt-1.5">
                    <div className="h-1.5 bg-[var(--bg-input)] rounded-full overflow-hidden">
                      <div
                        className="h-full bg-gradient-to-r from-[var(--accent)] to-[var(--accent-purple)] transition-all duration-300"
                        style={{ width: `${downloadProgress}%` }}
                      />
                    </div>
                  </div>
                )}
              </div>
            </>
          )}
        </div>

        <div className="flex items-center gap-2 flex-shrink-0">
          {!error && !isDownloading && (
            <button
              className="btn-primary flex items-center gap-1.5 text-xs py-1.5 px-3"
              onClick={handleInstall}
            >
              <Download size={12} />
              <span>Install & Restart</span>
            </button>
          )}
          <button
            className="titlebar-btn w-7 h-7 rounded"
            onClick={() => setDismissed(true)}
            aria-label="Dismiss"
          >
            <X size={14} className="text-[var(--text-muted)]" />
          </button>
        </div>
      </div>
    </div>
  );
}
