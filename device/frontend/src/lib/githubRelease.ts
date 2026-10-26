export const OPENSTREAMER_REPOSITORY = "felizey/Openstreamer";
export const OPENSTREAMER_LATEST_RELEASE_API = `https://api.github.com/repos/${OPENSTREAMER_REPOSITORY}/releases/latest`;

export interface ReleaseAsset {
  name: string;
  url: string;
  size: number;
}

export interface GitHubRelease {
  tag: string;
  title: string;
  url: string;
  assets: ReleaseAsset[];
}

interface Version {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[];
}

function parseVersion(value: string): Version | null {
  const match =
    /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(
      value.trim(),
    );
  if (!match) return null;
  const prerelease = match[4]?.split(".") ?? [];
  if (
    prerelease.some(
      (part) =>
        !part || (/^\d+$/.test(part) && part.length > 1 && part[0] === "0"),
    )
  ) {
    return null;
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease,
  };
}

/** Returns null when either version is not a valid SemVer value. */
export function compareVersions(
  candidate: string,
  installed: string,
): number | null {
  const left = parseVersion(candidate);
  const right = parseVersion(installed);
  if (!left || !right) return null;

  for (const key of ["major", "minor", "patch"] as const) {
    if (left[key] !== right[key]) return left[key] > right[key] ? 1 : -1;
  }

  if (!left.prerelease.length || !right.prerelease.length) {
    if (left.prerelease.length === right.prerelease.length) return 0;
    return left.prerelease.length ? -1 : 1;
  }
  const length = Math.max(left.prerelease.length, right.prerelease.length);
  for (let index = 0; index < length; index++) {
    const a = left.prerelease[index];
    const b = right.prerelease[index];
    if (a === undefined) return -1;
    if (b === undefined) return 1;
    if (a === b) continue;
    const aNumeric = /^\d+$/.test(a);
    const bNumeric = /^\d+$/.test(b);
    if (aNumeric && bNumeric) return Number(a) > Number(b) ? 1 : -1;
    if (aNumeric !== bNumeric) return aNumeric ? -1 : 1;
    return a > b ? 1 : -1;
  }
  return 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function releasePageUrl(value: unknown): string | null {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    if (
      url.protocol === "https:" &&
      url.hostname === "github.com" &&
      url.pathname.startsWith(`/${OPENSTREAMER_REPOSITORY}/releases/tag/`)
    ) {
      return url.href;
    }
  } catch {
    // A malformed URL is not a usable release link.
  }
  return null;
}

function assetUrl(value: unknown, tag: string): string | null {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    const prefix = `/${OPENSTREAMER_REPOSITORY}/releases/download/${tag}/`;
    if (
      url.protocol === "https:" &&
      url.hostname === "github.com" &&
      decodeURIComponent(url.pathname).startsWith(prefix)
    ) {
      return url.href;
    }
  } catch {
    // Ignore malformed asset URLs from the response.
  }
  return null;
}

/** Parses GitHub's latest published stable release response. */
export function parseLatestRelease(value: unknown): GitHubRelease | null {
  if (!isRecord(value) || value.draft === true || value.prerelease === true)
    return null;
  const tag = typeof value.tag_name === "string" ? value.tag_name : "";
  if (!parseVersion(tag)) return null;
  const url = releasePageUrl(value.html_url);
  if (!url) return null;

  const assets: ReleaseAsset[] = [];
  if (Array.isArray(value.assets)) {
    for (const item of value.assets) {
      if (!isRecord(item) || typeof item.name !== "string") continue;
      const assetUrlValue = assetUrl(item.browser_download_url, tag);
      if (!assetUrlValue) continue;
      const name = item.name;
      if (!/\.(?:osupd|deb|sig)$/i.test(name)) continue;
      assets.push({
        name,
        url: assetUrlValue,
        size: typeof item.size === "number" && item.size >= 0 ? item.size : 0,
      });
    }
  }

  return {
    tag,
    title:
      typeof value.name === "string" && value.name.trim() ? value.name : tag,
    url,
    assets,
  };
}
