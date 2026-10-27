// Selora IDE — Live diagnostics and Problems panel (Phase Z2/Z4)
import { useMemo, useState, useSyncExternalStore } from 'react';
import { getAppSnapshot, subscribeApp } from '../../app/app-store';
import type { Diagnostic } from '../../bridge/bridge';
import { Icon } from '../../components/Icon';
import { editorStore } from '../editor/editor-store';

export type ProblemSeverity = Diagnostic['severity'];
type FilterType = 'all' | ProblemSeverity;

const SEVERITY_ORDER: ProblemSeverity[] = ['error', 'warning', 'info', 'hint'];
const SEVERITY_LABEL: Record<ProblemSeverity, string> = {
  error: 'Errors',
  warning: 'Warnings',
  info: 'Information',
  hint: 'Hints',
};

const fileName = (path: string): string => path.split(/[\\/]/).pop() || path;
const parentPath = (path: string): string => {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join('/');
};

function severityIcon(severity: ProblemSeverity) {
  if (severity === 'error') return 'severity-error' as const;
  if (severity === 'warning') return 'severity-warning' as const;
  return 'severity-info' as const;
}

export function ProblemsPanel() {
  const app = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const diagnostics = app.bootstrap?.diagnostics ?? [];

  const [activeFilter, setActiveFilter] = useState<FilterType>('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedKey, setSelectedKey] = useState<string | null>(null);

  const counts = useMemo(
    () => ({
      all: diagnostics.length,
      error: diagnostics.filter((item) => item.severity === 'error').length,
      warning: diagnostics.filter((item) => item.severity === 'warning').length,
      info: diagnostics.filter((item) => item.severity === 'info').length,
      hint: diagnostics.filter((item) => item.severity === 'hint').length,
    }),
    [diagnostics],
  );

  const filteredProblems = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    return diagnostics.filter((item) => {
      if (activeFilter !== 'all' && item.severity !== activeFilter) return false;
      if (!query) return true;
      return (
        item.message.toLowerCase().includes(query) ||
        item.path.toLowerCase().includes(query)
      );
    });
  }, [diagnostics, activeFilter, searchQuery]);

  const groups = useMemo(() => {
    const byPath = new Map<string, Diagnostic[]>();
    for (const item of filteredProblems) {
      const existing = byPath.get(item.path);
      if (existing) existing.push(item);
      else byPath.set(item.path, [item]);
    }

    return Array.from(byPath.entries())
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([path, items]) => ({
        path,
        total: items.length,
        severities: SEVERITY_ORDER.map((severity) => ({
          severity,
          items: items.filter((item) => item.severity === severity),
        })).filter((group) => group.items.length > 0),
      }));
  }, [filteredProblems]);

  const openProblem = (problem: Diagnostic, key: string) => {
    setSelectedKey(key);
    // Protocol/LSP diagnostics are 0-based; the reveal target is 1-based.
    void editorStore.openPathAtLine(problem.path, Math.max(1, problem.line + 1));
  };

  return (
    <div className="dock-panel problems-panel" data-testid="problems-panel">
      <div className="dock-toolbar problems-toolbar">
        <div className="dock-toolbar-left">
          <div className="problems-filter-tabs" role="tablist" aria-label="Problem severity filters">
            <button
              type="button"
              role="tab"
              aria-selected={activeFilter === 'all'}
              className={'problems-filter-tab' + (activeFilter === 'all' ? ' is-active' : '')}
              onClick={() => setActiveFilter('all')}
            >
              <span>All</span>
              <span className="problems-badge problems-badge-all">{counts.all}</span>
            </button>
            {SEVERITY_ORDER.map((severity) => (
              <button
                type="button"
                role="tab"
                aria-selected={activeFilter === severity}
                className={'problems-filter-tab' + (activeFilter === severity ? ' is-active' : '')}
                onClick={() => setActiveFilter(severity)}
                key={severity}
              >
                <span>{SEVERITY_LABEL[severity]}</span>
                <span className={'problems-badge problems-badge-' + severity}>{counts[severity]}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="dock-toolbar-right">
          <div className="problems-search-wrap">
            <span className="problems-search-icon" aria-hidden="true">
              <Icon name="search" size={12} />
            </span>
            <input
              type="text"
              className="problems-search-input"
              placeholder="Filter problems…"
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              aria-label="Filter problems"
              spellCheck={false}
            />
            {searchQuery ? (
              <button
                type="button"
                className="problems-search-clear"
                onClick={() => setSearchQuery('')}
                aria-label="Clear filter"
                title="Clear filter"
              >
                <Icon name="close" size={10} />
              </button>
            ) : null}
          </div>
        </div>
      </div>

      <div className="problems-list" role="list">
        {filteredProblems.length === 0 ? (
          <div className="problems-empty-view">
            {diagnostics.length === 0 ? (
              <>
                <div className="problems-empty-icon">
                  <Icon name="check" size={24} />
                </div>
                <span>No problems have been detected in the workspace.</span>
              </>
            ) : (
              <>
                <div className="problems-empty-icon">
                  <Icon name="search" size={20} />
                </div>
                <span>No problems match the current filter.</span>
                <button
                  type="button"
                  className="dock-btn"
                  onClick={() => {
                    setActiveFilter('all');
                    setSearchQuery('');
                  }}
                  title="Clear problem filters"
                >
                  Clear Filters
                </button>
              </>
            )}
          </div>
        ) : (
          groups.map((group) => (
            <div className="problems-file-group" key={group.path}>
              <div className="problems-file-header" title={group.path}>
                <span className="problems-file-name">{fileName(group.path)}</span>
                <span className="problems-file-path">{parentPath(group.path)}</span>
                <span className="problems-file-count">{group.total}</span>
              </div>

              {group.severities.map((severityGroup) => (
                <div className="problems-severity-group" key={severityGroup.severity}>
                  <div className="problems-severity-header">
                    <span className={'problem-icon-' + severityGroup.severity} aria-hidden="true">
                      <Icon name={severityIcon(severityGroup.severity)} size={11} />
                    </span>
                    <span>{SEVERITY_LABEL[severityGroup.severity]}</span>
                    <span className="problems-severity-count">{severityGroup.items.length}</span>
                  </div>

                  {severityGroup.items.map((problem, index) => {
                    const key =
                      problem.path +
                      ':' +
                      problem.line +
                      ':' +
                      problem.column +
                      ':' +
                      problem.message +
                      ':' +
                      String(index);
                    return (
                      <div className="problem-row-item" role="listitem" key={key}>
                        <button
                          type="button"
                          className={
                            'problem-row severity-' +
                            problem.severity +
                            (selectedKey === key ? ' is-selected' : '')
                          }
                          onClick={() => openProblem(problem, key)}
                          title={problem.path + ':' + problem.line + ':' + problem.column}
                          aria-label={
                            'Open ' +
                            problem.path +
                            ' at line ' +
                            String(problem.line) +
                            ', column ' +
                            String(problem.column)
                          }
                        >
                          <span className="problem-icon-cell" aria-hidden="true">
                            <span className={'problem-icon-' + problem.severity}>
                              <Icon name={severityIcon(problem.severity)} size={14} />
                            </span>
                          </span>

                          <span className="problem-main">
                            <span className="problem-desc">{problem.message}</span>
                          </span>

                          <span className="problem-meta">
                            <span className="problem-location">
                              {problem.line}:{problem.column}
                            </span>
                          </span>
                        </button>
                      </div>
                    );
                  })}
                </div>
              ))}
            </div>
          ))
        )}
      </div>
    </div>
  );
}

