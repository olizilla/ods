import { describe, it, expect } from 'vitest';
import worker, { pathToKey } from '../../src/index';
import { env } from 'cloudflare:test';

describe('Router & Path-to-Key Mapping', () => {
  it('maps /releases.json to releases.json', () => {
    expect(pathToKey('/releases.json')).toBe('releases.json');
    expect(pathToKey('releases.json')).toBe('releases.json');
  });

  it('maps blobs with sha256: to sha256/', () => {
    expect(
      pathToKey('/v2/ods-data/blobs/sha256:083525aae40231344be2fa3f14064ca2e455b0bcce9868d0e9051558ebbf5b0a')
    ).toBe('v2/ods-data/blobs/sha256/083525aae40231344be2fa3f14064ca2e455b0bcce9868d0e9051558ebbf5b0a');
  });

  it('maps manifests by tag directly', () => {
    expect(pathToKey('/v2/ods-data/manifests/2026-07-31_1.0.1')).toBe(
      'v2/ods-data/manifests/2026-07-31_1.0.1'
    );
    expect(pathToKey('/v2/ods-data/manifests/2026-07-31')).toBe(
      'v2/ods-data/manifests/2026-07-31'
    );
    expect(pathToKey('/v2/ods-data/manifests/latest')).toBe(
      'v2/ods-data/manifests/latest'
    );
  });

  it('maps manifests by digest to blobs/sha256/<hex>', () => {
    expect(
      pathToKey('/v2/ods-data/manifests/sha256:0f2a000000000000000000000000000000000000000000000000000000000000')
    ).toBe('v2/ods-data/blobs/sha256/0f2a000000000000000000000000000000000000000000000000000000000000');
  });

  it('maps friendly paths and latest paths untouched', () => {
    expect(pathToKey('/2026-07-31/1.0.1/orgs.parquet')).toBe(
      '2026-07-31/1.0.1/orgs.parquet'
    );
    expect(pathToKey('/latest/orgs.parquet')).toBe('latest/orgs.parquet');
  });

  it('preserves multi-segment repository names opaquely without parsing', () => {
    expect(
      pathToKey('/v2/homebrew/core/jq/blobs/sha256:1111111111111111111111111111111111111111111111111111111111111111')
    ).toBe('v2/homebrew/core/jq/blobs/sha256/1111111111111111111111111111111111111111111111111111111111111111');
  });

  it('serves GET / root text/plain landing page', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('text/plain');
    const text = await res.text();
    expect(text).toContain('ods.fyi — NHS Organisation Data Service, as Parquet.');
    expect(text).toContain('duckdb -c');
    expect(text).toContain('Releases: https://ods.fyi/releases.json');
    expect(text).toContain('Source:   https://github.com/olizilla/ods');
  });

  it('serves GET /v2/ returning 200 and {} unauthenticated', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/v2/'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('application/json');
    const body = await res.json();
    expect(body).toEqual({});
  });
});
