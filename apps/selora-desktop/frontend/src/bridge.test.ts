import { describe, expect, it } from 'vitest';
import type { BootstrapState } from './bridge';

describe('bootstrap contract', () => { it('keeps the reconnect snapshot fields explicit', () => { const state: Pick<BootstrapState, 'protocol_version' | 'core_instance_id' | 'snapshot_sequence'> = { protocol_version: 1, core_instance_id: 'core-1', snapshot_sequence: 4 }; expect(state.snapshot_sequence).toBe(4); }); });

