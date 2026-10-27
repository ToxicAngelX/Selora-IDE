// Selora IDE — Markdown Message & Code Block Parser (Cursor/Windsurf parity)
import React from 'react';
import { CodeBlock } from './CodeBlock';

export type MarkdownMessageProps = {
  content: string;
  className?: string;
};

type Block =
  | { type: 'text'; content: string }
  | { type: 'code'; code: string; language: string };

function parseBlocks(markdown: string): Block[] {
  if (!markdown) return [];

  const codeFenceRegex = /```([a-zA-Z0-9_-]*)\r?\n([\s\S]*?)```/g;
  const blocks: Block[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = codeFenceRegex.exec(markdown)) !== null) {
    const textBefore = markdown.slice(lastIndex, match.index);
    if (textBefore.trim()) {
      blocks.push({ type: 'text', content: textBefore });
    }

    blocks.push({
      type: 'code',
      language: match[1] || 'text',
      code: match[2],
    });

    lastIndex = match.index + match[0].length;
  }

  const remainingText = markdown.slice(lastIndex);
  if (remainingText.trim() || blocks.length === 0) {
    blocks.push({ type: 'text', content: remainingText });
  }

  return blocks;
}

function renderFormattedInline(text: string): React.ReactNode {
  // Simple inline parser for `code`, **bold**, and plain text
  const inlineRegex = /(`[^`]+`|\*\*[^*]+\*\*)/g;
  const parts = text.split(inlineRegex);

  return parts.map((part, i) => {
    if (part.startsWith('`') && part.endsWith('`') && part.length > 2) {
      return (
        <code key={i} className="inline-code-pill">
          {part.slice(1, -1)}
        </code>
      );
    }
    if (part.startsWith('**') && part.endsWith('**') && part.length > 4) {
      return (
        <strong key={i} className="message-bold">
          {part.slice(2, -2)}
        </strong>
      );
    }
    return part;
  });
}

export const MarkdownMessage: React.FC<MarkdownMessageProps> = ({ content, className = '' }) => {
  // If no code blocks, render the standard paragraph directly
  if (!content.includes('```')) {
    return (
      <div className={`message-text-content ${className}`}>
        <p className="message-paragraph">{renderFormattedInline(content)}</p>
      </div>
    );
  }

  const blocks = parseBlocks(content);

  return (
    <div className={`message-text-content ${className}`}>
      {blocks.map((block, idx) => {
        if (block.type === 'code') {
          return (
            <CodeBlock
              key={idx}
              code={block.code}
              language={block.language}
            />
          );
        }
        return (
          <p key={idx} className="message-paragraph">
            {renderFormattedInline(block.content)}
          </p>
        );
      })}
    </div>
  );
};
