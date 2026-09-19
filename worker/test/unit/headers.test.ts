import { describe, it, expect, beforeEach } from 'vitest';
import worker, { clearManifestCache, deriveHeaders, sha256Hex } from '../../src/index';
import { env } from 'cloudflare:test';
import expectedKeys from '../fixtures/expected-keys.json';

describe('Derived Headers & Caching Policies', () => {
  const sampleLayerBytes = new Uint8Array([1, 2, 3, 4]);
  let sampleLayerDigest: string;
  let sampleManifestBytes: Uint8Array;
  let sampleManifestDigest: string;

  beforeEach(async () => {
    clearManifestCache();
    sampleLayerDigest = await sha256Hex(sampleLayerBytes);
    sampleManifestBytes = new TextEncoder().encode(
      JSON.stringify({
        schemaVersion: 2,
        mediaType: 'application/vnd.oci.image.manifest.v1+json',
        config: {
          mediaType: 'application/vnd.fyi.ods.provenance.v1+json',
          digest: 'sha256:083525aae40231344be2fa3f14064ca2e455b0bcce9868d0e9051558ebbf5b0a',
          size: 1024,
        },
        layers: [
          {
            mediaType: 'application/vnd.apache.parquet',
            digest: `sha256:${sampleLayerDigest}`,
            size: sampleLayerBytes.length,
            annotations: {
              'org.opencontainers.image.title': 'orgs.parquet',
            },
          },
        ],
      })
    );
    sampleManifestDigest = await sha256Hex(sampleManifestBytes);

    // Populate manifest tags and blobs in R2
    await env.BUCKET.put('v2/ods-data/manifests/2026-07-31_0.1.0', sampleManifestBytes);
    await env.BUCKET.put('v2/ods-data/manifests/latest', sampleManifestBytes);
    await env.BUCKET.put(`v2/ods-data/blobs/sha256/${sampleManifestDigest}`, sampleManifestBytes);
    await env.BUCKET.put(`v2/ods-data/blobs/sha256/${sampleLayerDigest}`, sampleLayerBytes);
    await env.BUCKET.put('releases.json', new TextEncoder().encode('{"releases":[]}'));
  });

  const EXPECTED_CONTRACT: Record<string, { contentType: string; cacheControl: string }> = {
    'v2/ods-data/blobs/sha256/0b19e9910a525c3ceb11ed7b821e1c390d1b18cac24be0f4ae5a477a9409690f': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/091145d75e23b23a22618ab63bc442bbf07cf362fcaca5c9b8a1b2dccba630f5': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/865ae1b1b0ad804aa57abc90de9805be6727516afd83cb86c808e33c78461ea0': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/434089f072f166fade4426cfc24c1e360c0273b1c562f410832890a2bb8fd623': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/5dd6b71bb3898e49df0c3653b6f8ad56ce5acc5ef6a24a69966b7371adb03381': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/f589910447402a3a1547051e3c30e4e94afee98e36fbd78230379b870d49cd0f': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/manifests/2026-07-31': {
      contentType: 'application/vnd.oci.image.manifest.v1+json',
      cacheControl: 'no-cache',
    },
    'v2/ods-data/manifests/2026-07-31_0.1.0': {
      contentType: 'application/vnd.oci.image.manifest.v1+json',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/manifests/latest': {
      contentType: 'application/vnd.oci.image.manifest.v1+json',
      cacheControl: 'no-cache',
    },
  };

  it('derives correct content-type and cache-control for every key shape in expected-keys.json against static contract', () => {
    // 1. Assert all keys in expectedKeys exist in contract
    expect(expectedKeys.slice().sort()).toEqual(Object.keys(EXPECTED_CONTRACT).sort());

    for (const key of expectedKeys) {
      const path = `/${key}`;
      const headers = deriveHeaders(path, 'etag123');
      const expected = EXPECTED_CONTRACT[key];

      expect(headers.get('content-type')).toBe(expected.contentType);
      expect(headers.get('cache-control')).toBe(expected.cacheControl);
    }
  });

  it('serves GET /releases.json with application/json and no-cache', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/releases.json'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('application/json');
    expect(res.headers.get('cache-control')).toBe('no-cache');
  });

  it('Acceptance 8: manifest by digest and by tag return identical responses and manifest media type', async () => {
    const digestHex = sampleManifestDigest;
    const digestUrl = `https://ods.fyi/v2/ods-data/manifests/sha256:${digestHex}`;
    const tagUrl = `https://ods.fyi/v2/ods-data/manifests/2026-07-31_0.1.0`;

    const resDigest = await worker.fetch(new Request(digestUrl), env);
    const resTag = await worker.fetch(new Request(tagUrl), env);

    expect(resDigest.status).toBe(200);
    expect(resTag.status).toBe(200);

    expect(resDigest.headers.get('content-type')).toBe('application/vnd.oci.image.manifest.v1+json');
    expect(resTag.headers.get('content-type')).toBe('application/vnd.oci.image.manifest.v1+json');

    expect(resDigest.headers.get('docker-content-digest')).toBe(`sha256:${digestHex}`);
    expect(resTag.headers.get('docker-content-digest')).toBe(`sha256:${digestHex}`);

    const bytesDigest = new Uint8Array(await resDigest.arrayBuffer());
    const bytesTag = new Uint8Array(await resTag.arrayBuffer());
    expect(bytesDigest).toEqual(bytesTag);
  });

  it('serves resolved versioned path with immutable cache and layer digest', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/2026-07-31/0.1.0/orgs.parquet'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('application/vnd.apache.parquet');
    expect(res.headers.get('cache-control')).toBe('public, max-age=31536000, immutable');
    expect(res.headers.get('etag')).toBe(`"sha256:${sampleLayerDigest}"`);
    expect(res.headers.get('docker-content-digest')).toBe(`sha256:${sampleLayerDigest}`);
    expect(res.headers.get('content-length')).toBe(sampleLayerBytes.length.toString());
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes).toEqual(sampleLayerBytes);
  });

  it('serves resolved latest path with max-age=300 and layer digest', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/latest/orgs.parquet'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('application/vnd.apache.parquet');
    expect(res.headers.get('cache-control')).toBe('public, max-age=300');
    expect(res.headers.get('etag')).toBe(`"sha256:${sampleLayerDigest}"`);
    expect(res.headers.get('docker-content-digest')).toBe(`sha256:${sampleLayerDigest}`);
    expect(res.headers.get('content-length')).toBe(sampleLayerBytes.length.toString());
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes).toEqual(sampleLayerBytes);
  });

  it('serves resolved root convenience path with max-age=300 and layer digest', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/orgs.parquet'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('application/vnd.apache.parquet');
    expect(res.headers.get('cache-control')).toBe('public, max-age=300');
    expect(res.headers.get('etag')).toBe(`"sha256:${sampleLayerDigest}"`);
    expect(res.headers.get('docker-content-digest')).toBe(`sha256:${sampleLayerDigest}`);
    expect(res.headers.get('content-length')).toBe(sampleLayerBytes.length.toString());
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes).toEqual(sampleLayerBytes);
  });

  it('returns plain text 404 for missing file in latest manifest', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/latest/missing.parquet'), env);
    expect(res.status).toBe(404);
    expect(res.headers.get('content-type')).toContain('text/plain');
    const text = await res.text();
    expect(text).toContain('Not found. See https://ods.fyi/');
  });

  it('Acceptance 12: If-None-Match returns 304 Not Modified when ETag matches', async () => {
    const getRes = await worker.fetch(new Request('https://ods.fyi/2026-07-31/0.1.0/orgs.parquet'), env);
    expect(getRes.status).toBe(200);
    const etag = getRes.headers.get('etag');
    expect(etag).toBeTruthy();

    const condRes = await worker.fetch(
      new Request('https://ods.fyi/2026-07-31/0.1.0/orgs.parquet', {
        headers: { 'if-none-match': etag! },
      }),
      env
    );
    expect(condRes.status).toBe(304);
  });

  it('serves /schema/releases.v1.json with application/schema+json and immutable cache-control', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/schema/releases.v1.json'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('application/schema+json');
    expect(res.headers.get('cache-control')).toBe('public, max-age=31536000, immutable');
    const json = await res.json() as Record<string, unknown>;
    expect(json['$id']).toBe('https://ods.fyi/schema/releases.v1.json');
    expect(json['$schema']).toBe('https://json-schema.org/draft/2020-12/schema');
  });
});
