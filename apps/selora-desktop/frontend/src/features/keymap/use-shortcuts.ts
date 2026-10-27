// Selora IDE — Phase Z2 global shortcut listener.
//
// One `keydown` listener (ARCHITECTURE §7.1). Esc and the palette chord are
// always live (Esc closes the palette when one is open, else dismisses the
// topmost toast); every other command binding is swallowed (`preventDefault`)
// only when matched, and is inert while a text input inside a surface has
// focus or the palette is open.
import { useEffect } from 'react';
import { keybindMatches, normalizeEventKey, type Keybind } from './keybind';
import { resolveKeymap, type CommandId } from './commands';
import { toastStore } from '../toasts/toast-store';
import type { SettingsView } from '../../bridge/bridge';

export type ShortcutOptions = {
  settings: SettingsView | null;
  enabled: boolean;
  paletteOpen: boolean;
  onCommand: (id: CommandId) => void;
  onClosePalette: () => void;
};

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
}

export function useShortcuts({ settings, enabled, paletteOpen, onCommand, onClosePalette }: ShortcutOptions): void {
  useEffect(() => {
    if (!enabled) return;
    const keymap = resolveKeymap(settings);
    const ordered: Array<[CommandId, Keybind]> = [...keymap.entries()];

    const handler = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if (normalizeEventKey(event.key) === 'esc') {
        // §7.1: Esc closes the palette; with no modal open it dismisses the
        // topmost toast.
        if (paletteOpen) {
          event.preventDefault();
          onClosePalette();
        } else {
          toastStore.dismissTopmost();
        }
        return;
      }
      // The palette chord works even while a text input has focus (§7.1).
      const paletteBinding = keymap.get('selora.openCommandPalette');
      if (paletteBinding && keybindMatches(paletteBinding, event)) {
        event.preventDefault();
        onCommand('selora.openCommandPalette');
        return;
      }
      // Everything else is inert while a modal is open or a text input has focus.
      if (paletteOpen) return;
      // Closing the editor stays live inside the CodeMirror surface: its
      // contenteditable would otherwise swallow Ctrl+F4, and the tab-menu hint
      // promises it works from the editor.
      const closeBinding = keymap.get('selora.fileCloseEditor');
      if (closeBinding && keybindMatches(closeBinding, event)) {
        event.preventDefault();
        onCommand('selora.fileCloseEditor');
        return;
      }
      if (isTextEntry(event.target)) return;
      for (const [id, keybind] of ordered) {
        if (id === 'selora.openCommandPalette') continue; // handled above
        if (keybindMatches(keybind, event)) {
          event.preventDefault();
          onCommand(id);
          return;
        }
      }
    };

    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [enabled, paletteOpen, settings, onCommand, onClosePalette]);
}
