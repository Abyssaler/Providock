import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
// Release tooling is executable JavaScript, shared by validation and the publishing CLI.
import { createManifest, UPDATE_ENDPOINT } from "../scripts/create-updater-manifest.mjs";

const signature = Buffer.from(
  [
    "untrusted comment: synthetic signature, not cryptographically valid",
    Buffer.alloc(74).toString("base64"),
    "trusted comment: synthetic",
    Buffer.alloc(64).toString("base64"),
  ].join("\n"),
).toString("base64");
const input = {
  version: "2.0.3",
  pubDate: "2026-10-12T00:00:00Z",
  notes: "Release notes",
  macSignature: signature,
  windowsSignature: signature,
};

describe("v2 更新清单发布契约", () => {
  it("两平台 URL 与签名来自当前版本，应用配置使用独立 v2 端点", () => {
    const manifest = createManifest(input);
    expect(Object.keys(manifest.platforms)).toEqual(["darwin-aarch64", "windows-x86_64"]);
    expect(manifest.platforms["darwin-aarch64"].url).toMatch(/v2\.0\.3\/Providock\.app\.tar\.gz$/);
    expect(manifest.platforms["windows-x86_64"].url).toMatch(
      /v2\.0\.3\/Providock_2\.0\.3_x64-setup\.exe$/,
    );
    expect(manifest.platforms["darwin-aarch64"].signature).toBe(signature);
    const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
    expect(config.plugins.updater.endpoints).toEqual([UPDATE_ENDPOINT]);
    expect(config.version).toBe(JSON.parse(readFileSync("package.json", "utf8")).version);
  });
  it("拒绝预发布、路径注入、无效时间或未提供签名", () => {
    for (const version of ["2.0.3-beta.1", "../2.0.3", "v2.0.3"])
      expect(() => createManifest({ ...input, version })).toThrow();
    expect(() => createManifest({ ...input, pubDate: "today" })).toThrow();
    expect(() => createManifest({ ...input, windowsSignature: "" })).toThrow();
    expect(() =>
      createManifest({ ...input, macSignature: Buffer.from("invalid").toString("base64") }),
    ).toThrow();
  });
});
