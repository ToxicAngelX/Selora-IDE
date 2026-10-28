// Selora IDE — Phase Z2 root component: the shell + overlays.
//
// Screens: connecting / error while the bootstrap flow runs; the
// seven-region shell once `bootstrap` lands, with the command palette and
// toast viewport as overlays. Command execution lives in the keymap feature
// (runCommandAction) so this component stays free of business logic.
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from 'react';
import { currentEnvelopeContext, getAppSnapshot, retryBootstrap, subscribeApp } from './app-store';
import { openExternal, openFolderAt, pickFolder } from '../bridge/bridge';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { editorStore } from '../features/editor/editor-store';
import { toastStore } from '../features/toasts/toast-store';
import { AppRuntime } from './providers';
import { ShellGrid } from '../layout/ShellGrid';
import { CommandPalette } from '../components/CommandPalette';
import { CheatSheet, useCheatSheetTrigger } from '../components/CheatSheet';
import { AboutDialog, useAboutTrigger } from '../components/AboutDialog';
import { UpdatePrompt } from '../components/UpdatePrompt';
import { ToastViewport } from '../components/ToastViewport';
import { OnboardingOverlay } from '../features/onboarding/OnboardingOverlay';
import { useShortcuts } from '../features/keymap/use-shortcuts';
import { buildCommandList, runCommandAction, type CommandId, type CommandSpec } from '../features/keymap/commands';
import { layoutStore } from '../features/layout/layout-store';
// The connect flow pulls the provider cards and their icon set; it only
// renders on demand, so keep it in its own chunk (Z6 split).
const ConnectModal = lazy(async () => {
  const module = await import('../features/auth/ConnectModal');
  return { default: module.ConnectModal };
});
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
  const [cheatSheetOpen, setCheatSheetOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [connectModalOpen, setConnectModalOpen] = useState(false);

  const settings = app.bootstrap?.settings ?? null;
  const commands: CommandSpec[] = useMemo(() => buildCommandList(settings), [settings]);

  const openFolder = useCallback(async () => {
    const context = currentEnvelopeContext();
    if (!context) {
      toastStore.pushLocal('warning', 'Connect to a workspace before opening a folder.');
      return;
    }
    try {
      const root = await pickFolder();
      if (!root) return;
      await openFolderAt(root, context);
    } catch (error) {
      const detail = error instanceof Error && error.message.trim() ? error.message.trim() : 'Unknown error';
      toastStore.pushLocal('error', `Open folder failed: ${detail}`);
    }
  }, []);

  const openDocumentation = useCallback(() => {
    const context = currentEnvelopeContext();
    if (!context) {
      toastStore.pushLocal('warning', 'Documentation needs a live shell connection.');
      return;
    }
    void openExternal('https://github.com/ToxicAngelX/Selora-IDE#readme', context).catch(
      (error: unknown) =>
        toastStore.pushLocal(
          'error',
          `Could not open documentation: ${error instanceof Error ? error.message : String(error)}`,
        ),
    );
  }, []);

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
        openCheatsheet: () => setCheatSheetOpen(true),
        showAbout: () => window.dispatchEvent(new CustomEvent('selora:about')),
        openFolder: () => void openFolder(),
        saveActive: () => editorStore.saveActive(),
        closeActive: () => {
          const editor = editorStore.getSnapshot();
          const tab = editor.tabs.find((item) => item.id === editor.activeBufferId);
          if (!tab) return;
          if (tab.isDirty) {
            toastStore.pushLocal(
              'warning',
              'Save or discard the unsaved changes from the tab before closing it.',
            );
            return;
          }
          editorStore.closeBuffer(tab.id);
        },
        closeWindow: () => {
          try {
            void getCurrentWindow().close();
          } catch {
            // Not inside the Tauri WebView.
          }
        },
        openDocumentation,
      };
      runCommandAction(id, deps);
    },
    [openDocumentation, openFolder],
  );
  const onClosePalette = useCallback(() => setPaletteOpen(false), []);
  const openCheatSheet = useCallback(() => setCheatSheetOpen(true), []);
  const openAbout = useCallback(() => setAboutOpen(true), []);
  useCheatSheetTrigger(openCheatSheet);
  useAboutTrigger(openAbout);

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
      <CheatSheet open={cheatSheetOpen} commands={commands} onClose={() => setCheatSheetOpen(false)} />
      <AboutDialog open={aboutOpen} onClose={() => setAboutOpen(false)} />
      <UpdatePrompt autoCheck={app.bootstrap.settings.check_for_updates ?? true} />
      <OnboardingOverlay />
      {connectModalOpen ? (
        <Suspense fallback={null}>
          <ConnectModal
            open={connectModalOpen}
            onClose={() => setConnectModalOpen(false)}
            bootstrap={app.bootstrap}
          />
        </Suspense>
      ) : null}
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
