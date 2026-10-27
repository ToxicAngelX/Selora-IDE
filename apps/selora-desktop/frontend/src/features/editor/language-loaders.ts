import type { Extension } from '@codemirror/state';

export type LanguageId =
  | 'plain'
  | 'rust'
  | 'javascript'
  | 'jsx'
  | 'typescript'
  | 'tsx'
  | 'css'
  | 'json'
  | 'markdown';

type ImportedLanguageId = Exclude<LanguageId, 'plain'>;

function extensionOf(path: string): string {
  const filename = path.split(/[\\/]/).pop() ?? path;
  const dot = filename.lastIndexOf('.');
  return dot === -1 ? '' : filename.slice(dot + 1).toLowerCase();
}

export function languageFor(path: string): LanguageId {
  switch (extensionOf(path)) {
    case 'rs':
      return 'rust';
    case 'js':
      return 'javascript';
    case 'jsx':
      return 'jsx';
    case 'ts':
      return 'typescript';
    case 'tsx':
      return 'tsx';
    case 'css':
      return 'css';
    case 'json':
      return 'json';
    case 'md':
    case 'markdown':
      return 'markdown';
    default:
      return 'plain';
  }
}

const languageLoaders: Record<ImportedLanguageId, () => Promise<Extension>> = {
  rust: async () => (await import('@codemirror/lang-rust')).rust(),
  javascript: async () => (await import('@codemirror/lang-javascript')).javascript(),
  jsx: async () => (await import('@codemirror/lang-javascript')).javascript({ jsx: true }),
  typescript: async () =>
    (await import('@codemirror/lang-javascript')).javascript({ typescript: true }),
  tsx: async () =>
    (await import('@codemirror/lang-javascript')).javascript({ typescript: true, jsx: true }),
  css: async () => (await import('@codemirror/lang-css')).css(),
  json: async () => (await import('@codemirror/lang-json')).json(),
  markdown: async () => (await import('@codemirror/lang-markdown')).markdown(),
};

export async function loadLanguage(id: LanguageId): Promise<Extension> {
  if (id === 'plain') return [];
  return languageLoaders[id]();
}
