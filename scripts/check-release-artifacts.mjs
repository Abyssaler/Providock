import { readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const { version } = JSON.parse(readFileSync("package.json", "utf8"));
const base = "src-tauri/target/release/bundle";
const files =
  process.platform === "darwin"
    ? [
        "dmg/Providock_" + version + "_aarch64.dmg",
        "dmg/Providock_" + version + "_aarch64.dmg.sig",
        "macos/Providock.app.tar.gz",
        "macos/Providock.app.tar.gz.sig",
      ]
    : process.platform === "win32"
      ? [
          "nsis/Providock_" + version + "_x64-setup.exe",
          "nsis/Providock_" + version + "_x64-setup.exe.sig",
        ]
      : null;

if (!files) throw new Error("Unsupported release platform");
for (const file of files) {
  const path = join(base, file);
  const stat = statSync(path);
  if (!stat.isFile() || !stat.size) throw new Error("Empty artifact: " + file);
  console.log("Verified release artifact: " + file);
}
