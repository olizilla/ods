import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { Miniflare } from 'miniflare';
import esbuild from 'esbuild';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
import { execFileSync, execSync } from 'node:child_process';
import crypto from 'node:crypto';

function walkDir(base: string, current: string = base): string[] {
  let results: string[] = [];
  const entries = fs.readdirSync(current, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(current, entry.name);
    if (entry.isDirectory()) {
      results = results.concat(walkDir(base, full));
    } else if (entry.isFile()) {
      results.push(path.relative(base, full).replace(/\\/g, '/'));
    }
  }
  return results;
}

describe('End-to-End Worker & OCI Registry via Miniflare', () => {
  let mf: Miniflare;
  let serverUrl: string;
  let port: string;

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
      r2Buckets: ['BUCKET'],
      port: 8789,
    });
    const url = await mf.ready;
    serverUrl = url.origin;
    port = url.port;
  });

  afterAll(async () => {
    await mf.dispose();
  });

  it('Acceptance 1: Full round-trip seam test (ods make release -> Miniflare R2 -> ods pull -> byte-identical)', async () => {
    const odsBin = path.resolve(__dirname, '../../../target/debug/ods');
    if (!fs.existsSync(odsBin)) {
      throw new Error(`ods binary not found at ${odsBin}. Run 'cargo build' first.`);
    }

    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ods-e2e-seam-'));
    try {
      const fixtureDir = path.join(tmpDir, 'fixture');
      const trudDir = path.join(fixtureDir, 'trud');
      const repoDir = path.join(tmpDir, 'repo');
      const repoDataDir = path.join(repoDir, 'data');
      const workspaceDir = path.join(tmpDir, 'workspace');

      fs.mkdirSync(trudDir, { recursive: true });
      fs.mkdirSync(repoDataDir, { recursive: true });
      fs.mkdirSync(workspaceDir, { recursive: true });

      // 1. Create source release files
      const notesContent = '# Release Notes\nEnd-to-end seam test release notes.\n';
      const datapackageContent = '{"name": "test-dataset", "version": "0.1.0"}';
      const orgsContent = Buffer.from([0x50, 0x41, 0x52, 0x31, 0x01, 0x02, 0x03, 0x04, 0x50, 0x41, 0x52, 0x31]); // valid-ish parquet magic
      const rolesContent = Buffer.from([0x50, 0x41, 0x52, 0x31, 0x05, 0x06, 0x07, 0x08, 0x50, 0x41, 0x52, 0x31]);

      fs.writeFileSync(path.join(fixtureDir, 'NOTES.md'), notesContent);
      fs.writeFileSync(path.join(fixtureDir, 'datapackage.json'), datapackageContent);
      fs.writeFileSync(path.join(fixtureDir, 'orgs.parquet'), orgsContent);
      fs.writeFileSync(path.join(fixtureDir, 'roles.parquet'), rolesContent);

      // Create synthetic outer TRUD zip
      const zipPath = path.join(trudDir, 'hscorgrefdataxml_data_7.0.0_20260731000001.zip');
      const dummyTxt = path.join(tmpDir, 'dummy.txt');
      fs.writeFileSync(dummyTxt, 'dummy source zip');
      execSync(`zip -j -q "${zipPath}" "${dummyTxt}"`);
      const zipSha256 = crypto.createHash('sha256').update(fs.readFileSync(zipPath)).digest('hex').toUpperCase();

      // 2. Initialize temporary git repo
      fs.writeFileSync(path.join(repoDir, 'Cargo.toml'), '[package]\nname = "ods"\nversion = "0.1.0"\n');
      fs.mkdirSync(path.join(repoDir, 'src'), { recursive: true });
      fs.writeFileSync(path.join(repoDir, 'src', 'main.rs'), 'fn main() {}\n');
      fs.writeFileSync(
        path.join(repoDataDir, 'releases.json'),
        JSON.stringify({
          _type: 'ods_release_index',
          index_version: 2,
          mirrors: [{ url: `${serverUrl}/v2/ods-data` }],
          releases: [],
        })
      );

      execSync(
        'git init -b main --quiet && git config user.name "Test" && git config user.email "test@example.com" && git config commit.gpgsign false && git config tag.gpgsign false && git add . && GIT_AUTHOR_DATE="2026-01-01T00:00:00Z" GIT_COMMITTER_DATE="2026-01-01T00:00:00Z" git commit --no-gpg-sign -m "initial" --quiet && git tag --no-sign "v0.1.0"',
        { cwd: repoDir }
      );
      const gitSha = execSync('git rev-parse HEAD', { cwd: repoDir, encoding: 'utf-8' }).trim();

      const provenanceObj = {
        _type: 'ods_provenance',
        dataset_version: '0.1.0',
        publication_date: '2026-07-28',
        publication_source: 'TRUD',
        publication_seq_num: '4700',
        publication_type: 'Full',
        publication_record_count: 2,
        trud_release_name: 'Release 7.0.0',
        trud_release_date: '2026-07-31',
        trud_release_file: 'hscorgrefdataxml_data_7.0.0_20260731000001.zip',
        trud_release_filesize_bytes: 37983173,
        trud_release_sha256: zipSha256,
        trud_release_sha256_verified: 'trud_api',
        tool_name: 'ods',
        tool_version: '0.1.0',
        tool_git_sha: gitSha,
        tool_git_dirty: false,
        dataset_doi: '10.5281/zenodo.12345',
      };
      fs.writeFileSync(path.join(fixtureDir, '_provenance.json'), JSON.stringify(provenanceObj, null, 2));

      // 3. Run ods make release on fixture release to produce dist/
      const distDir = path.join(repoDir, 'dist');
      execFileSync(odsBin, ['make', 'release', '--input', fixtureDir, '--tool-repo', repoDir, '--output', distDir, '--offline'], {
        stdio: 'pipe',
      });

      expect(fs.existsSync(distDir)).toBe(true);

      // 4. Load every single object from dist/ into in-process Miniflare R2
      const bucket = await mf.getR2Bucket('BUCKET');
      const distKeys = walkDir(distDir);
      expect(distKeys.length).toBe(9);

      for (const relKey of distKeys) {
        const filePath = path.join(distDir, relKey);
        const fileContent = new Uint8Array(fs.readFileSync(filePath));
        await bucket.put(relKey, fileContent);
      }

      // Load updated data/releases.json with mirror pointing to Miniflare URL
      const updatedReleasesJson = fs.readFileSync(path.join(repoDataDir, 'releases.json'), 'utf-8');
      const parsedReleases = JSON.parse(updatedReleasesJson);
      parsedReleases.mirrors = [{ url: `${serverUrl}/v2/ods-data` }];
      await bucket.put('releases.json', new TextEncoder().encode(JSON.stringify(parsedReleases, null, 2)));

      // 5. Run ods pull from the Miniflare server
      execFileSync(odsBin, ['pull', '2026-07-31', '--verbose', '--no-progress'], {
        cwd: workspaceDir,
        env: {
          ...process.env,
          ODS_RELEASE_INDEX_URL: `${serverUrl}/releases.json`,
        },
        stdio: 'pipe',
      });

      // 6. Assert pulled release directory is byte-identical to source release directory
      const pulledDir = path.join(workspaceDir, 'ods_data', 'releases', '2026-07-31');
      expect(fs.existsSync(pulledDir)).toBe(true);

      const filesToVerify = ['orgs.parquet', 'roles.parquet', 'datapackage.json', 'NOTES.md', '_provenance.json'];
      for (const file of filesToVerify) {
        const sourceBytes = fs.readFileSync(path.join(fixtureDir, file));
        const pulledBytes = fs.readFileSync(path.join(pulledDir, file));
        expect(pulledBytes).toEqual(sourceBytes);
      }

      // Verify that pull never writes OCI metadata to client workspace
      expect(fs.existsSync(path.join(pulledDir, 'oci'))).toBe(false);
      expect(fs.existsSync(path.join(pulledDir, '_release.json'))).toBe(false);
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it('Acceptance 10: /latest/ actually moves when a new release is published', async () => {
    const bucket = await mf.getR2Bucket('BUCKET');

    const publishRelease = async (orgsBytes: Uint8Array) => {
      const orgsDigest = crypto.createHash('sha256').update(orgsBytes).digest('hex');
      await bucket.put(`v2/ods-data/blobs/sha256/${orgsDigest}`, orgsBytes);

      const manifestObj = {
        schemaVersion: 2,
        mediaType: 'application/vnd.oci.image.manifest.v1+json',
        config: { digest: 'sha256:dummyconfig', size: 10 },
        layers: [
          {
            mediaType: 'application/vnd.apache.parquet',
            digest: `sha256:${orgsDigest}`,
            size: orgsBytes.length,
            annotations: {
              'org.opencontainers.image.title': 'orgs.parquet',
            },
          },
        ],
      };
      const manifestBytes = new TextEncoder().encode(JSON.stringify(manifestObj));
      await bucket.put('v2/ods-data/manifests/latest', manifestBytes);
    };

    // Initial release
    const v1Bytes = new Uint8Array([10, 20, 30]);
    await publishRelease(v1Bytes);

    const res1 = await mf.dispatchFetch(`${serverUrl}/latest/orgs.parquet`);
    expect(res1.status).toBe(200);
    const body1 = new Uint8Array(await res1.arrayBuffer());
    expect(body1).toEqual(v1Bytes);

    // Wait for in-worker manifest cache TTL to expire
    await new Promise((resolve) => setTimeout(resolve, 1100));

    // Newer release overwrites /latest/
    const v2Bytes = new Uint8Array([40, 50, 60, 70]);
    await publishRelease(v2Bytes);

    const res2 = await mf.dispatchFetch(`${serverUrl}/latest/orgs.parquet`);
    expect(res2.status).toBe(200);
    const body2 = new Uint8Array(await res2.arrayBuffer());
    expect(body2).toEqual(v2Bytes);
  });

  it('Acceptance 11: Frontier release resolution through /releases.json', async () => {
    const bucket = await mf.getR2Bucket('BUCKET');

    // Frontier release newer than baked index
    const frontierIndex = {
      _type: 'ods_release_index',
      index_version: 2,
      mirrors: [{ url: `${serverUrl}/v2/ods-data` }],
      releases: [
        {
          trud_release_date: '2027-01-31',
          dataset_version: '2.0.0',
          tag: '2027-01-31_2.0.0',
          manifest_digest: 'sha256:frontier123',
          trud_release_sha256: 'FRONTIERSHA',
          tool_version: '0.2.0',
        },
      ],
    };
    await bucket.put('releases.json', new TextEncoder().encode(JSON.stringify(frontierIndex)));

    const res = await mf.dispatchFetch(`${serverUrl}/releases.json`);
    expect(res.status).toBe(200);
    expect(res.headers.get('cache-control')).toBe('no-cache');
    const indexData = (await res.json()) as typeof frontierIndex;
    expect(indexData.releases[0].trud_release_date).toBe('2027-01-31');
  });

  it('Acceptance 21: <name> stays opaque for multi-segment names', async () => {
    const bucket = await mf.getR2Bucket('BUCKET');
    const blobData = new Uint8Array([99, 88, 77]);
    const hex = '3333333333333333333333333333333333333333333333333333333333333333';

    await bucket.put(`v2/some/nested/pkg/blobs/sha256/${hex}`, blobData);

    const res = await mf.dispatchFetch(`${serverUrl}/v2/some/nested/pkg/blobs/sha256:${hex}`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('application/octet-stream');
    expect(res.headers.get('docker-content-digest')).toBe(`sha256:${hex}`);
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes).toEqual(blobData);
  });
});
