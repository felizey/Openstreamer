import assert from "node:assert/strict";
import test from "node:test";
import {
  compareVersions,
  parseLatestRelease,
} from "../src/lib/githubRelease.ts";

const baseRelease = {
  draft: false,
  prerelease: false,
  tag_name: "v0.1.10",
  name: "Openstreamer 0.1.10",
  html_url: "https://github.com/felizey/Openstreamer/releases/tag/v0.1.10",
  body: "Bug fixes",
  published_at: "2026-10-25T12:00:00Z",
  assets: [
    {
      name: "openstreamer.osupd",
      browser_download_url:
        "https://github.com/felizey/Openstreamer/releases/download/v0.1.10/openstreamer.osupd",
      size: 1200,
    },
    {
      name: "untrusted.osupd",
      browser_download_url: "https://example.com/untrusted.osupd",
      size: 10,
    },
    {
      name: "notes.txt",
      browser_download_url:
        "https://github.com/felizey/Openstreamer/releases/download/v0.1.10/notes.txt",
      size: 20,
    },
  ],
};

test("parses the stable release and keeps only update assets from Openstreamer", () => {
  const release = parseLatestRelease(baseRelease);
  assert.ok(release);
  assert.equal(release.tag, "v0.1.10");
  assert.equal(release.title, "Openstreamer 0.1.10");
  assert.deepEqual(release.assets, [
    {
      name: "openstreamer.osupd",
      url: "https://github.com/felizey/Openstreamer/releases/download/v0.1.10/openstreamer.osupd",
      size: 1200,
    },
  ]);
});

test("rejects malformed and prerelease records", () => {
  assert.equal(parseLatestRelease({ ...baseRelease, prerelease: true }), null);
  assert.equal(
    parseLatestRelease({ ...baseRelease, tag_name: "nightly" }),
    null,
  );
  assert.equal(
    parseLatestRelease({
      ...baseRelease,
      html_url: "https://example.com/felizey/Openstreamer/releases/tag/v0.1.10",
    }),
    null,
  );
});

test("compares release tags and ignores build metadata", () => {
  assert.equal(compareVersions("v0.1.10", "0.1.9+build.1"), 1);
  assert.equal(compareVersions("0.1.9", "v0.1.9+build.2"), 0);
  assert.equal(compareVersions("0.1.9-rc.2", "0.1.9-rc.10"), -1);
  assert.equal(compareVersions("0.1.9", "0.1.9-rc.1"), 1);
  assert.equal(compareVersions("latest", "0.1.9"), null);
});
