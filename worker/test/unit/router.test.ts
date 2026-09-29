import { describe, it, expect, beforeEach } from 'vitest';
import worker, { pathToKey, clearManifestCache, sha256Hex } from '../../src/index';
import { env } from 'cloudflare:test';
// The page is built from these figures, so it names this release's date.
import siteRelease from '../../../site/src/data/release.json';

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
    expect(text).toContain(
      'ods.fyi — All the organisations and sites in the NHS Organisation Data Service,\nas queryable & verifiable Parquet files. An independent project.'
    );
    expect(text).toContain('duckdb -c');
    expect(text).toContain('Releases:  https://ods.fyi/releases.json');
    expect(text).toContain('Code:      https://github.com/olizilla/ods');
    expect(text).toContain('Data from: NHS England, via NHS TRUD, under the Open Government Licence');
  });

  it('serves GET /v2/ returning 200 and {} unauthenticated', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/v2/'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('application/json');
    const body = await res.json();
    expect(body).toEqual({});
  });

  it('serves GET / as the site page when Accept includes text/html', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/', { headers: { Accept: 'text/html' } }),
      env
    );
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('text/html');
    expect(res.headers.get('cache-control')).toBe('public, max-age=300');
    const text = await res.text();
    expect(text).toContain('ods.fyi');
    expect(text).toContain(siteRelease.release.date);
  });

  it('serves GET / as ROOT_TEXT when Accept is */*', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/', { headers: { Accept: '*/*' } }),
      env
    );
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('text/plain');
    const text = await res.text();
    expect(text).toContain('Releases:  https://ods.fyi/releases.json');
  });

  it('serves /_astro/ files through ASSETS, with their own type, ahead of the dataset lookup', async () => {
    // The hashed filename changes whenever the styles do, so read it from the page itself.
    const page = await worker.fetch(
      new Request('https://ods.fyi/', { headers: { Accept: 'text/html' } }),
      env
    );
    const href = (await page.text()).match(/href="(\/_astro\/[^"]+\.css)"/)?.[1];
    expect(href).toBeTruthy();
    const res = await worker.fetch(new Request(`https://ods.fyi${href}`), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('cache-control')).toBe('public, max-age=31536000, immutable');
    expect(res.headers.get('content-type')).toContain('text/css');
  });

  it('serves the unhashed site icons through ASSETS instead of falling through to the dataset lookup', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/favicon-32.png'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('cache-control')).toBe('public, max-age=300');
    expect(res.headers.get('content-type')).toContain('image/png');
  });
});

describe('Root convenience path still resolves through the latest manifest', () => {
  const sampleLayerBytes = new Uint8Array([1, 2, 3, 4]);
  let sampleLayerDigest: string;

  beforeEach(async () => {
    clearManifestCache();
    sampleLayerDigest = await sha256Hex(sampleLayerBytes);
    const sampleManifestBytes = new TextEncoder().encode(
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
    await env.BUCKET.put('v2/ods-data/manifests/latest', sampleManifestBytes);
    await env.BUCKET.put(`v2/ods-data/blobs/sha256/${sampleLayerDigest}`, sampleLayerBytes);
  });

  it('serves GET /orgs.parquet through the latest manifest, unaffected by the site routes', async () => {
    const res = await worker.fetch(new Request('https://ods.fyi/orgs.parquet'), env);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('application/vnd.apache.parquet');
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes).toEqual(sampleLayerBytes);
  });
});
