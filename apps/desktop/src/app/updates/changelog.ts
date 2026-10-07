export type ChangelogEntry = {
  version: string;
  date: string;
  notes: string;
};

export type Changelog = {
  generatedAt: string;
  entries: ChangelogEntry[];
};

const CHANGELOG_URL =
  "https://github.com/ProdStalker/p2puick-updates/releases/latest/download/changelog.json";
const GITHUB_RELEASES_URL =
  "https://api.github.com/repos/ProdStalker/p2puick-updates/releases?per_page=50";

export function compareVersions(a: string, b: string): number {
  const pa = parseVersion(a);
  const pb = parseVersion(b);
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] < pb[i] ? -1 : 1;
  }
  return 0;
}

function parseVersion(raw: string): [number, number, number] {
  const clean = raw.trim().replace(/^v/i, "");
  const parts = clean.split(/[.+-]/).map((p) => Number.parseInt(p, 10));
  return [parts[0] || 0, parts[1] || 0, parts[2] || 0];
}

export async function fetchChangelog(): Promise<Changelog> {
  try {
    const res = await fetch(CHANGELOG_URL, {
      headers: { Accept: "application/json" },
      cache: "no-store",
    });
    if (res.ok) {
      const data = (await res.json()) as Changelog;
      if (Array.isArray(data.entries)) {
        return normalizeChangelog(data);
      }
    }
  } catch {
    // fall through
  }
  return fetchChangelogFromGithub();
}

async function fetchChangelogFromGithub(): Promise<Changelog> {
  const res = await fetch(GITHUB_RELEASES_URL, {
    headers: { Accept: "application/vnd.github+json" },
    cache: "no-store",
  });
  if (!res.ok) {
    throw new Error(`Changelog indisponible (${res.status})`);
  }
  const releases = (await res.json()) as Array<{
    tag_name?: string;
    published_at?: string;
    body?: string | null;
  }>;
  const entries: ChangelogEntry[] = releases
    .map((r) => ({
      version: (r.tag_name ?? "").replace(/^v/i, ""),
      date: r.published_at ?? "",
      notes: (r.body ?? "").trim(),
    }))
    .filter((e) => e.version && e.notes);
  return normalizeChangelog({ generatedAt: new Date().toISOString(), entries });
}

function normalizeChangelog(data: Changelog): Changelog {
  const entries = [...data.entries]
    .map((e) => ({
      version: String(e.version).replace(/^v/i, ""),
      date: e.date ?? "",
      notes: (e.notes ?? "").trim(),
    }))
    .filter((e) => e.version)
    .sort((a, b) => compareVersions(b.version, a.version));
  return { generatedAt: data.generatedAt || "", entries };
}

export function entriesSince(
  changelog: Changelog,
  fromVersion: string,
  toVersion?: string,
): ChangelogEntry[] {
  return changelog.entries.filter((e) => {
    if (compareVersions(e.version, fromVersion) <= 0) return false;
    if (toVersion && compareVersions(e.version, toVersion) > 0) return false;
    return true;
  });
}

export function formatEntriesPlain(entries: ChangelogEntry[]): string {
  if (!entries.length) return "";
  return entries
    .map((e) => {
      const notes = e.notes || "—";
      return `• v${e.version}\n  ${notes.replace(/\n/g, "\n  ")}`;
    })
    .join("\n\n");
}

export function formatDateShort(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso.slice(0, 10);
  return d.toLocaleDateString("fr-FR", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
