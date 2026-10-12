import * as crypto from "crypto";
import * as fg from "fast-glob";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import type { FolderInput } from "../generated/FolderInput";

/** The files the sidecar looks for in an app folder, up to two levels deep. */
export const detectionFileNames = [
  "ledger_app.toml",
  "Makefile",
  "Cargo.toml",
  "conftest.py",
  "docker-compose.yml",
];

/**
 * Of these, only their presence matters. The content of `conftest.py` is read: it says which apps the tests of
 * a suite load, which the extension can then build.
 */
const contentNotNeeded = new Set(["docker-compose.yml"]);
const maxDepth = 2;

/** The tests of an app are often deeper than its Makefile: `test/python/conftest.py`, `tests/standalone/conftest.py`. */
const conftestName = "conftest.py";
const conftestMaxDepth = 4;
/** Where a `conftest.py` is not the one of a suite: environments, dependencies and what a build leaves. */
const notSuiteDirectories = ["node_modules", ".git", ".venv", "venv", "site-packages", "build", ".test_dependencies"];

function maxDepthOf(file: string): number {
  return path.basename(file) === conftestName ? conftestMaxDepth : maxDepth;
}

function readText(file: string): string {
  try {
    return fs.readFileSync(file, "utf-8");
  }
  catch {
    return "";
  }
}

/** One Python environment per project, kept in the storage of the extension. */
function hostVenv(context: vscode.ExtensionContext, folder: vscode.WorkspaceFolder): string {
  const key = crypto.createHash("sha1").update(folder.uri.fsPath).digest("hex").slice(0, 8);
  const name = `${path.basename(folder.uri.fsPath)}-${key}`;
  return path.join(context.globalStorageUri.fsPath, "venvs", name);
}

function collectFolder(context: vscode.ExtensionContext, folder: vscode.WorkspaceFolder): FolderInput {
  const cwd = folder.uri.fsPath;
  const near = fg.sync(detectionFileNames.map(name => `**/${name}`), {
    cwd,
    deep: maxDepth,
    onlyFiles: true,
    ignore: ["**/node_modules/**", "**/.git/**"],
  });
  const conftests = fg.sync(`**/${conftestName}`, {
    cwd,
    deep: conftestMaxDepth,
    onlyFiles: true,
    ignore: notSuiteDirectories.map(directory => `**/${directory}/**`),
  });
  const found = [...new Set([...near, ...conftests])];
  return {
    path: cwd,
    uriPath: folder.uri.path,
    hostVenv: hostVenv(context, folder),
    legacySettings: vscode.workspace.getConfiguration("ledgerDevTools", folder.uri).get("appSettings") ?? null,
    files: found.map(file => ({
      path: file,
      content: contentNotNeeded.has(path.posix.basename(file)) ? "" : readText(path.join(cwd, file)),
    })),
  };
}

export function collectFolders(context: vscode.ExtensionContext): FolderInput[] {
  return (vscode.workspace.workspaceFolders ?? []).map(folder => collectFolder(context, folder));
}

/** Whether a file that changed is one of those detection reads, close enough to its folder. */
export function isDetectionFile(uri: vscode.Uri): boolean {
  const folder = vscode.workspace.getWorkspaceFolder(uri);
  if (!folder || !detectionFileNames.includes(path.basename(uri.fsPath))) {
    return false;
  }
  const parts = path.relative(folder.uri.fsPath, uri.fsPath).split(path.sep);
  const skipped = path.basename(uri.fsPath) === conftestName ? notSuiteDirectories : ["node_modules"];
  return !parts.some(part => skipped.includes(part)) && parts.length <= maxDepthOf(uri.fsPath);
}
