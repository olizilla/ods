export interface Env {
  BUCKET: R2Bucket;
}

const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS',
  'Access-Control-Allow-Headers': 'Range, If-Match, If-None-Match',
  'Access-Control-Expose-Headers': 'Content-Range, Content-Length, Accept-Ranges, ETag, Docker-Content-Digest',
  'Access-Control-Max-Age': '86400',
};

import releasesV1Schema from '../schema/releases.v1.json';

const SCHEMAS: Record<string, unknown> = {
  'releases.v1.json': releasesV1Schema,
};

const ROOT_TEXT = `ods.fyi — All the organisations and sites in the NHS Organisation Data Service,
as queryable & verifiable Parquet files. An independent project.

  duckdb -c "SELECT * FROM 'https://ods.fyi/orgs.parquet' LIMIT 5"

Running more than a few queries? Fetch it once and query locally:

  cargo install --git https://github.com/olizilla/ods
  ods pull

Releases:  https://ods.fyi/releases.json
Code:      https://github.com/olizilla/ods
Data from: NHS England, via NHS TRUD, under the Open Government Licence
`;

/**
 * Pure function mapping request path to R2 object key.
 */
export function pathToKey(pathname: string): string {
  const clean = pathname.replace(/^\/+/, '');

  // Rule: /manifests/sha256:<hex> -> /blobs/sha256/<hex>
  const manifestDigestMatch = clean.match(/^(.*\/)?manifests\/sha256:([a-fA-F0-9]{64})$/);
  if (manifestDigestMatch) {
    const prefix = manifestDigestMatch[1] || '';
    const hex = manifestDigestMatch[2].toLowerCase();
    return `${prefix}blobs/sha256/${hex}`;
  }

  // Rule: replace sha256:<hex> with sha256/<hex> (lowercased)
  return clean.replace(/sha256:([a-fA-F0-9]{64})/gi, (_, hex) => `sha256/${hex.toLowerCase()}`);
}

/**
 * Calculates SHA-256 hex digest of a Uint8Array or ArrayBuffer.
 */
