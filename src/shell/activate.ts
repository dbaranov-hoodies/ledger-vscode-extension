import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import type { InitializeParams } from "../generated/InitializeParams";
import { collectFolders, detectionFileNames, isDetectionFile } from "./folders";
import { LogTerminals } from "./logTerminals";
import { type RequestName, type Snapshot, webviewRequests } from "./protocol";
import { readSettings } from "./settings";
import { Sidecar } from "./sidecar";
import { ShellWebview } from "./webview";

const savedKey = "ledgerSaved";
const viewId = "appWebView";

function sidecarPath(context: vscode.ExtensionContext): string | undefined {
  const executable = process.platform === "win32" ? "ledger-sidecar.exe" : "ledger-sidecar";
  const target = path.join(context.extensionPath, "backend", "target");
  const configured = vscode.workspace.getConfiguration("ledgerDevTools").get<string>("sidecarPath");
  return [
    configured,
    path.join(context.extensionPath, "bin", executable),
    path.join(target, "debug", executable),
    path.join(target, "release", executable),
  ].find(candidate => candidate && fs.existsSync(candidate));
}

function platformName(): InitializeParams["platform"] {
  switch (process.platform) {
    case "win32":
      return "windows";
    case "darwin":
      return "macos";
    default:
      return "linux";
  }
}

function hostIds(): InitializeParams["hostIds"] {
  return typeof process.getuid === "function" && typeof process.getgid === "function"
    ? { uid: process.getuid(), gid: process.getgid() }
    : null;
}

/** Waits for changes to settle, so a burst of them is looked at once. */
function debounced(action: () => void, delay: number): () => void {
  let timer: NodeJS.Timeout | undefined;
  return () => {
    clearTimeout(timer);
    timer = setTimeout(action, delay);
  };
}

function show(level: "info" | "warning" | "error", text: string): void {
  switch (level) {
    case "info":
      vscode.window.showInformationMessage(text);
      break;
    case "warning":
      vscode.window.showWarningMessage(text);
      break;
    case "error":
      vscode.window.showErrorMessage(text);
      break;
  }
}

/**
 * The extension when the work is done by the sidecar: this only reads what VS Code knows (folders,
 * settings), shows what the sidecar says, and sends back what the user chooses.
 */
export async function activateShell(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel("Ledger DevTools");
  context.subscriptions.push(output);

  const binary = sidecarPath(context);
  if (!binary) {
    vscode.window.showErrorMessage(
      "The Ledger sidecar was not found. Build it with `npm run build:sidecar`, or set ledgerDevTools.sidecarPath.",
    );
    return;
  }

  const sidecar = new Sidecar(binary, output);
  const view = new ShellWebview(context);
  const terminals = new LogTerminals();
  context.subscriptions.push(
    sidecar,
    terminals,
    vscode.window.registerWebviewViewProvider(viewId, view, { webviewOptions: { retainContextWhenHidden: true } }),
  );

  const ask = async (method: RequestName, params: unknown) => {
    try {
      const result = await sidecar.request(method, params as never);
      if (method !== "runAction") {
        view.setSnapshot(result as Snapshot);
      }
    }
    catch (error) {
      vscode.window.showWarningMessage(error instanceof Error ? error.message : String(error));
    }
  };
  const refreshFolders = debounced(() => ask("refresh", { folders: collectFolders(context) }), 500);

  context.subscriptions.push(
    sidecar.on("snapshot", snapshot => view.setSnapshot(snapshot)),
    sidecar.on("saved", ({ saved }) => context.workspaceState.update(savedKey, saved)),
    sidecar.on("log", log => terminals.write(log)),
    sidecar.on("message", ({ level, text }) => show(level, text)),
    sidecar.on("actionFinished", (finished) => {
      view.actionFinished(finished);
      if (!finished.success) {
        vscode.window.showErrorMessage(`${finished.id} failed${finished.message ? `: ${finished.message}` : "."}`);
      }
    }),
    sidecar.onDidExit(reason => show("error", `The Ledger sidecar ${reason}. Reload the window to restart it.`)),
    view.onRequest(({ method, params }) => {
      if (webviewRequests.includes(method as RequestName)) {
        ask(method as RequestName, params);
      }
    }),
    view.onDidBecomeVisible(() => ask("refresh", { folders: null })),
    vscode.window.onDidChangeWindowState(({ focused }) => focused && ask("refresh", { folders: null })),
    vscode.workspace.onDidChangeWorkspaceFolders(refreshFolders),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("ledgerDevTools") && !event.affectsConfiguration("ledgerDevTools.appSettings")) {
        ask("updateSettings", readSettings(context));
      }
    }),
  );
  const watcher = vscode.workspace.createFileSystemWatcher(`**/{${detectionFileNames.join(",")}}`);
  const onFileEvent = (uri: vscode.Uri) => isDetectionFile(uri) && refreshFolders();
  context.subscriptions.push(
    watcher,
    watcher.onDidCreate(onFileEvent),
    watcher.onDidChange(onFileEvent),
    watcher.onDidDelete(onFileEvent),
  );

  try {
    const snapshot = await sidecar.request("initialize", {
      platform: platformName(),
      hostIds: hostIds(),
      display: process.env.DISPLAY ?? null,
      settings: readSettings(context),
      saved: context.workspaceState.get(savedKey) ?? null,
      folders: collectFolders(context),
    });
    view.setSnapshot(snapshot);
  }
  catch (error) {
    show("error", `The Ledger sidecar could not start: ${error instanceof Error ? error.message : String(error)}`);
  }
}
