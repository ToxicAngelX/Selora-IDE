// Selora IDE — Premium AI IDE Code Block (Cursor/Windsurf parity)
//
// Features:
// - Header bar with language badge (e.g. RUST, TYPESCRIPT)
// - Copy button with smooth checkmark transition
// - "Apply to Editor" button with custom event dispatch & feedback
// - Line numbers with faint styling
// - Clean syntax colors (monochrome/syntax palette tokens)
import React, { useState } from 'react';
import { Copy, Check, Code2, Sparkles } from '../../../components/icons';

export type CodeBlockProps = {
  code: string;
  language?: string;
  showLineNumbers?: boolean;
};

// Lightweight tokenizer for syntax highlighting without external heavy dependencies
function highlightLine(line: string): React.ReactNode {
  if (!line) return <span>&nbsp;</span>;

  // Single-line comments (// or #)
  const commentMatch = line.match(/^(\s*)((\/\/|#).*)$/);
  if (commentMatch) {
    return (
      <>
        <span>{commentMatch[1]}</span>
        <span className="syn-comment">{commentMatch[2]}</span>
      </>
    );
  }

  // Tokenize words, strings, numbers, punctuation
  const tokenRegex = /(".*?"|'.*?'|`.*?`|\b(?:fn|function|pub|struct|impl|enum|type|interface|class|let|const|var|return|if|else|match|for|while|async|await|import|export|from|use|mod|crate|mut|true|false|null|undefined|self|None|Some|Ok|Err)\b|\b(?:String|str|bool|u8|u16|u32|u64|usize|i32|i64|f32|f64|Option|Result|Vec|Promise|Record|Array|Map|Set|number|boolean|any|void)\b|\b\d+(?:\.\d+)?\b|[{}()[\];,.:]|=>|->|::|\S+|\s+)/g;

  const tokens: React.ReactNode[] = [];
  let match: RegExpExecArray | null;
  let idx = 0;

  const keywords = new Set([
    'fn', 'function', 'pub', 'struct', 'impl', 'enum', 'type', 'interface',
    'class', 'let', 'const', 'var', 'return', 'if', 'else', 'match', 'for',
    'while', 'async', 'await', 'import', 'export', 'from', 'use', 'mod',
    'crate', 'mut', 'self', 'def',
  ]);

  const constants = new Set(['true', 'false', 'null', 'undefined', 'None', 'Some', 'Ok', 'Err']);

  const types = new Set([
    'String', 'str', 'bool', 'u8', 'u16', 'u32', 'u64', 'usize', 'i32', 'i64',
    'f32', 'f64', 'Option', 'Result', 'Vec', 'Promise', 'Record', 'Array',
    'Map', 'Set', 'number', 'boolean', 'any', 'void',
  ]);

  while ((match = tokenRegex.exec(line)) !== null) {
    const text = match[0];
    const key = `token-${idx++}`;

    if (text.startsWith('"') || text.startsWith("'") || text.startsWith('`')) {
      tokens.push(<span key={key} className="syn-string">{text}</span>);
    } else if (keywords.has(text)) {
      tokens.push(<span key={key} className="syn-keyword">{text}</span>);
    } else if (constants.has(text)) {
      tokens.push(<span key={key} className="syn-const">{text}</span>);
    } else if (types.has(text)) {
      tokens.push(<span key={key} className="syn-type">{text}</span>);
    } else if (/^\d+(\.\d+)?$/.test(text)) {
      tokens.push(<span key={key} className="syn-number">{text}</span>);
    } else if (/^[{}()[\];,.:]$/.test(text)) {
      tokens.push(<span key={key} className="syn-punct">{text}</span>);
    } else {
      tokens.push(<span key={key}>{text}</span>);
    }
  }

  return tokens.length > 0 ? tokens : line;
}

export const CodeBlock: React.FC<CodeBlockProps> = ({
  code,
  language = 'TEXT',
  showLineNumbers = true,
}) => {
  const [copied, setCopied] = useState(false);
  const [applied, setApplied] = useState(false);

  const cleanLang = (language || 'CODE').toUpperCase().trim();
  const trimmedCode = code.replace(/\r\n/g, '\n').replace(/^\n+|\n+$/g, '');
  const lines = trimmedCode.split('\n');

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(trimmedCode);
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    } catch {
      setCopied(false);
    }
  };

  const handleApply = () => {
    if (typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('selora:apply-code', {
          detail: { code: trimmedCode, language: cleanLang.toLowerCase() },
        })
      );
    }
    setApplied(true);
    setTimeout(() => setApplied(false), 2000);
  };

  return (
    <div className="selora-code-block-root">
      {/* Code Header Bar */}
      <div className="code-block-header">
        <div className="code-header-left">
          <Code2 size={12} className="code-lang-icon" />
          <span className="code-lang-badge">{cleanLang}</span>
          <span className="code-line-count-badge">
            {lines.length} {lines.length === 1 ? 'line' : 'lines'}
          </span>
        </div>

        <div className="code-header-actions">
          {/* Apply to Editor Button */}
          <button
            type="button"
            className={`code-action-btn apply-btn ${applied ? 'is-applied' : ''}`}
            onClick={handleApply}
            title="Apply this snippet directly to active editor"
            aria-label="Apply to Editor"
          >
            {applied ? <Check size={11} className="text-ok" /> : <Sparkles size={11} />}
            <span>{applied ? 'Applied' : 'Apply to Editor'}</span>
          </button>

          {/* Copy Button */}
          <button
            type="button"
            className={`code-action-btn copy-btn ${copied ? 'is-copied' : ''}`}
            onClick={handleCopy}
            title={copied ? 'Copied code to clipboard!' : 'Copy code block'}
            aria-label={copied ? 'Copied code' : 'Copy code'}
          >
            {copied ? <Check size={11} className="text-ok" /> : <Copy size={11} />}
            <span>{copied ? 'Copied' : 'Copy'}</span>
          </button>
        </div>
      </div>

      {/* Code Scrollable Area */}
      <div className="code-block-container">
        <pre className="code-block-pre">
          <code className={`language-${cleanLang.toLowerCase()}`}>
            {lines.map((lineText, lineIdx) => (
              <div key={lineIdx} className="code-line-row">
                {showLineNumbers && (
                  <span className="code-line-number" aria-hidden="true">
                    {lineIdx + 1}
                  </span>
                )}
                <span className="code-line-content">{highlightLine(lineText)}</span>
              </div>
            ))}
          </code>
        </pre>
      </div>
    </div>
  );
};
