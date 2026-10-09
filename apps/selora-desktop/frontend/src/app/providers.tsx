// Selora IDE — Phase Z2 app providers.
//
// `AppRuntime` owns the app-lifetime side effects: it starts the
// bootstrap/replay flow once (StrictMode-safe) and routes the bridge's
// command-error stream into the toast surface (ARCHITECTURE §4.4).
import { useEffect, type ReactNode } from 'react';
import { startApp } from './app-store';
import { onBridgeCommandError } from '../bridge/bridge';
import { toastStore } from '../features/toasts/toast-store';

export function AppRuntime({ children }: { children: ReactNode }) {
  useEffect(() => {
    void startApp();
  }, []);
  useEffect(
    () =>
      onBridgeCommandError((error) => {
        toastStore.pushLocal('error', error.message);
      }),
    [],
  );
  return <>{children}</>;
}
