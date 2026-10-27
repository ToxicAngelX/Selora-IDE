// Selora IDE — CodeBlock Unit Tests
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { CodeBlock } from './CodeBlock';

describe('CodeBlock Component', () => {
  beforeEach(() => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
      writable: true,
      configurable: true,
    });
  });

  it('renders language badge, line numbers, and formatted lines', () => {
    const code = `fn main() {\n    println!("Hello Selora!");\n}`;
    const { getByText } = render(<CodeBlock code={code} language="rust" />);

    expect(getByText('RUST')).toBeTruthy();
    expect(getByText('3 lines')).toBeTruthy();
    expect(getByText('1')).toBeTruthy();
    expect(getByText('2')).toBeTruthy();
    expect(getByText('3')).toBeTruthy();
    expect(getByText('fn')).toBeTruthy();
  });

  it('handles copy button click and clipboard write', async () => {
    const code = `const answer = 42;`;
    const { getByLabelText, findByText } = render(<CodeBlock code={code} language="typescript" />);

    const copyBtn = getByLabelText('Copy code');
    fireEvent.click(copyBtn);

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(code);
    expect(await findByText('Copied')).toBeTruthy();
  });

  it('dispatches selora:apply-code custom event when Apply to Editor is clicked', () => {
    const dispatchSpy = vi.spyOn(window, 'dispatchEvent');
    const code = `pub fn verify() -> bool { true }`;

    const { getByLabelText, getByText } = render(<CodeBlock code={code} language="rust" />);

    const applyBtn = getByLabelText('Apply to Editor');
    fireEvent.click(applyBtn);

    expect(dispatchSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        type: 'selora:apply-code',
      })
    );
    expect(getByText('Applied')).toBeTruthy();
    dispatchSpy.mockRestore();
  });
});
