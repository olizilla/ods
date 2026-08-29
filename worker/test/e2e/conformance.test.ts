import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { Miniflare } from 'miniflare';
import esbuild from 'esbuild';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
import { execFileSync, execSync } from 'node:child_process';
import crypto from 'node:crypto';

function findConformanceBinary(): string | null {
  // 1. Explicit environment variable override
  if (process.env.OCI_CONFORMANCE_BIN && fs.existsSync(process.env.OCI_CONFORMANCE_BIN)) {
    return process.env.OCI_CONFORMANCE_BIN;
  }

  // 2. PATH resolution via which/where
  try {
    const whichOut = execSync('which conformance 2>/dev/null', { encoding: 'utf-8' }).trim();
    if (whichOut && fs.existsSync(whichOut)) {
      return whichOut;
    }
  } catch {}

  // 3. Check $(go env GOPATH)/bin/conformance
  try {
    const gopath = execSync('go env GOPATH 2>/dev/null', { encoding: 'utf-8' }).trim();
    const candidate = path.join(gopath, 'bin', 'conformance');
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  } catch {}

  // 4. Check ~/go/bin/conformance
  const homeCandidate = path.join(os.homedir(), 'go', 'bin', 'conformance');
  if (fs.existsSync(homeCandidate)) {
    return homeCandidate;
  }

  return null;
}

describe('Official OCI Distribution Spec Conformance Test', () => {
  let mf: Miniflare;
  let serverUrl: string;
  let port: string;
  let manifestDigestHex: string;
  let provDigestHex: string;
  let parquetDigestHex: string;

  beforeAll(async () => {
    const buildResult = esbuild.buildSync({
      entryPoints: [path.resolve(__dirname, '../../src/index.ts')],
      bundle: true,
      format: 'esm',
      write: false,
      target: 'esnext',
    });
    const scriptContent = buildResult.outputFiles[0].text;

    mf = new Miniflare({
      modules: true,
      script: scriptContent,
      compatibilityDate: '2024-12-30',
      compatibilityFlags: ['nodejs_compat'],
      r2Buckets: ['BUCKET'],
      port: 8788,
    });

    const url = await mf.ready;
    serverUrl = url.origin;
    port = url.port;

    const bucket = await mf.getR2Bucket('BUCKET');

    // Seed release objects
    const provBytes = new TextEncoder().encode(
      JSON.stringify({
        _type: 'ods_provenance',
        trud_release_date: '2026-07-31',
        dataset_version: '1.0.1',
      })
    );
    provDigestHex = crypto.createHash('sha256').update(provBytes).digest('hex');

    const parquetBytes = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
    parquetDigestHex = crypto.createHash('sha256').update(parquetBytes).digest('hex');

    const manifestObj = {
      schemaVersion: 2,
      mediaType: 'application/vnd.oci.image.manifest.v1+json',
      config: {
        mediaType: 'application/vnd.fyi.ods.provenance.v1+json',
        digest: `sha256:${provDigestHex}`,
        size: provBytes.length,
      },
      layers: [
        {
          mediaType: 'application/vnd.apache.parquet',
          digest: `sha256:${parquetDigestHex}`,
          size: parquetBytes.length,
          annotations: { 'org.opencontainers.image.title': 'orgs.parquet' },
        },
      ],
    };
    const manifestBytes = new TextEncoder().encode(JSON.stringify(manifestObj));
    manifestDigestHex = crypto.createHash('sha256').update(manifestBytes).digest('hex');

    // Put objects in R2
    await bucket.put('v2/ods-data/manifests/2026-07-31_1.0.1', manifestBytes);
    await bucket.put('v2/ods-data/manifests/2026-07-31', manifestBytes);
    await bucket.put('v2/ods-data/manifests/latest', manifestBytes);
    await bucket.put(`v2/ods-data/blobs/sha256/${manifestDigestHex}`, manifestBytes);
    await bucket.put(`v2/ods-data/blobs/sha256/${provDigestHex}`, provBytes);
    await bucket.put(`v2/ods-data/blobs/sha256/${parquetDigestHex}`, parquetBytes);
    await bucket.put('2026-07-31/1.0.1/orgs.parquet', parquetBytes);
    await bucket.put('latest/orgs.parquet', parquetBytes);
  });

  afterAll(async () => {
    await mf.dispose();
  });

  const conformanceBin = findConformanceBinary();

  it.skipIf(!conformanceBin)(
    'Acceptance 4: Official OCI distribution-spec conformance suite passes for pull workflow',
    () => {
      if (!conformanceBin) return;

      const env = {
      ...process.env,
      OCI_REGISTRY: `127.0.0.1:${port}`,
      OCI_TLS: 'disabled',
      OCI_REPO1: 'ods-data',
      OCI_API_PULL: 'true',
      OCI_API_PUSH: 'false',
      OCI_API_BLOBS_DELETE: 'false',
      OCI_API_MANIFESTS_DELETE: 'false',
      OCI_API_TAGS_DELETE: 'false',
      OCI_API_TAGS_LIST: 'false',
      OCI_API_REFERRER: 'false',
      OCI_RO_DATA_TAGS: '2026-07-31_1.0.1 2026-07-31 latest',
      OCI_RO_DATA_MANIFESTS: `sha256:${manifestDigestHex}`,
      OCI_RO_DATA_BLOBS: `sha256:${provDigestHex} sha256:${parquetDigestHex}`,
      OCI_LOG: 'warn',
    };

    let output = '';
    try {
      output = execFileSync(conformanceBin, [], { env, stdio: 'pipe' }).toString();
    } catch (err: any) {
      output = (err.stdout ? err.stdout.toString() : '') + '\n' + (err.stderr ? err.stderr.toString() : '');
    }

    expect(output).toContain('OCI Conformance Result: Pass');
    expect(output).toContain('FAIL..........................:          0');
    expect(output).toContain('Error.........................:          0');
    expect(output).toContain('Ping..........................:       Pass');
    expect(output).toContain('Manifest get by tag...........:       Pass');
    expect(output).toContain('Manifest get by digest........:       Pass');
    expect(output).toContain('Blob get......................:       Pass');
  });
});
