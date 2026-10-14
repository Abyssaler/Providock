import { readFileSync } from "node:fs";
import { createHash, createPublicKey, verify } from "node:crypto";
import { basename, resolve } from "node:path";

// Independent Ed25519/minisign verification, including authenticated trusted comments.
const [directory, version] = process.argv.slice(2);
if (!directory || !/^\d+\.\d+\.\d+$/.test(version ?? ""))
  throw new Error("Usage: node scripts/verify-release-signatures.mjs ASSETS VERSION");
const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const decodeKey = (text) => {
  for (let i = 0; i < 3; i++) {
    if (text.startsWith("untrusted comment:"))
      return Buffer.from(text.trim().split(/\r?\n/)[1], "base64");
    const bytes = Buffer.from(text.trim(), "base64");
    if (bytes.length === 42) return bytes;
    text = bytes.toString();
  }
  throw new Error("Invalid minisign public key encoding");
};
const configured = decodeKey(Buffer.from(config.plugins.updater.pubkey, "base64").toString());
const published = decodeKey(readFileSync(resolve(directory, "providock.key.pub"), "utf8"));
if (configured.length !== 42 || !configured.equals(published))
  throw new Error("Release public key differs from the application's trusted key");
const key = createPublicKey({
  key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), configured.subarray(10)]),
  format: "der",
  type: "spki",
});
const names = [
  `Providock_${version}_aarch64.dmg`,
  "Providock.app.tar.gz",
  `Providock_${version}_x64-setup.exe`,
];
if (process.argv[4] && !names.includes(process.argv[4]))
  throw new Error("Unknown release artifact");
for (const name of names.filter((name) => !process.argv[4] || name === process.argv[4])) {
  const data = readFileSync(resolve(directory, name));
  const lines = Buffer.from(
    readFileSync(resolve(directory, name + ".sig"), "utf8").trim(),
    "base64",
  )
    .toString()
    .trim()
    .split(/\r?\n/);
  const sig = Buffer.from(lines[1] ?? "", "base64");
  if (
    sig.length !== 74 ||
    sig.subarray(0, 2).toString() !== "ED" ||
    !sig.subarray(2, 10).equals(configured.subarray(2, 10)) ||
    !lines[2]?.startsWith("trusted comment: ")
  )
    throw new Error(`Invalid signature structure/key/algorithm: ${name}`);
  const comment = lines[2].slice(17);
  if (!comment.includes(basename(name)) || !comment.includes(`:${version}`))
    throw new Error(`Signature does not authenticate expected filename/version: ${name}`);
  if (
    !verify(null, createHash("blake2b512").update(data).digest(), key, sig.subarray(10)) ||
    !verify(
      null,
      Buffer.concat([sig.subarray(10), Buffer.from(comment)]),
      key,
      Buffer.from(lines[3], "base64"),
    )
  )
    throw new Error(`Signature verification failed: ${name}`);
  console.log(`Verified ${name}`);
}
