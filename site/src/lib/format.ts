// Numbers are stored as numbers and formatted here with thousands separators; sizes are
// stored in bytes and formatted to one decimal place of MB (base 1024, matching `ods`).

export function formatCount(n: number): string {
  return n.toLocaleString('en-GB');
}

export function formatMB(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
}
