// Selora IDE — Phase Z4 CodeMirror 6 editor pane.
//
// One EditorView lives for the whole pane lifetime and swaps its document,
// language, and reveal target as the active buffer changes. Local edits are
// converted into protocol TextChange records and sent with a monotonically
// increasing local version; protocol echoes are applied without being sent
// back, and selection changes are debounced before leaving the webview.
import { useEffect, useRef } from 'react';
import {
  Annotation,
  Compartment,
  EditorSelection,
  EditorState,
  Transaction,
  type ChangeSet,
  type Extension,
  type Text,
} from '@codemirror/state';
import {
  EditorView,
  crosshairCursor,
  drawSelection,
  dropCursor,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  hoverTooltip,
  keymap,
  lineNumbers,
  rectangularSelection,
  type Tooltip,
  type ViewUpdate,
} from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { bracketMatching, foldGutter, indentOnInput } from '@codemirror/language';
import { search, searchKeymap } from '@codemirror/search';
import {
  autocompletion,
  completionKeymap,
  startCompletion,
  type CompletionContext,
  type CompletionResult as CmCompletionResult,
  type CompletionSource,
} from '@codemirror/autocomplete';
import { lintGutter, linter } from '@codemirror/lint';
import {
  applyEditorChange,
  completionAt,
  definitionAt,
  hoverAt,
  setSelection,
  type BufferSnapshot,
  type Diagnostic,
  type HoverResult,
  type TextChange,
} from '../../bridge/bridge';
import { getAppSnapshot, subscribeApp } from '../../app/app-store';
import { keybindMatches, parseKeybind } from '../keymap/keybind';
import { createSeloraTheme } from './cm-theme';
import { createSeloraTheme, currentTheme } from './cm-theme';
import {
  currentEditorContext,
  editorStore,
  recordEditorSelection,
  type RevealTarget,
} from './editor-store';
import { languageFor, loadLanguage } from './language-loaders';
import { completionOptions, toCmDiagnostics, watchDefinitionNavigation } from './lsp';
import { offsetToPosition, positionToOffset } from './text-offsets';
import { toastStore } from '../toasts/toast-store';

const ExternalDocumentSync = Annotation.define<boolean>();
const SELECTION_DEBOUNCE_MS = 150;
const HOVER_TIME_MS = 600;
const COMPLETION_TYPING_DELAY_MS = 250;
const ANSWER_TIMEOUT_MS = 2500;
const EMPTY_DIAGNOSTICS: readonly Diagnostic[] = [];

export type ScrollGeometry = {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
};

export type CodeMirrorPaneProps = {
  buffer: BufferSnapshot;
  onScroll?: (geometry: ScrollGeometry) => void;
  revealTarget?: RevealTarget | null;
  /** Persisted editor preferences. Omitted in isolated tests = defaults. */
  settings?: EditorPreferences;
};

/** The settings fields the pane applies to a live EditorView. */
export type EditorPreferences = {
  editor_font_size: number;
  editor_tab_size: number;
  editor_word_wrap: boolean;
  keybind_save: string;
};

export const DEFAULT_EDITOR_PREFERENCES: EditorPreferences = {
  editor_font_size: 14,
  editor_tab_size: 4,
  editor_word_wrap: false,
  keybind_save: 'Ctrl+S',
};

/** Font size, tab width, word wrap, and the rebindable save chord. */
export function editorPreferencesExtension(preferences: EditorPreferences): Extension {
  return [
    EditorView.theme({
      '.cm-scroller': { fontSize: `${preferences.editor_font_size}px` },
      '.cm-content': { tabSize: String(preferences.editor_tab_size) },
    }),
    // Inline content attributes make the applied values observable (and keep
    // the editor correct even before the theme stylesheet resolves).
    EditorView.contentAttributes.of({
      'data-editor-font-size': String(preferences.editor_font_size),
      'data-editor-tab-size': String(preferences.editor_tab_size),
      'data-editor-word-wrap': String(preferences.editor_word_wrap),
      style: `font-size: ${preferences.editor_font_size}px; tab-size: ${preferences.editor_tab_size};`,
    }),
    preferences.editor_word_wrap ? EditorView.lineWrapping : [],
  ];
}

