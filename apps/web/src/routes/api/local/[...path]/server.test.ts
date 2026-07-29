import { describe, expect, it } from 'vitest';
import { pathFor } from './+server';

describe('local agent browser proxy allowlist', () => {
  it('allows the profile and queue operations used by the dashboard', () => {
    expect(pathFor('GET', 'printers')).toBe('/v1/local/printers');
    expect(pathFor('GET', 'printers/prt_1/queue')).toBe('/v1/local/printers/prt_1/queue');
    expect(pathFor('PUT', 'printers/prt_1/exposure')).toBe(
      '/v1/local/printers/prt_1/exposure'
    );
    expect(pathFor('POST', 'printers/prt_1/test-page')).toBe(
      '/v1/local/printers/prt_1/test-page'
    );
    expect(pathFor('POST', 'profiles/profile_1/validate')).toBe(
      '/v1/local/profiles/profile_1/validate'
    );
    expect(pathFor('DELETE', 'printers/prt_1/profiles/profile_1')).toBe(
      '/v1/local/printers/prt_1/profiles/profile_1'
    );
  });

  it('does not expose capture sessions, tokens, or native profile blobs to the browser', () => {
    expect(pathFor('POST', 'printers/prt_1/profile-capture-sessions')).toBeNull();
    expect(pathFor('POST', 'profile-capture-sessions/session_1/complete')).toBeNull();
    expect(pathFor('DELETE', 'profile-capture-sessions/session_1')).toBeNull();
    expect(pathFor('GET', 'profiles/profile_1/native-blob')).toBeNull();
    expect(pathFor('POST', 'printers/prt_1/profiles')).toBeNull();
    expect(pathFor('PUT', 'printers/prt_1/profiles/profile_1')).toBeNull();
  });

  it('rejects methods and paths outside the explicit allowlist', () => {
    expect(pathFor('PATCH', 'printers/prt_1/exposure')).toBeNull();
    expect(pathFor('POST', 'profiles/profile_1/validate/extra')).toBeNull();
    expect(pathFor('GET', `printers/${'x'.repeat(513)}/queue`)).toBeNull();
  });
});
