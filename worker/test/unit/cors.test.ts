import { describe, it, expect, beforeEach } from 'vitest';
import worker, { clearManifestCache, sha256Hex } from '../../src/index';
import { env } from 'cloudflare:test';

describe('CORS Configuration', () => {
  beforeEach(async () => {
    clearManifestCache();
    const layerBytes = new Uint8Array([1, 2, 3, 4]);
    const layerDigest = await sha256Hex(layerBytes);
    await env.BUCKET.put(`v2/ods-data/blobs/sha256/${layerDigest}`, layerBytes);

    const manifestObj = {
      schemaVersion: 2,
      mediaType: 'application/vnd.oci.image.manifest.v1+json',
      config: { digest: 'sha256:1111', size: 10 },
      layers: [
        {
          mediaType: 'application/vnd.apache.parquet',
          digest: `sha256:${layerDigest}`,
          size: layerBytes.length,
          annotations: {
            'org.opencontainers.image.title': 'orgs.parquet',
          },
        },
      ],
    };
    await env.BUCKET.put(
      'v2/ods-data/manifests/latest',
      new TextEncoder().encode(JSON.stringify(manifestObj))
    );
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
