export function requirePersistedKey(status) {
  if (status?.has_key !== true || status?.source !== 'keyring') {
    throw new Error('Walkthrough requires a persisted Credential Manager key; environment-only keys do not qualify.');
  }
}

export function requireCleanRuntime(exceptions, errors) {
  if (exceptions.length || errors.length) {
    throw new Error(`Runtime gate failed: ${exceptions.length} exception(s), ${errors.length} console/log error(s).`);
  }
}
