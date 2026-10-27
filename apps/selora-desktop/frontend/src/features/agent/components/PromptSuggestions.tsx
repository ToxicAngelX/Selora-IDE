// Selora IDE — Animated Prompt Suggestion Pills (Hero & In-Feed Presence)
//
// Categories: "Fix Bug", "Add Tests", "Refactor Code", "Explain Flow"
// Styled with subtle animated gradient border shimmer, category badge pills,
// and hover elevation compliant with DESIGN_TOKENS.md and calm motion.
import React from 'react';
import {
  AlertCircle,
  CheckCircle,
  Code2,
  Lightbulb,
} from '../../../components/icons';

export interface SuggestionEntry {
  id: string;
  category: 'Fix Bug' | 'Add Tests' | 'Refactor Code' | 'Explain Flow';
  prompt: string;
  subtext: string;
  icon: typeof AlertCircle;
  categoryClass: string;
}

export const PROMPT_SUGGESTIONS: SuggestionEntry[] = [
  {
    id: 'sugg-fix-bug',
    category: 'Fix Bug',
    prompt: 'Review the workspace for bugs',
    subtext: 'Search the code, trace edge cases, and report real defects',
    icon: AlertCircle,
    categoryClass: 'sugg-cat-fix',
  },
  {
    id: 'sugg-add-tests',
    category: 'Add Tests',
    prompt: 'Write unit tests for a module',
    subtext: 'Read the source and add assertions for happy paths & boundaries',
    icon: CheckCircle,
    categoryClass: 'sugg-cat-tests',
  },
  {
    id: 'sugg-refactor',
    category: 'Refactor Code',
    prompt: 'Refactor for clean architecture & modularity',
    subtext: 'Simplify complex functions and clarify type contracts',
    icon: Code2,
    categoryClass: 'sugg-cat-refactor',
  },
  {
    id: 'sugg-explain',
    category: 'Explain Flow',
    prompt: 'Explain this workspace architecture',
    subtext: 'Trace execution flow, state lifecycle, and IPC bridge',
    icon: Lightbulb,
    categoryClass: 'sugg-cat-explain',
  },
];

export type PromptSuggestionsProps = {
  onSelectPrompt: (prompt: string) => void;
  layout?: 'grid' | 'inline-pills';
  className?: string;
};

export const PromptSuggestions: React.FC<PromptSuggestionsProps> = ({
  onSelectPrompt,
  layout = 'grid',
  className = '',
}) => {
  return (
    <div
      className={`prompt-suggestions-container layout-${layout} ${className}`}
      role="region"
      aria-label="Suggested prompts"
    >
      {PROMPT_SUGGESTIONS.map((item) => {
        const Icon = item.icon;
        return (
          <button
            key={item.id}
            type="button"
            className="prompt-shimmer-pill"
            onClick={() => onSelectPrompt(item.prompt)}
            title={item.subtext}
          >
            <div className="shimmer-border-layer" aria-hidden="true" />
            <div className="shimmer-pill-content">
              <span className={`category-tag ${item.categoryClass}`}>
                <Icon size={12} className="category-icon" />
                <span className="category-name">{item.category}</span>
              </span>
              <span className="prompt-label">{item.prompt}</span>
            </div>
          </button>
        );
      })}
    </div>
  );
};
