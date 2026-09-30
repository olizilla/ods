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
          mediaType: 'application/vnd.oci.empty.v1+json',
          digest: 'sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a',
          size: 2,
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

  // What a key's headers must be follows from its shape, so a dataset bump, which regenerates
  // expected-keys.json with new digests and a new version, changes no expectation here.
  // A release's blobs are its Parquet layers, the manifest itself and the empty config the
  // manifest names (`{}`): the files carry their own provenance, and `datapackage.json` is
  // never packed.
  const OCI_MANIFEST_TYPE = 'application/vnd.oci.image.manifest.v1+json';
  const IMMUTABLE = 'public, max-age=31536000, immutable';
  const CONTRACT_SHAPES: { shape: RegExp; contentType: string; cacheControl: string }[] = [
    // a blob, by digest
    { shape: /^v2\/[^/]+\/blobs\/sha256\/[0-9a-f]{64}$/, contentType: 'application/octet-stream', cacheControl: IMMUTABLE },
    // a versioned manifest tag: <date>_<semver>
    { shape: /^v2\/[^/]+\/manifests\/\d{4}-\d{2}-\d{2}_\d+\.\d+\.\d+$/, contentType: OCI_MANIFEST_TYPE, cacheControl: IMMUTABLE },
    // a moving tag: <date> or latest
    { shape: /^v2\/[^/]+\/manifests\/(\d{4}-\d{2}-\d{2}|latest)$/, contentType: OCI_MANIFEST_TYPE, cacheControl: 'no-cache' },
  ];

  it('derives correct content-type and cache-control for every key shape in expected-keys.json against static contract', () => {
    expect(expectedKeys.length).toBeGreaterThan(0);
    for (const key of expectedKeys) {
      // Every key in expected-keys.json has a shape the contract knows.
      const matches = CONTRACT_SHAPES.filter((c) => c.shape.test(key));
      expect(matches.length, `no contract shape for ${key}`).toBe(1);
      const expected = matches[0];

      const headers = deriveHeaders(`/${key}`, 'etag123');
      expect(headers.get('content-type'), key).toBe(expected.contentType);
      expect(headers.get('cache-control'), key).toBe(expected.cacheControl);
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

  it('serves /schema/ods-datapackage.v1.json with application/schema+json and immutable cache-control', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/schema/ods-datapackage.v1.json'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('application/schema+json');
    expect(res.headers.get('cache-control')).toBe('public, max-age=31536000, immutable');
    const json = await res.json() as Record<string, unknown>;
    expect(json['$id']).toBe('https://ods.fyi/schema/ods-datapackage.v1.json');
    expect(json['$schema']).toBe('http://json-schema.org/draft-07/schema#');
  });
});
