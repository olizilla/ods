export type Kind = 'ink' | 'dim' | 'accent' | 'frame' | 'header' | 'role';

export interface Segment {
  text: string;
  kind: Kind;
}

const KEYWORDS = /\b(SELECT|FROM|WHERE|AND|OR|ORDER BY|LIKE|LIMIT)\b/g;
const QUOTED = /('[^']*'|"[^"]*")/g;
const ROLE_CODE = /\bRO\d+\b/g;
const DUCKDB_OPEN = 'duckdb -c "';

// Keywords in accent, quoted text in `literal`, everything else in `plain`.
export function colourQuoted(line: string, plain: Kind, literal: Kind): Segment[] {
  const segments: Segment[] = [];
  let cursor = 0;
  const combined = new RegExp(`${KEYWORDS.source}|${QUOTED.source}`, 'g');
  let match: RegExpExecArray | null;
  while ((match = combined.exec(line)) !== null) {
    if (match.index > cursor) {
      segments.push({ text: line.slice(cursor, match.index), kind: plain });
    }
    const isQuoted = match[0].startsWith("'") || match[0].startsWith('"');
    segments.push({ text: match[0], kind: isQuoted ? literal : 'accent' });
    cursor = match.index + match[0].length;
  }
  if (cursor < line.length) {
    segments.push({ text: line.slice(cursor), kind: plain });
  }
  return segments.length > 0 ? segments : [{ text: line, kind: plain }];
}

// SQL: syntax recedes, literals stand out.
export function colourSql(line: string): Segment[] {
  return colourQuoted(line, 'dim', 'ink');
}

// A `duckdb -c "…"` command: the shell wrapper stays ink, the SQL inside is coloured as SQL.
// The command may span lines; the opening sits on the first and the closing quote on the last.
export function colourDuckdbCommand(lines: string[]): Segment[][] {
  return lines.map((line, i) => {
    let head = '';
    let tail = '';
    let body = line;
    if (i === 0 && body.startsWith(DUCKDB_OPEN)) {
      head = DUCKDB_OPEN;
      body = body.slice(DUCKDB_OPEN.length);
    }
    if (i === lines.length - 1 && body.endsWith('"')) {
      tail = '"';
      body = body.slice(0, -1);
    }
    return [
      ...(head ? [{ text: head, kind: 'ink' as Kind }] : []),
      ...colourSql(body),
      ...(tail ? [{ text: tail, kind: 'ink' as Kind }] : []),
    ];
  });
}

// Role codes (RO76, RO198…) get their own colour wherever they sit in ink or dim text.
export function markRoleCodes(segments: Segment[]): Segment[] {
  return segments.flatMap((seg) => {
    if (seg.kind !== 'ink' && seg.kind !== 'dim') return [seg];
    const out: Segment[] = [];
    let cursor = 0;
    for (const m of seg.text.matchAll(ROLE_CODE)) {
      const at = m.index ?? 0;
      if (at > cursor) out.push({ text: seg.text.slice(cursor, at), kind: seg.kind });
      out.push({ text: m[0], kind: 'role' });
      cursor = at + m[0].length;
    }
    if (cursor === 0) return [seg];
    if (cursor < seg.text.length) out.push({ text: seg.text.slice(cursor), kind: seg.kind });
    return out;
  });
}
