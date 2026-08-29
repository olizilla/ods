import { describe, it, expect } from 'vitest';
import worker from '../../src/index';
import { env } from 'cloudflare:test';

describe('Error Formats & Vocabularies', () => {
  it('Acceptance 19: 404 under /v2/ returns OCI JSON error shape', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/v2/ods-data/manifests/nope'), env);
    expect(res.status).toBe(404);
    expect(res.headers.get('content-type')).toContain('application/json');
    const body = await res.json();
    expect(body).toEqual({
      errors: [
        {
          code: 'NAME_UNKNOWN',
          message: 'manifest unknown',
          detail: null,
        },
      ],
    });
  });

  it('Acceptance 19: 404 outside /v2/ returns plain text error message', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/nope'), env);
    expect(res.status).toBe(404);
    expect(res.headers.get('content-type')).toContain('text/plain');
    const text = await res.text();
    expect(text).toBe('Not found. See https://ods.fyi/ for what lives here.\n');
  });

  it('Rejects unsupported methods with 405 Method Not Allowed', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/releases.json', { method: 'POST', body: 'test' }),
      env
    );
    expect(res.status).toBe(405);
  });
});
