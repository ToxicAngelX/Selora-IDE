// Selora IDE — Live workspace search (DESIGN §3.3)
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type FC,
} from 'react';
import {
  currentEnvelopeContext,
  getAppSnapshot,
  subscribeApp,
} from '../../app/app-store';
import {
  searchFiles,
  type BootstrapState,
  type SearchHit,
} from '../../bridge/bridge';
import { CloseIcon, Search } from '../../components/icons';
import { editorStore } from '../editor/editor-store';
import { FileTypeIcon } from './FileTreeView';

const MAX_VISIBLE_HITS = 200;

const fileName = (path: string): string => path.split(/[\\/]/).pop() || path;
const parentPath = (path: string): string => {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join('/');
};

function formatError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

type SearchError = { code: string | null; message: string };

function formatSearchError(cause: unknown): SearchError {
  if (typeof cause === 'object' && cause !== null) {
    const candidate = cause as { code?: unknown; message?: unknown };
    if (typeof candidate.message === 'string') {
      return {
        code: typeof candidate.code === 'string' ? candidate.code : null,
        message: candidate.message,
      };
    }
  }
  return { code: null, message: formatError(cause) };
}

function queryMatches(
  resultQuery: { query: string; case_sensitive: boolean; whole_word: boolean; use_regex: boolean },
  query: string,
  caseSensitive: boolean,
  wholeWord: boolean,
  useRegex: boolean,
): boolean {
  return (
    resultQuery.query === query &&
    resultQuery.case_sensitive === caseSensitive &&
    resultQuery.whole_word === wholeWord &&
    resultQuery.use_regex === useRegex
  );
}

function renderHighlightedPreview(hit: SearchHit, query: string, caseSensitive: boolean) {
  const preview = hit.preview;
  const compare = (value: string) => (caseSensitive ? value : value.toLowerCase());
  const needle = compare(query);
  let start = Math.max(0, hit.column - 1);

  if (compare(preview.slice(start, start + query.length)) !== needle) {
    start = compare(preview).indexOf(needle);
  }
  if (start < 0 || !query) return <span>{preview}</span>;

  return (
    <span className="search-code-preview">
      {preview.slice(0, start)}
      <mark className="search-highlight-mark">{preview.slice(start, start + query.length)}</mark>
      {preview.slice(start + query.length)}
    </span>
  );
}

export interface SearchPanelViewProps {
  bootstrap?: BootstrapState;
  onNavigateToMatch?: (filePath: string, line: number) => void;
}

