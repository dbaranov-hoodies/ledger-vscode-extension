// Builds the release sidecar and puts it in bin/, which is where the extension looks first.
const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const executable = process.platform === "win32" ? "ledger-sidecar.exe" : "ledger-sidecar";

execFileSync(
  "cargo",
  ["build", "--release", "--manifest-path", path.join(root, "backend", "Cargo.toml"), "-p", "ledger-sidecar"],
  { stdio: "inherit" },
);
fs.mkdirSync(path.join(root, "bin"), { recursive: true });
fs.copyFileSync(
  path.join(root, "backend", "target", "release", executable),
  path.join(root, "bin", executable),
);