/** The save chord resolved from settings, not a hard-coded Mod-s. */
export function saveKeymapExtension(preferences: EditorPreferences): Extension {
  const binding =
    parseKeybind(preferences.keybind_save) ?? parseKeybind(DEFAULT_EDITOR_PREFERENCES.keybind_save)!;
  return EditorView.domEventHandlers({
    keydown(event) {
      if (!keybindMatches(binding, event)) return false;
      event.preventDefault();
      editorStore.saveActive();
      return true;
    },
  });
}

type LocalBufferState = {
  text: string;
  version: number;
};

/** The local version for a buffer; falls back to the snapshot's version. */
function localVersionOf(states: Map<string, LocalBufferState>, buffer: BufferSnapshot): number {
  return states.get(buffer.buffer_id)?.version ?? buffer.version;
}

function languageKeyFor(buffer: BufferSnapshot): string {
  return `${buffer.buffer_id}:${buffer.path}`;
}

function restoreSelection(buffer: BufferSnapshot): EditorSelection | undefined {
  if (!buffer.selection) return undefined;
  return EditorSelection.single(
    positionToOffset(buffer.text, buffer.selection.anchor),
    positionToOffset(buffer.text, buffer.selection.active),
  );
}

function diagnosticsExtension(activePath: string, diagnostics: readonly Diagnostic[]): Extension {
  return [
    linter((view) => toCmDiagnostics(activePath, diagnostics, view.state.doc.toString())),
    lintGutter(),
  ];
}

type AnswerWithBuffer = { buffer_id: string };

type AnswerWaiter<T extends AnswerWithBuffer> = {
  promise: Promise<T | null>;
  cancel: () => void;
};

/** Wait for a fresh app-store answer originating from this request. */
function waitForFreshAnswer<T extends AnswerWithBuffer>(
  read: (snapshot: ReturnType<typeof getAppSnapshot>) => T | null,
  bufferId: string,
  previous: T | null,
): AnswerWaiter<T> {
  let settled = false;
  let unsubscribe: (() => void) | null = null;
  let timer: number | null = null;
  let settle: (value: T | null) => void = () => undefined;
  const promise = new Promise<T | null>((resolve) => {
    settle = resolve;
  });

  const finish = (value: T | null): void => {
    if (settled) return;
    settled = true;
    if (timer !== null) window.clearTimeout(timer);
    unsubscribe?.();
    unsubscribe = null;
    settle(value);
  };

  const check = (): void => {
    const value = read(getAppSnapshot());
    if (value && value !== previous && value.buffer_id === bufferId) finish(value);
  };

  timer = window.setTimeout(() => finish(null), ANSWER_TIMEOUT_MS);
  unsubscribe = subscribeApp(check);
  check();
  return { promise, cancel: () => finish(null) };
}

function clampOffset(text: string, offset: number): number {
  return Math.min(Math.max(Math.trunc(offset), 0), text.length);
}

function buildHoverTooltip(
  answer: HoverResult,
  docText: string,
  fallbackPos: number,
): Tooltip | null {
  if (!answer.contents || !answer.contents.trim()) return null;

  let from = clampOffset(docText, fallbackPos);
  let to = from;
  if (answer.range) {
    from = clampOffset(docText, positionToOffset(docText, answer.range.start));
    to = clampOffset(docText, positionToOffset(docText, answer.range.end));
    if (to < from) to = from;
  }

  const dom = document.createElement('div');
  dom.className = 'selora-hover-tooltip';
  dom.setAttribute('role', 'tooltip');
  const contents = document.createElement('pre');
  contents.className = 'selora-hover-contents';
  contents.textContent = answer.contents;
  dom.append(contents);

  return {
    pos: from,
    end: to > from ? to : undefined,
    above: true,
    create: () => ({ dom }),
  };
}

/**
 * Convert CodeMirror's ChangeSet into protocol changes. Ranges refer to the
 * document before the transaction (LSP convention); the replacement carries
 * the newly inserted text, so the result is independent of the post-edit doc.
 */