export const SearchPanelView: FC<SearchPanelViewProps> = ({ bootstrap, onNavigateToMatch }) => {
  const app = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const liveBootstrap = app.bootstrap ?? bootstrap;
  const workspaceRoot = liveBootstrap?.workspace.root ?? null;

  const [query, setQuery] = useState('');
  const [matchCase, setMatchCase] = useState(false);
  const [matchWholeWord, setMatchWholeWord] = useState(false);
  const [matchRegex, setMatchRegex] = useState(false);
  const [collapsedFiles, setCollapsedFiles] = useState<Set<string>>(new Set());
  const [selectedHit, setSelectedHit] = useState<string | null>(null);
  const [isSearching, setIsSearching] = useState(false);
  const [error, setError] = useState<SearchError | null>(null);
  const lastSuccessfulResults = useRef(app.searchResults);

  const trimmedQuery = query.trim();

  useEffect(() => {
    if (app.searchResults) lastSuccessfulResults.current = app.searchResults;
  }, [app.searchResults]);

  const matchingResults = useMemo(() => {
    const results = app.searchResults;
    if (!results || !trimmedQuery) return null;
    return queryMatches(results.query, query, matchCase, matchWholeWord, matchRegex)
      ? results
      : null;
  }, [app.searchResults, query, trimmedQuery, matchCase, matchWholeWord, matchRegex]);

  const currentResults =
    error?.code === 'invalid_query' ? lastSuccessfulResults.current : matchingResults;

  useEffect(() => {
    if (!workspaceRoot || !trimmedQuery) {
      setIsSearching(false);
      setError(null);
      return;
    }

    let cancelled = false;
    setIsSearching(true);
    setError(null);

    const timer = setTimeout(() => {
      const context = currentEnvelopeContext();
      if (!context) {
        if (!cancelled) {
          setIsSearching(false);
          setError({
            code: null,
            message: 'Selora is still connecting. Try the search again in a moment.',
          });
        }
        return;
      }

      void searchFiles(
        {
          query,
          case_sensitive: matchCase,
          whole_word: matchWholeWord,
          use_regex: matchRegex,
        },
        context,
      ).catch((cause: unknown) => {
        if (cancelled) return;
        setIsSearching(false);
        setError(formatSearchError(cause));
      });
    }, 250);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [workspaceRoot, query, trimmedQuery, matchCase, matchWholeWord, matchRegex]);

  useEffect(() => {
    if (currentResults) setIsSearching(false);
  }, [currentResults]);

  // Another surface (palette, AI action) can prefill the query. The existing
  // debounce effect runs the search once the state lands.
  useEffect(() => {
    const handleSearchQuery = (event: Event) => {
      const detail = (event as CustomEvent<{ query?: unknown }>).detail;
      if (detail && typeof detail.query === 'string') setQuery(detail.query);
    };
    window.addEventListener('selora:search-query', handleSearchQuery);
    return () => window.removeEventListener('selora:search-query', handleSearchQuery);
  }, []);

  useEffect(() => {
    setCollapsedFiles(new Set());
    setSelectedHit(null);
  }, [query, matchCase, matchWholeWord, matchRegex]);

  const allHits = currentResults?.hits ?? [];
  const visibleHits = useMemo(() => allHits.slice(0, MAX_VISIBLE_HITS), [allHits]);
  const groups = useMemo(() => {
    const byPath = new Map<string, SearchHit[]>();
    for (const hit of visibleHits) {
      const existing = byPath.get(hit.path);
      if (existing) existing.push(hit);
      else byPath.set(hit.path, [hit]);
    }
    return Array.from(byPath.entries()).map(([path, hits]) => ({ path, hits }));
  }, [visibleHits]);

  const toggleGroup = (path: string) => {
    setCollapsedFiles((previous) => {
      const next = new Set(previous);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const openHit = (hit: SearchHit) => {
    setSelectedHit(hit.path + ':' + hit.line + ':' + hit.column);
    onNavigateToMatch?.(hit.path, hit.line);
    void editorStore.openPathAtLine(hit.path, hit.line);
  };

  if (!workspaceRoot) {
    return (
      <div className="search-panel-view" data-testid="search-panel-view">
        <div className="search-empty-state">
          <div className="search-empty-icon" aria-hidden="true">
            <Search size={22} />
          </div>
          <h3>Open a folder to search</h3>
          <p>Workspace search is available after a folder is open.</p>
        </div>
      </div>
    );
  }

  const shownCount = visibleHits.length;
  const totalCount = allHits.length;
  const previewQuery = currentResults?.query.query ?? query;
  const previewCaseSensitive = currentResults?.query.case_sensitive ?? matchCase;
  const summary = trimmedQuery
    ? isSearching && !currentResults
      ? 'Searching…'
      : currentResults
        ? String(shownCount) +
          (totalCount > shownCount ? ' of ' + String(totalCount) : '') +
          (totalCount === 1 ? ' result in ' : ' results in ') +
          String(groups.length) +
          (groups.length === 1 ? ' file' : ' files')
        : 'Type to search workspace'
    : 'Type to search workspace';

  return (
    <div className="search-panel-view" data-testid="search-panel-view">
      <div className="search-controls-box">
        <div className="search-input-row">
          <div className="search-input-field-wrapper">
            <span className="search-field-prefix-icon" aria-hidden="true">
              <Search size={13} />
            </span>
            <input
              className="search-field-input"
              type="text"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search the workspace"
              aria-label="Search"
              spellCheck={false}
              autoComplete="off"
            />
            {query ? (
              <button
                type="button"
                className="search-clear-btn"
                title="Clear Search"
                aria-label="Clear Search"
                onClick={() => setQuery('')}
              >
                <CloseIcon size={13} />
              </button>
            ) : null}

            <div className="search-modifiers-group">
              <button
                type="button"
                className={'modifier-btn' + (matchCase ? ' is-active' : '')}
                title="Match Case"
                aria-label="Match Case"
                aria-pressed={matchCase}
                onClick={() => setMatchCase((previous) => !previous)}
              >
                Aa
              </button>
              <button
                type="button"
                className={'modifier-btn' + (matchWholeWord ? ' is-active' : '')}
                title="Match Whole Word"
                aria-label="Match Whole Word"
                aria-pressed={matchWholeWord}
                onClick={() => setMatchWholeWord((previous) => !previous)}
              >
                {'\\b'}
              </button>
              <button
                type="button"
                className={'modifier-btn' + (matchRegex ? ' is-active' : '')}
                title="Regex"
                aria-label="Regex"
                aria-pressed={matchRegex}
                onClick={() => setMatchRegex((previous) => !previous)}
              >
                {'.*'}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="search-summary-bar" aria-live="polite">
        <span className="search-count-label">{summary}</span>
        {totalCount > MAX_VISIBLE_HITS ? (
          <span className="search-limit-note">
            Showing first {MAX_VISIBLE_HITS}
          </span>
        ) : null}
      </div>

      {error ? (
        <div className="search-status-message is-error" role="status" aria-live="polite">
          {error.message}
        </div>
      ) : null}

      <div className="search-results-tree" role="region" aria-label="Search results">
        {trimmedQuery && !isSearching && currentResults && allHits.length === 0 ? (
          <div className="search-results-empty">No results found.</div>
        ) : null}

        {groups.map((group) => {
          const isCollapsed = collapsedFiles.has(group.path);
          return (
            <div className="search-file-group" key={group.path}>
              <button
                type="button"
                className="search-file-header-row"
                onClick={() => toggleGroup(group.path)}
                title={group.path}
                aria-expanded={!isCollapsed}
              >
                <span className="search-file-chevron" aria-hidden="true">
                  {isCollapsed ? <span>›</span> : <span>⌄</span>}
                </span>
                <FileTypeIcon name={fileName(group.path)} />
                <span className="search-file-name">{fileName(group.path)}</span>
                <span className="search-file-path-hint">{parentPath(group.path)}</span>
                <span className="search-count-badge" title={String(group.hits.length) + ' matches'}>
                  {group.hits.length}
                </span>
              </button>

              {!isCollapsed ? (
                <div className="search-match-lines-list">
                  {group.hits.map((hit) => {
                    const hitKey = hit.path + ':' + hit.line + ':' + hit.column;
                    return (
                      <button
                        type="button"
                        className={'search-match-line-row' + (selectedHit === hitKey ? ' is-selected' : '')}
                        key={hitKey}
                        onClick={() => openHit(hit)}
                        title={hit.path + ':' + hit.line + ':' + hit.column}
                        aria-label={'Open ' + hit.path + ' at line ' + String(hit.line)}
                      >
                        <span className="match-line-number" title={'Line ' + String(hit.line) + ', column ' + String(hit.column)}>
                          {hit.line}:{hit.column}
                        </span>
                        <div className="match-preview-slot">
                          {renderHighlightedPreview(hit, previewQuery, previewCaseSensitive)}
                        </div>
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
};

