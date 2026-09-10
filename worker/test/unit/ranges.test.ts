import { describe, it, expect, beforeEach } from 'vitest';
import worker, { clearManifestCache, sha256Hex } from '../../src/index';
import { env } from 'cloudflare:test';

describe('Range Requests & HEAD Handlers', () => {
  const dummy1000Bytes = new Uint8Array(1000);
  for (let i = 0; i < 1000; i++) {
    dummy1000Bytes[i] = i % 256;
  }

  let manifestBytes: Uint8Array;

  beforeEach(async () => {
    clearManifestCache();
    const dummyDigest = await sha256Hex(dummy1000Bytes);
    await env.BUCKET.put(`v2/ods-data/blobs/sha256/${dummyDigest}`, dummy1000Bytes);

    const manifestObj = {
      schemaVersion: 2,
      mediaType: 'application/vnd.oci.image.manifest.v1+json',
      config: { digest: 'sha256:1111', size: 10 },
      layers: [
        {
          mediaType: 'application/vnd.apache.parquet',
          digest: `sha256:${dummyDigest}`,
          size: dummy1000Bytes.length,
          annotations: {
            'org.opencontainers.image.title': 'orgs.parquet',
          },
        },
      ],
    };
    manifestBytes = new TextEncoder().encode(JSON.stringify(manifestObj));
    await env.BUCKET.put('v2/ods-data/manifests/2026-07-31_1.0.1', manifestBytes);
    await env.BUCKET.put('v2/ods-data/manifests/latest', manifestBytes);
  });

  it('handles standard slice range (bytes=0-15)', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/2026-07-31/1.0.1/orgs.parquet', {
        headers: { range: 'bytes=0-15' },
      }),
      env
    );
    expect(res.status).toBe(206);
    expect(res.headers.get('content-range')).toBe('bytes 0-15/1000');
    expect(res.headers.get('content-length')).toBe('16');
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes.length).toBe(16);
    expect(bytes[0]).toBe(0);
    expect(bytes[15]).toBe(15);
  });

  it('Acceptance 9: handles suffix range (bytes=-8)', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/2026-07-31/1.0.1/orgs.parquet', {
        headers: { range: 'bytes=-8' },
      }),
      env
    );
    expect(res.status).toBe(206);
    expect(res.headers.get('content-range')).toBe('bytes 992-999/1000');
    expect(res.headers.get('content-length')).toBe('8');
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes.length).toBe(8);
  });

  it('Acceptance 9: handles open-ended range (bytes=900-)', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/2026-07-31/1.0.1/orgs.parquet', {
        headers: { range: 'bytes=900-' },
      }),
      env
    );
    expect(res.status).toBe(206);
    expect(res.headers.get('content-range')).toBe('bytes 900-999/1000');
    expect(res.headers.get('content-length')).toBe('100');
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes.length).toBe(100);
  });

  it('Acceptance 9: returns 416 for unsatisfiable range', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/2026-07-31/1.0.1/orgs.parquet', {
        headers: { range: 'bytes=5000-6000' },
      }),
      env
    );
    expect(res.status).toBe(416);
    expect(res.headers.get('content-range')).toBe('bytes */1000');
  });

  it('Acceptance 14: multi-range requests get 200 with full body', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/2026-07-31/1.0.1/orgs.parquet', {
        headers: { range: 'bytes=0-10, 20-30' },
      }),
      env
    );
    expect(res.status).toBe(200);
    expect(res.headers.get('content-length')).toBe('1000');
    expect(res.headers.get('content-range')).toBeNull();
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes.length).toBe(1000);
  });

  it('HEAD returns 200 with no body and full Content-Length', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/2026-07-31/1.0.1/orgs.parquet', {
        method: 'HEAD',
      }),
      env
    );
    expect(res.status).toBe(200);
    expect(res.headers.get('content-length')).toBe('1000');
    const text = await res.text();
    expect(text).toBe('');
  });

  it('HEAD on manifest tag computes Docker-Content-Digest', async () => {
    const digestHex = await sha256Hex(manifestBytes);
    const res = await worker.fetch(
      new Request('https://ods.fyi/v2/ods-data/manifests/2026-07-31_1.0.1', {
        method: 'HEAD',
      }),
      env
    );
    expect(res.status).toBe(200);
    expect(res.headers.get('docker-content-digest')).toBe(`sha256:${digestHex}`);
    expect(res.headers.get('content-type')).toBe('application/vnd.oci.image.manifest.v1+json');
    expect(res.headers.get('content-length')).toBe(manifestBytes.length.toString());
    const text = await res.text();
    expect(text).toBe('');
  });

  it('handles range request on latest/orgs.parquet returning 206 slice', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/latest/orgs.parquet', {
        headers: { range: 'bytes=0-15' },
      }),
      env
    );
    expect(res.status).toBe(206);
    expect(res.headers.get('content-range')).toBe('bytes 0-15/1000');
    expect(res.headers.get('content-length')).toBe('16');
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes.length).toBe(16);
    expect(bytes[0]).toBe(0);
    expect(bytes[15]).toBe(15);
  });

  it('handles range request on bare root orgs.parquet returning 206 slice', async () => {
    const res = await worker.fetch(
      new Request('https://ods.fyi/orgs.parquet', {
        headers: { range: 'bytes=100-199' },
      }),
      env
    );
    expect(res.status).toBe(206);
    expect(res.headers.get('content-range')).toBe('bytes 100-199/1000');
    expect(res.headers.get('content-length')).toBe('100');
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes.length).toBe(100);
    expect(bytes[0]).toBe(100);
    expect(bytes[99]).toBe(199);
  });
});