export function computeTextChanges(docBefore: Text, changes: ChangeSet): TextChange[] {
  const before = docBefore.toString();
  const result: TextChange[] = [];
  changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
    result.push({
      start: offsetToPosition(before, fromA),
      end: offsetToPosition(before, toA),
      replacement: inserted.toString(),
    });
  });
  return result;
}

export function CodeMirrorPane({
  buffer,
  onScroll,
  revealTarget = null,
  settings = DEFAULT_EDITOR_PREFERENCES,
}: CodeMirrorPaneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const bufferRef = useRef(buffer);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  // Local document state per buffer: the text the user is looking at and the
  // version the next `apply_editor_change` must use. The local doc stays
  // authoritative until the backend echo arrives, so switching away from a
  // buffer with edits in flight cannot drop them.
  const localBuffersRef = useRef<Map<string, LocalBufferState>>(new Map());
  const activeIdRef = useRef(buffer.buffer_id);
  const languageKeyRef = useRef('');
  const languageRequestRef = useRef(0);
  const selectionTimerRef = useRef<number | null>(null);
  const suppressEditsRef = useRef(false);
  const createStateRef = useRef<((nextBuffer: BufferSnapshot) => EditorState) | null>(null);
  const onScrollRef = useRef(onScroll);
  const languageCompartment = useRef(new Compartment());
  const diagnosticsCompartment = useRef(new Compartment());
  const editorPreferencesCompartment = useRef(new Compartment());
  const saveKeymapCompartment = useRef(new Compartment());
  const themeCompartment = useRef(new Compartment());
  const diagnosticsRef = useRef<readonly Diagnostic[]>(EMPTY_DIAGNOSTICS);

  bufferRef.current = buffer;
  onScrollRef.current = onScroll;

  const configureLanguage = (view: EditorView, nextBuffer: BufferSnapshot): void => {
    const requestId = ++languageRequestRef.current;
    const bufferId = nextBuffer.buffer_id;
    const path = nextBuffer.path;
    const language = languageFor(path);

    if (language === 'plain') {
      view.dispatch({
        effects: languageCompartment.current.reconfigure([]),
      });
      return;
    }

    void loadLanguage(language)
      .then((extension) => {
        if (
          requestId !== languageRequestRef.current ||
          viewRef.current !== view ||
          bufferRef.current.buffer_id !== bufferId ||
          bufferRef.current.path !== path
        ) {
          return;
        }
        view.dispatch({
          effects: languageCompartment.current.reconfigure(extension),
        });
      })
      .catch(() => undefined);
  };

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const initialBuffer = bufferRef.current;
    diagnosticsRef.current = getAppSnapshot().bootstrap?.diagnostics ?? EMPTY_DIAGNOSTICS;
    languageKeyRef.current = languageKeyFor(initialBuffer);
    localBuffersRef.current = new Map([
      [initialBuffer.buffer_id, { text: initialBuffer.text, version: initialBuffer.version }],
    ]);

    const scheduleSelection = (): void => {
      if (selectionTimerRef.current !== null) {
        window.clearTimeout(selectionTimerRef.current);
      }
      selectionTimerRef.current = window.setTimeout(() => {
        selectionTimerRef.current = null;
        const view = viewRef.current;
        const current = bufferRef.current;
        const context = currentEditorContext();
        if (!view || !context) return;
        const text = view.state.doc.toString();
        const main = view.state.selection.main;
        const selection = {
          anchor: offsetToPosition(text, main.anchor),
          active: offsetToPosition(text, main.head),
          version: localVersionOf(localBuffersRef.current, current),
        };
        // Tell the store right away so the agent context does not wait for the
        // protocol echo of this debounced set_selection.
        recordEditorSelection(current.buffer_id, selection);
        void setSelection(current.buffer_id, selection, context).catch(() => undefined);
      }, SELECTION_DEBOUNCE_MS);
    };

    const handleUpdate = (update: ViewUpdate): void => {
      if (update.docChanged && !suppressEditsRef.current) {
        const external = update.transactions.some(
          (transaction) => transaction.annotation(ExternalDocumentSync) === true,
        );
        const current = bufferRef.current;
        const context = currentEditorContext();
        if (!external && context) {
          const changes = computeTextChanges(update.startState.doc, update.changes);
          if (changes.length > 0) {
            // The wire contract expects the *current* backend version, which
            // is the version before this batch. Advance the local ref only
            // after capturing it, so the first edit sends 1 (not 2) and the
            // next edit predicts the version this batch will produce.
            const baseVersion = localVersionOf(localBuffersRef.current, current);
            const nextVersion = baseVersion + 1;
            localBuffersRef.current.set(current.buffer_id, {
              text: update.state.doc.toString(),
              version: nextVersion,
            });
            void applyEditorChange(current.buffer_id, baseVersion, changes, context).catch(() => {
              // The backend rejected the batch (stale version or invalid
              // range). Roll the local version back to the last version the
              // backend is known to hold instead of drifting forward.
              const entry = localBuffersRef.current.get(current.buffer_id);
              if (!entry) return;
              const snapshotVersion =
                bufferRef.current.buffer_id === current.buffer_id
                  ? bufferRef.current.version
                  : baseVersion;
              entry.version = Math.max(baseVersion, snapshotVersion);
              toastStore.pushLocal(
                'warning',
                'An editor change could not be synchronized with the workspace; the buffer version was resynced.',
              );
            });
          }
        }
      }
      if ((update.selectionSet || update.docChanged) && !suppressEditsRef.current) {
        scheduleSelection();
      }
    };

    let hoverSequence = 0;
    let cancelHover: (() => void) | null = null;
    const hoverSource = async (view: EditorView, pos: number): Promise<Tooltip | null> => {
      const current = bufferRef.current;
      const context = currentEditorContext();
      if (!context) return null;

      const docText = view.state.doc.toString();
      const requestId = ++hoverSequence;
      cancelHover?.();
      const waiter = waitForFreshAnswer(
        (snapshot) => snapshot.hoverResult,
        current.buffer_id,
        getAppSnapshot().hoverResult,
      );
      cancelHover = waiter.cancel;

      try {
        await hoverAt(current.buffer_id, offsetToPosition(docText, pos), context);
      } catch {
        waiter.cancel();
        return null;
      }

      const answer = await waiter.promise;
      if (
        !answer ||
        requestId !== hoverSequence ||
        bufferRef.current.buffer_id !== current.buffer_id
      ) {
        return null;
      }
      return buildHoverTooltip(answer, docText, pos);
    };

    let completionSequence = 0;
    let cancelCompletion: (() => void) | null = null;
    const completionSource: CompletionSource = async (context: CompletionContext) => {
      const current = bufferRef.current;
      const word = context.matchBefore(/[\w$]*/);
      if (!context.explicit && (!word || word.from === word.to)) return null;

      const envelope = currentEditorContext();
      if (!envelope) return null;

      const requestId = ++completionSequence;
      cancelCompletion?.();
      const waiter = waitForFreshAnswer(
        (snapshot) => snapshot.completionResult,
        current.buffer_id,
        getAppSnapshot().completionResult,
      );
      cancelCompletion = waiter.cancel;
      const position = offsetToPosition(context.state.doc.toString(), context.pos);

      try {
        await completionAt(current.buffer_id, position, envelope);
      } catch {
        waiter.cancel();
        return null;
      }

      const answer = await waiter.promise;
      if (
        !answer ||
        requestId !== completionSequence ||
        bufferRef.current.buffer_id !== current.buffer_id
      ) {
        return null;
      }

      const options = completionOptions(answer);
      if (options.length === 0) return null;
      const result: CmCompletionResult = {
        from: word?.from ?? context.pos,
        to: context.pos,
        options,
      };
      return result;
    };

    const requestDefinition = (view: EditorView, pos: number): void => {
      const current = bufferRef.current;
      const context = currentEditorContext();
      if (!context) return;
      void definitionAt(
        current.buffer_id,
        offsetToPosition(view.state.doc.toString(), pos),
        context,
      ).catch(() => undefined);
    };

    const definitionBinding = {
      key: 'F12',
      preventDefault: true,
      run: (view: EditorView): boolean => {
        requestDefinition(view, view.state.selection.main.head);
        return true;
      },
    };
    const definitionClick = EditorView.domEventHandlers({
      mousedown(event, view) {
        if (event.button !== 0 || (!event.ctrlKey && !event.metaKey)) return false;
        const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
        if (pos === null) return false;
        event.preventDefault();
        requestDefinition(view, pos);
        return true;
      },
    });

    const baseExtensions = [
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightSpecialChars(),
      history(),
      foldGutter(),
      drawSelection(),
      dropCursor(),
      EditorState.allowMultipleSelections.of(true),
      indentOnInput(),
      bracketMatching(),
      rectangularSelection(),
      crosshairCursor(),
      highlightActiveLine(),
      autocompletion({
        override: [completionSource],
        activateOnTypingDelay: COMPLETION_TYPING_DELAY_MS,
      }),
      hoverTooltip(hoverSource, { hoverTime: HOVER_TIME_MS, hideOnChange: true }),
      definitionClick,
      search({ top: true }),
      keymap.of([
        definitionBinding,
        { key: 'Ctrl-Space', preventDefault: true, run: startCompletion },
        ...searchKeymap,
        ...historyKeymap,
        ...completionKeymap,
        ...defaultKeymap,
        indentWithTab,
      ]),
      cmTheme,
      editorPreferencesCompartment.current.of(
        editorPreferencesExtension(settingsRef.current),
      ),
      saveKeymapCompartment.current.of(saveKeymapExtension(settingsRef.current)),
      themeCompartment.current.of(createSeloraTheme()),
      EditorView.updateListener.of(handleUpdate),
    ];

    const createState = (nextBuffer: BufferSnapshot): EditorState =>
      EditorState.create({
        doc: nextBuffer.text,
        selection: restoreSelection(nextBuffer),
        extensions: [
          ...baseExtensions,
          languageCompartment.current.of([]),
          diagnosticsCompartment.current.of(
            diagnosticsExtension(nextBuffer.path, diagnosticsRef.current),
          ),
        ],
      });
    createStateRef.current = createState;
    const state = createState(initialBuffer);

    const view = new EditorView({ state, parent: host });
    viewRef.current = view;
    configureLanguage(view, initialBuffer);

    const emitScroll = (): void => {
      const callback = onScrollRef.current;
      if (!callback) return;
      callback({
        scrollTop: view.scrollDOM.scrollTop,
        scrollHeight: view.scrollDOM.scrollHeight,
        clientHeight: view.scrollDOM.clientHeight,
      });
    };
    view.scrollDOM.addEventListener('scroll', emitScroll, { passive: true });
    const resizeObserver =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => emitScroll());
    resizeObserver?.observe(view.scrollDOM);
    const initialMeasure = window.setTimeout(emitScroll, 0);

    return () => {
      window.clearTimeout(initialMeasure);
      if (selectionTimerRef.current !== null) {
        window.clearTimeout(selectionTimerRef.current);
        selectionTimerRef.current = null;
      }
      resizeObserver?.disconnect();
      view.scrollDOM.removeEventListener('scroll', emitScroll);
      view.destroy();
      viewRef.current = null;
    };
  }, []);

  useEffect(
    () =>
      watchDefinitionNavigation({
        getDefinitionResult: () => getAppSnapshot().definitionResult,
        getActiveBufferId: () => bufferRef.current.buffer_id,
        subscribe: subscribeApp,
        openPathAtLine: (path, line) => editorStore.openPathAtLine(path, line),
      }),
    [],
  );

  // A theme switch is a live reconfiguration, not an editor rebuild: the CSS
  // variables repaint the chrome, and this swaps CodeMirror's `dark` flag so
  // its built-in affordances match the selected palette.
  useEffect(() => {
    if (typeof MutationObserver === 'undefined') return;
    let theme = currentTheme();
    const observer = new MutationObserver(() => {
      const next = currentTheme();
      if (next === theme) return;
      theme = next;
      const view = viewRef.current;
      if (!view) return;
      view.dispatch({
        effects: themeCompartment.current.reconfigure(createSeloraTheme(next)),
      });
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme'],
    });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const readDiagnostics = (): readonly Diagnostic[] =>
      getAppSnapshot().bootstrap?.diagnostics ?? EMPTY_DIAGNOSTICS;
    let lastDiagnostics = readDiagnostics();
    diagnosticsRef.current = lastDiagnostics;

    return subscribeApp(() => {
      const nextDiagnostics = readDiagnostics();
      if (nextDiagnostics === lastDiagnostics) return;
      lastDiagnostics = nextDiagnostics;
      diagnosticsRef.current = nextDiagnostics;

      const view = viewRef.current;
      if (!view) return;
      view.dispatch({
        effects: diagnosticsCompartment.current.reconfigure(
          diagnosticsExtension(bufferRef.current.path, nextDiagnostics),
        ),
      });
    });
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: [
        editorPreferencesCompartment.current.reconfigure(
          editorPreferencesExtension(settings),
        ),
        saveKeymapCompartment.current.reconfigure(saveKeymapExtension(settings)),
      ],
    });
  }, [
    settings.editor_font_size,
    settings.editor_tab_size,
    settings.editor_word_wrap,
    settings.keybind_save,
  ]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: diagnosticsCompartment.current.reconfigure(
        diagnosticsExtension(buffer.path, diagnosticsRef.current),
      ),
    });
  }, [buffer.buffer_id, buffer.path]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;

    const switched = activeIdRef.current !== buffer.buffer_id;
    const cached = localBuffersRef.current.get(buffer.buffer_id);
    activeIdRef.current = buffer.buffer_id;

    if (switched) {
      const createState = createStateRef.current;
      if (!createState) return;
      // Edits whose echo has not arrived yet are newer than the incoming
      // snapshot, so restore the cached local document instead of rebuilding
      // from the (older) protocol text.
      const nextBuffer: BufferSnapshot =
        cached && cached.version > buffer.version
          ? { ...buffer, text: cached.text, version: cached.version }
          : buffer;
      localBuffersRef.current.set(buffer.buffer_id, {
        text: nextBuffer.text,
        version: nextBuffer.version,
      });
      suppressEditsRef.current = true;
      view.setState(createState(nextBuffer));
      suppressEditsRef.current = false;
      languageKeyRef.current = languageKeyFor(nextBuffer);
      configureLanguage(view, nextBuffer);
      return;
    }

    // `buffer_changed` advances the version before its snapshot text lands.
    // Only a strictly newer remote snapshot may replace the document; an
    // equal-version echo (or stale older one) must never undo keystrokes that
    // are already in flight.
    if (buffer.version > (cached?.version ?? 0)) {
      localBuffersRef.current.set(buffer.buffer_id, {
        text: buffer.text,
        version: buffer.version,
      });
      const viewText = view.state.doc.toString();
      if (viewText !== buffer.text) {
        const anchor = Math.min(view.state.selection.main.anchor, buffer.text.length);
        view.dispatch({
          changes: { from: 0, to: view.state.doc.length, insert: buffer.text },
          selection: { anchor },
          annotations: [ExternalDocumentSync.of(true), Transaction.addToHistory.of(false)],
        });
      }
    } else if (!cached) {
      localBuffersRef.current.set(buffer.buffer_id, {
        text: view.state.doc.toString(),
        version: buffer.version,
      });
    }

    const languageKey = languageKeyFor(buffer);
    if (languageKeyRef.current !== languageKey) {
      languageKeyRef.current = languageKey;
      configureLanguage(view, buffer);
    }
  }, [buffer.buffer_id, buffer.text, buffer.version, buffer.language, buffer.path]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    if (!revealTarget || revealTarget.path !== buffer.path) return;

    const lineNumber = Math.min(Math.max(Math.trunc(revealTarget.line), 1), view.state.doc.lines);
    const line = view.state.doc.line(lineNumber);
    view.dispatch({
      selection: { anchor: line.from },
      effects: EditorView.scrollIntoView(line.from, { y: 'center' }),
    });
    view.focus();
    editorStore.clearReveal(revealTarget.requestId);
  }, [buffer.path, buffer.text, buffer.version, revealTarget]);

  return <div className="editor-code-host" ref={hostRef} data-buffer-id={buffer.buffer_id} />;
}
