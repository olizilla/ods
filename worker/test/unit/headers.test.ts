import { describe, it, expect, beforeEach } from 'vitest';
import worker, { deriveHeaders, sha256Hex } from '../../src/index';
import { env } from 'cloudflare:test';
import expectedKeys from '../fixtures/expected-keys.json';

describe('Derived Headers & Caching Policies', () => {
  const sampleManifestBytes = new TextEncoder().encode(
    JSON.stringify({
      schemaVersion: 2,
      mediaType: 'application/vnd.oci.image.manifest.v1+json',
      config: {
        mediaType: 'application/vnd.fyi.ods.provenance.v1+json',
        digest: 'sha256:083525aae40231344be2fa3f14064ca2e455b0bcce9868d0e9051558ebbf5b0a',
        size: 1024,
      },
      layers: [],
    })
  );

  beforeEach(async () => {
    // Populate manifest tag and blob in R2
    const manifestDigest = await sha256Hex(sampleManifestBytes);
    await env.BUCKET.put('v2/ods-data/manifests/2026-07-31_1.0.1', sampleManifestBytes);
    await env.BUCKET.put(`v2/ods-data/blobs/sha256/${manifestDigest}`, sampleManifestBytes);
    await env.BUCKET.put('2026-07-31/1.0.1/orgs.parquet', new Uint8Array([1, 2, 3, 4]));
    await env.BUCKET.put('latest/orgs.parquet', new Uint8Array([5, 6, 7, 8]));
    await env.BUCKET.put('releases.json', new TextEncoder().encode('{"releases":[]}'));
  });

  const EXPECTED_CONTRACT: Record<string, { contentType: string; cacheControl: string }> = {
    '2026-07-31/1.0.1/NOTES.md': {
      contentType: 'text/markdown; charset=utf-8',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    '2026-07-31/1.0.1/_provenance.json': {
      contentType: 'application/json',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    '2026-07-31/1.0.1/datapackage.json': {
      contentType: 'application/json',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    '2026-07-31/1.0.1/orgs.parquet': {
      contentType: 'application/vnd.apache.parquet',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    '2026-07-31/1.0.1/roles.parquet': {
      contentType: 'application/vnd.apache.parquet',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'latest/NOTES.md': {
      contentType: 'text/markdown; charset=utf-8',
      cacheControl: 'no-cache',
    },
    'latest/_provenance.json': {
      contentType: 'application/json',
      cacheControl: 'no-cache',
    },
    'latest/datapackage.json': {
      contentType: 'application/json',
      cacheControl: 'no-cache',
    },
    'latest/orgs.parquet': {
      contentType: 'application/vnd.apache.parquet',
      cacheControl: 'no-cache',
    },
    'latest/roles.parquet': {
      contentType: 'application/vnd.apache.parquet',
      cacheControl: 'no-cache',
    },
    'v2/ods-data/blobs/sha256/0b19e9910a525c3ceb11ed7b821e1c390d1b18cac24be0f4ae5a477a9409690f': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/5dd6b71bb3898e49df0c3653b6f8ad56ce5acc5ef6a24a69966b7371adb03381': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/87fa60a579ab77f8bf1d9ee24566d5afe572229299081e9e679acbee78bee23c': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/c67e925e689a477fe47d148b8213523e8dc7d96a5636726f52b58faa7ed72ed6': {
      contentType: 'application/octet-stream',
      cacheControl: 'public, max-age=31536000, immutable',
    },
    'v2/ods-data/blobs/sha256/dafe84307d307fdada314a55037318b7e241336c71b6a7667b468787e99340b4': {
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
    'v2/ods-data/manifests/2026-07-31_1.0.1': {
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
    const digestHex = await sha256Hex(sampleManifestBytes);
    const digestUrl = `https://ods.fyi/v2/ods-data/manifests/sha256:${digestHex}`;
    const tagUrl = `https://ods.fyi/v2/ods-data/manifests/2026-07-31_1.0.1`;

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

  it('Acceptance 12: If-None-Match returns 304 Not Modified when ETag matches', async () => {
    const getRes = await worker.fetch(new Request('https://ods.fyi/2026-07-31/1.0.1/orgs.parquet'), env);
    expect(getRes.status).toBe(200);
    const etag = getRes.headers.get('etag');
    expect(etag).toBeTruthy();

    const condRes = await worker.fetch(
      new Request('https://ods.fyi/2026-07-31/1.0.1/orgs.parquet', {
        headers: { 'if-none-match': etag! },
      }),
      env
    );
    expect(condRes.status).toBe(304);
  });
});
