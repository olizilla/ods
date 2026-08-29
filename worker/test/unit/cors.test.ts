import { describe, it, expect, beforeEach } from 'vitest';
import worker from '../../src/index';
import { env } from 'cloudflare:test';

describe('CORS Configuration', () => {
  beforeEach(async () => {
    await env.BUCKET.put('latest/orgs.parquet', new Uint8Array([1, 2, 3, 4]));
  });

  it('Acceptance 15: OPTIONS returns 204 with full CORS headers', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/latest/orgs.parquet', {
        method: 'OPTIONS',
      }),
      env
    );
    expect(res.status).toBe(204);
    expect(res.headers.get('access-control-allow-origin')).toBe('*');
    expect(res.headers.get('access-control-allow-methods')).toBe('GET, HEAD, OPTIONS');
    expect(res.headers.get('access-control-allow-headers')).toContain('Range');
    expect(res.headers.get('access-control-expose-headers')).toBe(
      'Content-Range, Content-Length, Accept-Ranges, ETag, Docker-Content-Digest'
    );
    expect(res.headers.get('access-control-max-age')).toBe('86400');
  });

  it('Exposes critical headers to JavaScript / DuckDB-Wasm on GET', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/latest/orgs.parquet'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('access-control-allow-origin')).toBe('*');
    expect(res.headers.get('access-control-expose-headers')).toBe(
      'Content-Range, Content-Length, Accept-Ranges, ETag, Docker-Content-Digest'
    );
  });

  it('Exposes CORS headers on /v2/ ping response', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/v2/'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('access-control-allow-origin')).toBe('*');
    expect(res.headers.get('access-control-allow-methods')).toBe('GET, HEAD, OPTIONS');
    expect(res.headers.get('access-control-expose-headers')).toContain('Docker-Content-Digest');
  });
});
