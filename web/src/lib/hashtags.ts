// Hashtag rules mirror the server (plan 5.1): NFKC, case-folded, letters / digits / `_`, 1-64 chars.

export const MAX_TAG_LEN = 64;
export const MAX_TAGS = 32;

/** Normalise one hashtag; returns null when nothing valid is left. Invalid characters are dropped. */
export function normalizeTag(raw: string): string | null {
  const s = raw
    .trim()
    .replace(/^#+/, '')
    .normalize('NFKC')
    .toLowerCase()
    .normalize('NFKC')
    .replace(/[^\p{L}\p{N}\p{M}_]/gu, '')
    .slice(0, MAX_TAG_LEN);
  return s.length > 0 ? s : null;
}

/** Split free text (`#a #b, c`) into normalised, de-duplicated hashtags. */
export function parseTags(text: string): string[] {
  const out: string[] = [];
  for (const part of text.split(/[\s,]+/)) {
    const t = normalizeTag(part);
    if (t && !out.includes(t)) out.push(t);
  }
  return out;
}

/** Remove characters that can never be part of a hashtag, keeping separators the input reacts to. */
export function cleanDraft(text: string): string {
  return text.replace(/^#+/, '').replace(/[^\p{L}\p{N}\p{M}_\s,#]/gu, '');
}