export async function sha256Hex(data: ArrayBuffer | Uint8Array): Promise<string> {
  const hashBuffer = await crypto.subtle.digest('SHA-256', data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map((b) => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Pure function deriving headers from the *request path*.
 */
export function deriveHeaders(pathname: string, etag?: string, computedDigest?: string): Headers {
  const headers = new Headers();

  // Apply standard CORS headers
  for (const [k, v] of Object.entries(CORS_HEADERS)) {
    headers.set(k, v);
  }

  headers.set('Accept-Ranges', 'bytes');

  if (etag) {
    headers.set('ETag', etag.startsWith('"') ? etag : `"${etag}"`);
  }

  const clean = pathname.replace(/^\/+/, '');

  // 1. Content-Type
  if (clean.startsWith('schema/')) {
    headers.set('Content-Type', 'application/schema+json');
  } else if (clean.includes('/manifests/')) {
    headers.set('Content-Type', 'application/vnd.oci.image.manifest.v1+json');
  } else if (clean.includes('/blobs/')) {
    headers.set('Content-Type', 'application/octet-stream');
  } else if (clean.endsWith('.parquet')) {
    headers.set('Content-Type', 'application/vnd.apache.parquet');
  } else if (clean.endsWith('.json')) {
    headers.set('Content-Type', 'application/json');
  } else if (clean.endsWith('.md')) {
    headers.set('Content-Type', 'text/markdown; charset=utf-8');
  } else if (clean.endsWith('.zip')) {
    headers.set('Content-Type', 'application/zip');
  } else {
    headers.set('Content-Type', 'application/octet-stream');
  }

  // 2. Docker-Content-Digest
  const digestMatch = clean.match(/sha256:([a-fA-F0-9]{64})/i);
  if (digestMatch) {
    headers.set('Docker-Content-Digest', `sha256:${digestMatch[1].toLowerCase()}`);
  } else if (computedDigest) {
    headers.set('Docker-Content-Digest', computedDigest.startsWith('sha256:') ? computedDigest.toLowerCase() : `sha256:${computedDigest.toLowerCase()}`);
  }

  // 3. Cache-Control
  const isImmutable =
    clean.startsWith('schema/') ||
    clean.includes('/blobs/') ||
    /^\d{4}-\d{2}-\d{2}\/\d+\.\d+\.\d+\//.test(clean) ||
    /\/manifests\/\d{4}-\d{2}-\d{2}_\d+\.\d+\.\d+$/.test(clean);

  if (isImmutable) {
    headers.set('Cache-Control', 'public, max-age=31536000, immutable');
  } else if (clean.startsWith('latest/') || (!clean.includes('/') && clean !== 'releases.json' && clean !== 'v2')) {
    headers.set('Cache-Control', 'public, max-age=300');
  } else {
    headers.set('Cache-Control', 'no-cache');
  }

  return headers;
}

function errorResponse(pathname: string): Response {
  if (pathname.startsWith('/v2/')) {
    const headers = new Headers(CORS_HEADERS);
    headers.set('Content-Type', 'application/json');
    const body = JSON.stringify({
      errors: [
        {
          code: 'NAME_UNKNOWN',
          message: 'manifest unknown',
          detail: null,
        },
      ],
    });
    return new Response(body, { status: 404, headers });
  }

  const headers = new Headers(CORS_HEADERS);
  headers.set('Content-Type', 'text/plain; charset=utf-8');
  return new Response('Not found. See https://ods.fyi/ for what lives here.\n', {
    status: 404,
    headers,
  });
}

export interface NamedPathMatch {
  manifestTag: string;
  filename: string;
  isLatest: boolean;
}

export function matchNamedPath(pathname: string): NamedPathMatch | null {
  const clean = pathname.replace(/^\/+/, '');
  if (!clean || clean === 'v2' || clean.startsWith('v2/') || clean === 'releases.json' || clean.startsWith('schema/')) {
    return null;
  }

  // 1. Versioned path: <date>/<version>/<file>
  // e.g. 2026-08-28/0.1.0/orgs.parquet
  const versionedMatch = clean.match(/^(\d{4}-\d{2}-\d{2})\/(\d+\.\d+\.\d+)\/([^/]+)$/);
  if (versionedMatch) {
    return {
      manifestTag: `v2/ods-data/manifests/${versionedMatch[1]}_${versionedMatch[2]}`,
      filename: versionedMatch[3],
      isLatest: false,
    };
  }

  // 2. Latest path: latest/<file>
  // e.g. latest/orgs.parquet
  const latestMatch = clean.match(/^latest\/([^/]+)$/);
  if (latestMatch) {
    return {
      manifestTag: 'v2/ods-data/manifests/latest',
      filename: latestMatch[1],
      isLatest: true,
    };
  }

  // 3. Root convenience path: <file>
  // e.g. orgs.parquet
  if (!clean.includes('/')) {
    return {
      manifestTag: 'v2/ods-data/manifests/latest',
      filename: clean,
      isLatest: true,
    };
  }

  return null;
}

export interface ManifestLayer {
  mediaType: string;
  digest: string;
  size: number;
  annotations?: Record<string, string>;
}

export interface ManifestInfo {
  layersByTitle: Map<string, ManifestLayer>;
  fetchedAt: number;
}

const manifestCache = new Map<string, ManifestInfo>();

export function clearManifestCache(): void {
  manifestCache.clear();
}

export async function getManifest(tag: string, bucket: R2Bucket): Promise<ManifestInfo | null> {
  const cached = manifestCache.get(tag);
  const now = Date.now();
  const isLatest = tag.endsWith('/latest');
  const ttl = isLatest ? 1000 : Infinity;

  if (cached && now - cached.fetchedAt < ttl) {
    return cached;
  }

  const obj = await bucket.get(tag);
  if (!obj) {
    return null;
  }

  try {
    const text = await obj.text();
    const parsed = JSON.parse(text);
    const layersByTitle = new Map<string, ManifestLayer>();
    if (Array.isArray(parsed.layers)) {
      for (const layer of parsed.layers) {
        const title = layer.annotations?.['org.opencontainers.image.title'];
        if (title) {
          layersByTitle.set(title, layer);
        }
      }
    }
    const info: ManifestInfo = {
      layersByTitle,
      fetchedAt: now,
    };
    manifestCache.set(tag, info);
    return info;
  } catch {
    return null;
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const pathname = url.pathname;

    // Handle OPTIONS (CORS preflight)
    if (request.method === 'OPTIONS') {
      return new Response(null, {
        status: 204,
        headers: CORS_HEADERS,
      });
    }

    // Only GET and HEAD are supported
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      return new Response('Method Not Allowed', {
        status: 405,
        headers: CORS_HEADERS,
      });
    }

    // Root endpoint
    if (pathname === '/' || pathname === '') {
      if (request.method === 'HEAD') {
        const headers = new Headers(CORS_HEADERS);
        headers.set('Content-Type', 'text/plain; charset=utf-8');
        headers.set('Content-Length', new TextEncoder().encode(ROOT_TEXT).length.toString());
        return new Response(null, { status: 200, headers });
      }
      const headers = new Headers(CORS_HEADERS);
      headers.set('Content-Type', 'text/plain; charset=utf-8');
      return new Response(ROOT_TEXT, { status: 200, headers });
    }

    // OCI V2 Ping endpoint
    if (pathname === '/v2' || pathname === '/v2/') {
      const headers = new Headers(CORS_HEADERS);
      headers.set('Content-Type', 'application/json');
      return new Response('{}', { status: 200, headers });
    }

    // Schema endpoint: /schema/<file>
    const cleanPath = pathname.replace(/^\/+/, '');
    if (cleanPath.startsWith('schema/')) {
      const schemaFile = cleanPath.slice('schema/'.length);
      if (schemaFile in SCHEMAS) {
        const body = JSON.stringify(SCHEMAS[schemaFile], null, 2) + '\n';
        const headers = deriveHeaders(pathname);
        headers.set('Content-Length', new TextEncoder().encode(body).length.toString());
        if (request.method === 'HEAD') {
          return new Response(null, { status: 200, headers });
        }
        return new Response(body, { status: 200, headers });
      }
      return errorResponse(pathname);
    }

    const namedMatch = matchNamedPath(pathname);
    let key = pathToKey(pathname);
    let resolvedLayer: ManifestLayer | null = null;

    if (namedMatch) {
      const manifest = await getManifest(namedMatch.manifestTag, env.BUCKET);
      if (!manifest) {
        return errorResponse(pathname);
      }
      const layer = manifest.layersByTitle.get(namedMatch.filename);
      if (!layer) {
        return errorResponse(pathname);
      }
      resolvedLayer = layer;
      const hex = layer.digest.replace(/^sha256:/i, '').toLowerCase();
      key = `v2/ods-data/blobs/sha256/${hex}`;
    }

    // Range parsing: ignore multi-range requests and treat as unranged (full body 200)
    const rawRange = request.headers.get('range');
    const isMultiRange = rawRange ? rawRange.includes(',') : false;

    // Check if HEAD on manifest tag
    const isManifestTag = !resolvedLayer && pathname.includes('/manifests/') && !pathname.includes('sha256:');

    if (request.method === 'HEAD') {
      if (isManifestTag) {
        // Must hash body to set Docker-Content-Digest on HEAD
        const object = await env.BUCKET.get(key);
        if (!object) {
          return errorResponse(pathname);
        }
        const bodyBytes = await object.arrayBuffer();
        const digest = await sha256Hex(bodyBytes);
        const headers = deriveHeaders(pathname, object.httpEtag, digest);
        headers.set('Content-Length', object.size.toString());
        return new Response(null, { status: 200, headers });
      }

      const headObj = await env.BUCKET.head(key);
      if (!headObj) {
        return errorResponse(pathname);
      }
      const effectiveEtag = resolvedLayer ? resolvedLayer.digest : headObj.httpEtag;
      const effectiveDigest = resolvedLayer ? resolvedLayer.digest : undefined;
      const headers = deriveHeaders(pathname, effectiveEtag, effectiveDigest);
      headers.set('Content-Length', headObj.size.toString());
      return new Response(null, { status: 200, headers });
    }

    // Check If-None-Match helper
    const ifNoneMatch = request.headers.get('if-none-match');
    const checkIfNoneMatch = (etagToCheck?: string): boolean => {
      if (!ifNoneMatch || !etagToCheck) return false;
      const cleanEtag = etagToCheck.replace(/^"|"$/g, '');
      const clientEtag = ifNoneMatch.replace(/^"|"$/g, '');
      return cleanEtag === clientEtag || ifNoneMatch === '*';
    };

    if (!rawRange || isMultiRange) {
      const object = await env.BUCKET.get(key);
      if (!object) {
        return errorResponse(pathname);
      }

      const effectiveEtag = resolvedLayer ? resolvedLayer.digest : object.httpEtag;
      const effectiveDigest = resolvedLayer ? resolvedLayer.digest : undefined;

      // Check If-None-Match
      if (checkIfNoneMatch(effectiveEtag)) {
        const headers = deriveHeaders(pathname, effectiveEtag, effectiveDigest);
        return new Response(null, { status: 304, headers });
      }

      let computedDigest: string | undefined = effectiveDigest;
      let body: ReadableStream | ArrayBuffer = object.body;

      if (isManifestTag) {
        const bodyBytes = await object.arrayBuffer();
        computedDigest = await sha256Hex(bodyBytes);
        body = bodyBytes;
      }

      const headers = deriveHeaders(pathname, effectiveEtag, computedDigest);
      headers.set('Content-Length', object.size.toString());

      return new Response(body, {
        status: 200,
        headers,
      });
    }

    // Single range request: check head for 404 / 416 bounds first
    const headObj = await env.BUCKET.head(key);
    if (!headObj) {
      return errorResponse(pathname);
    }
    const headEtag = resolvedLayer ? resolvedLayer.digest : headObj.httpEtag;
    const headDigest = resolvedLayer ? resolvedLayer.digest : undefined;

    const rangeMatch = rawRange.match(/^bytes=(\d*)-(\d*)$/);
    if (rangeMatch) {
      const startStr = rangeMatch[1];
      const endStr = rangeMatch[2];
      let isUnsatisfiable = false;

      if (startStr !== '' && endStr !== '') {
        const start = parseInt(startStr, 10);
        const end = parseInt(endStr, 10);
        if (start >= headObj.size || start > end) {
          isUnsatisfiable = true;
        }
      } else if (startStr !== '') {
        const start = parseInt(startStr, 10);
        if (start >= headObj.size) {
          isUnsatisfiable = true;
        }
      } else if (endStr !== '') {
        const suffix = parseInt(endStr, 10);
        if (suffix === 0) {
          isUnsatisfiable = true;
        }
      }

      if (isUnsatisfiable) {
        const headers = deriveHeaders(pathname, headEtag, headDigest);
        headers.set('Content-Range', `bytes */${headObj.size}`);
        return new Response(null, {
          status: 416,
          headers,
        });
      }
    }

    let object: R2ObjectBody | null;
    try {
      object = await env.BUCKET.get(key, {
        range: request.headers,
      });
    } catch {
      object = null;
    }

    if (!object) {
      const headers = deriveHeaders(pathname, headEtag, headDigest);
      headers.set('Content-Range', `bytes */${headObj.size}`);
      return new Response(null, {
        status: 416,
        headers,
      });
    }

    const effectiveEtag = resolvedLayer ? resolvedLayer.digest : object.httpEtag;
    const effectiveDigest = resolvedLayer ? resolvedLayer.digest : undefined;

    // Check If-None-Match
    if (checkIfNoneMatch(effectiveEtag)) {
      const headers = deriveHeaders(pathname, effectiveEtag, effectiveDigest);
      return new Response(null, { status: 304, headers });
    }

    let computedDigest: string | undefined = effectiveDigest;
    if (isManifestTag) {
      const fullObj = await env.BUCKET.get(key);
      if (fullObj) {
        const bytes = await fullObj.arrayBuffer();
        computedDigest = await sha256Hex(bytes);
      }
    }

    const headers = deriveHeaders(pathname, effectiveEtag, computedDigest);

    if ('range' in object && object.range) {
      const offset = (object.range as { offset?: number }).offset ?? 0;
      const length = (object.range as { length?: number }).length ?? object.size;
      headers.set('Content-Range', `bytes ${offset}-${offset + length - 1}/${object.size}`);
      headers.set('Content-Length', length.toString());

      return new Response(object.body, {
        status: 206,
        headers,
      });
    }

    headers.set('Content-Length', object.size.toString());
    return new Response(object.body, {
      status: 200,
      headers,
    });
  },
};
