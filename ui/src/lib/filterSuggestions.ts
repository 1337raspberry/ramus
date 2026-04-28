// Shared client-side suggestion filter for chip autocompletes that match
// against an in-memory list (e.g. countries). Case-insensitive substring
// match; ranking is exact > prefix > substring with an alphabetical
// tiebreaker. Used by both the desktop FilterDropdown and mobile
// MobileFilterPanel — keep them in sync by editing this single source.
export function filterCountrySuggestions(all: string[], query: string, limit: number): string[] {
  const q = query.trim().toLowerCase();
  if (!q) return all.slice(0, limit);
  const scored: Array<{ score: number; name: string }> = [];
  for (const name of all) {
    const lower = name.toLowerCase();
    if (lower === q) scored.push({ score: 0, name });
    else if (lower.startsWith(q)) scored.push({ score: 1, name });
    else if (lower.includes(q)) scored.push({ score: 2, name });
  }
  scored.sort((a, b) => a.score - b.score || a.name.localeCompare(b.name));
  return scored.slice(0, limit).map((s) => s.name);
}
