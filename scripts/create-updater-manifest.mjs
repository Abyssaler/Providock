import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const MANIFEST_NAME = "providock-v2-latest.json";
export const UPDATE_ENDPOINT =
  "https://github.com/Abyssaler/Providock/releases/latest/download/" + MANIFEST_NAME;

function validateSignature(signature) {
  if (typeof signature !== "string" || !/^[A-Za-z0-9+/=\s]+$/.test(signature))
    throw new Error("Invalid minisign signature encoding");
  const lines = Buffer.from(signature.trim(), "base64").toString("utf8").trim().split(/\r?\n/);
  if (
    lines.length !== 4 ||
    !lines[0].startsWith("untrusted comment:") ||
    !lines[2].startsWith("trusted comment:") ||
    Buffer.from(lines[1], "base64").length !== 74 ||
    Buffer.from(lines[3], "base64").length !== 64
  )
    throw new Error("Invalid minisign signature structure");
  return signature.trim();
}

/** Structural validation only. Independently verify signatures against artifacts before publishing. */
export function createManifest({ version, pubDate, notes, macSignature, windowsSignature }) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error("A stable release version is required");
  if (
    !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z$/.test(pubDate) ||
    !Number.isFinite(Date.parse(pubDate))
  )
    throw new Error("A UTC RFC 3339 publish date is required");
  if (typeof notes !== "string" || !notes.trim()) throw new Error("Release notes are required");
  const base = `https://github.com/Abyssaler/Providock/releases/download/v${version}/`;
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: {
      "darwin-aarch64": {
        url: base + "Providock.app.tar.gz",
        signature: validateSignature(macSignature),
      },
      "windows-x86_64": {
        url: base + `Providock_${version}_x64-setup.exe`,
        signature: validateSignature(windowsSignature),
      },
    },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [assetsDirectory, notesFile, outputFile] = process.argv.slice(2);
  if (!assetsDirectory || !notesFile || !outputFile)
    throw new Error("Usage: node scripts/create-updater-manifest.mjs ASSETS NOTES OUTPUT");
  const { version } = JSON.parse(readFileSync("package.json", "utf8"));
  const manifest = createManifest({
    version,
    pubDate: new Date().toISOString(),
    notes: readFileSync(notesFile, "utf8"),
    macSignature: readFileSync(resolve(assetsDirectory, "Providock.app.tar.gz.sig"), "utf8"),
    windowsSignature: readFileSync(
      resolve(assetsDirectory, `Providock_${version}_x64-setup.exe.sig`),
      "utf8",
    ),
  });
  writeFileSync(outputFile, JSON.stringify(manifest, null, 2) + "\n");
  console.log(`Created ${MANIFEST_NAME} for v${version}`);
}
