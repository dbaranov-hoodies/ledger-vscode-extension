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

/** Of these, only their presence matters. */
const contentNotNeeded = new Set(["conftest.py", "docker-compose.yml"]);
const maxDepth = 2;

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
  const found = fg.sync(detectionFileNames.map(name => `**/${name}`), {
    cwd,
    deep: maxDepth,
    onlyFiles: true,
    ignore: ["**/node_modules/**", "**/.git/**"],
  });
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
  const relative = path.relative(folder.uri.fsPath, uri.fsPath);
  return !relative.split(path.sep).includes("node_modules") && relative.split(path.sep).length <= maxDepth;
}
