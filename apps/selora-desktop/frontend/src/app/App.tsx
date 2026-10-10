// Selora IDE — Phase Z2 root component: the shell + overlays.
//
// Screens: connecting / error while the bootstrap flow runs; the
// seven-region shell once `bootstrap` lands, with the command palette and
// toast viewport as overlays. Command execution lives in the keymap feature
// (runCommandAction) so this component stays free of business logic.
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { getAppSnapshot, retryBootstrap, subscribeApp } from './app-store';
import { AppRuntime } from './providers';
import { ShellGrid } from '../layout/ShellGrid';
import { CommandPalette } from '../components/CommandPalette';
import { ToastViewport } from '../components/ToastViewport';
import { useShortcuts } from '../features/keymap/use-shortcuts';
import { buildCommandList, runCommandAction, type CommandId, type CommandSpec } from '../features/keymap/commands';
import { layoutStore } from '../features/layout/layout-store';
import { ConnectModal } from '../features/auth/ConnectModal';
import type { Layout, SettingsView, SurfaceId } from '../bridge/bridge';

function ConnectingScreen() {
  return (
    <main className="boot-screen" aria-live="polite">
      <p>Connecting to Rust core…</p>
    </main>
  );
}

function ErrorScreen({ error, onRetry }: { error: string; onRetry: () => void }) {
  return (
    <main className="boot-screen boot-error" role="alert">
      <h1 className="boot-title">Selora IDE</h1>
      <p className="boot-error-message">{error}</p>
      <button type="button" className="empty-action" onClick={onRetry}>
        Reconnect
      </button>
    </main>
  );
}

function AppInner() {
  const app = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [connectModalOpen, setConnectModalOpen] = useState(false);

  const settings = app.bootstrap?.settings ?? null;
  const commands: CommandSpec[] = useMemo(() => buildCommandList(settings), [settings]);

  const onCommand = useCallback(
    (id: CommandId) => {
      const deps = {
        layout: (): Layout | null => layoutStore.getSnapshot().layout,
        toggle: (surface: SurfaceId) => {
          void layoutStore.toggle(surface);
        },
        saveLayout: () => {
          void layoutStore.saveNow();
        },
        openPalette: () => setPaletteOpen(true),
      };
      runCommandAction(id, deps);
    },
    [],
  );
  const onClosePalette = useCallback(() => setPaletteOpen(false), []);

  useEffect(() => {
    const handler = () => setConnectModalOpen(true);
    window.addEventListener('selora:open-connect', handler);
    return () => window.removeEventListener('selora:open-connect', handler);
  }, []);

  useShortcuts({ settings, enabled: app.phase === 'ready', paletteOpen, onCommand, onClosePalette });

  if (app.phase === 'error') {
    return <ErrorScreen error={app.error ?? 'Bootstrap failed'} onRetry={() => void retryBootstrap()} />;
  }
  if (app.phase !== 'ready' || !app.bootstrap) {
    return <ConnectingScreen />;
  }

  return (
    <>
      <ShellGrid
        bootstrap={app.bootstrap}
        commands={commands}
        onRunCommand={onCommand}
        onOpenConnect={() => setConnectModalOpen(true)}
      />
      <CommandPalette open={paletteOpen} commands={commands} onRun={onCommand} onClose={onClosePalette} />
      <ConnectModal
        open={connectModalOpen}
        onClose={() => setConnectModalOpen(false)}
        bootstrap={app.bootstrap}
      />
      <ToastViewport />
    </>
  );
}

export function App() {
  return (
    <AppRuntime>
      <AppInner />
    </AppRuntime>
  );
}
