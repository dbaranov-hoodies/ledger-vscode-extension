import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import type { InitializeParams } from "../generated/InitializeParams";
import { Wizard } from "../wizard";
import { AiReview } from "./ai";
import { registerCommands } from "./commands";
import { collectFolders, detectionFileNames, isDetectionFile } from "./folders";
import { LogTerminal } from "./logTerminal";
import { type RequestName, type Snapshot, webviewRequests } from "./protocol";
import { readSettings } from "./settings";
import { Sidecar } from "./sidecar";
import { runningActions, ShellStatusBar, stopRunningCommand } from "./statusBar";
import { ShellWebview } from "./webview";

const savedKey = "ledgerSaved";
const viewId = "appWebView";
const aiReviewId = "AI Review";

/** The sidecar that comes with the extension, or the one built in the repository. */
function sidecarPath(context: vscode.ExtensionContext): string | undefined {
  const executable = process.platform === "win32" ? "ledger-sidecar.exe" : "ledger-sidecar";
  const target = path.join(context.extensionPath, "backend", "target");
  return [
    path.join(context.extensionPath, "bin", executable),
    path.join(target, "debug", executable),
    path.join(target, "release", executable),
  ].find(candidate => fs.existsSync(candidate));
}

/** An extension package may not keep the permission to run the file. */
function allowToRun(binary: string): void {
  if (process.platform !== "win32") {
    try {
      fs.chmodSync(binary, 0o755);
    }
    catch {
      // If it cannot be changed, it may already be fine, and starting it will tell.
    }
  }
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
 * The extension: the work is done by the sidecar. This only reads what VS Code knows (folders,
 * settings, language models), shows what the sidecar says, and sends back what the user chooses.
 */
export async function activateShell(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel("Ledger DevTools");
  context.subscriptions.push(output);

  let latest: Snapshot | undefined;
  let announceFirst!: (snapshot: Snapshot) => void;
  const firstSnapshot = new Promise<Snapshot>(resolve => (announceFirst = resolve));
  new Wizard(context, firstSnapshot);

  const binary = sidecarPath(context);
  if (!binary) {
    vscode.window.showErrorMessage(
      "The Ledger sidecar was not found. Build it with `npm run build:sidecar`.",
    );
    return;
  }
  allowToRun(binary);

  const sidecar = new Sidecar(binary, output);
  const view = new ShellWebview(context);
  // Ctrl-C in the terminal of the extension stops what is running.
  const terminal = new LogTerminal(() => stopRunning());
  const statusBar = new ShellStatusBar();
  context.subscriptions.push(
    sidecar,
    terminal,
    statusBar,
    vscode.window.registerWebviewViewProvider(viewId, view, { webviewOptions: { retainContextWhenHidden: true } }),
  );

  const publish = (snapshot: Snapshot) => {
    latest = snapshot;
    view.setSnapshot(snapshot);
    statusBar.update(snapshot);
    announceFirst(snapshot);
  };
  const ask = async (method: RequestName, params: unknown) => {
    try {
      const result = await sidecar.request(method, params as never);
      if (method !== "runAction" && method !== "cancelAction") {
        publish(result as Snapshot);
      }
    }
    catch (error) {
      vscode.window.showWarningMessage(error instanceof Error ? error.message : String(error));
    }
  };
  const stopRunning = () => {
    const running = latest ? runningActions(latest) : [];
    for (const action of running) {
      // It may have ended on its own meanwhile, and then there is nothing to say.
      sidecar.request("cancelAction", { id: action.id }).catch(() => undefined);
    }
  };
  context.subscriptions.push(vscode.commands.registerCommand(stopRunningCommand, stopRunning));
  const refreshFolders = debounced(() => ask("refresh", { folders: collectFolders(context) }), 500);

  const ai = new AiReview(output, (models, selected) => ask("setAi", { models, selected }));
  const runAiReview = async () => {
    const folder = latest?.app?.folder;
    if (folder) {
      const success = await ai.run(folder);
      view.actionFinished({ id: aiReviewId, success, cancelled: false, message: null });
    }
  };
  context.subscriptions.push(ai, vscode.commands.registerCommand("aiReview", runAiReview));

  // Opens the file of a test, where the test starts.
  const openTest = async (id: string) => {
    try {
      const location = await sidecar.request("locateTest", { id });
      const document = await vscode.workspace.openTextDocument(location.path);
      const start = new vscode.Position(location.line - 1, 0);
      const range = new vscode.Range(start, start);
      const editor = await vscode.window.showTextDocument(document, { selection: range });
      editor.revealRange(range, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
    }
    catch (error) {
      vscode.window.showWarningMessage(error instanceof Error ? error.message : String(error));
    }
  };
  const setVerboseTests = (verbose: boolean) => {
    context.globalState.update("verboseTests", verbose);
    ask("updateSettings", readSettings(context));
  };
  // The extra requirements of the tests are a setting of the user, by the name of the app folder.
  const setTestRequirements = (text: string) => {
    const app = latest?.app;
    if (app) {
      const conf = vscode.workspace.getConfiguration("ledgerDevTools");
      const current = conf.get<Record<string, string>>("additionalReqsPerApp") ?? {};
      conf.update("additionalReqsPerApp", { ...current, [app.name]: text }, vscode.ConfigurationTarget.Global);
    }
  };
  registerCommands(context, { snapshot: () => latest, ask });

  context.subscriptions.push(
    sidecar.on("snapshot", publish),
    sidecar.on("saved", ({ saved }) => context.workspaceState.update(savedKey, saved)),
    sidecar.on("log", log => terminal.write(log)),
    sidecar.on("message", ({ level, text }) => show(level, text)),
    sidecar.on("actionFinished", (finished) => {
      view.actionFinished(finished);
      if (finished.cancelled) {
        vscode.window.showInformationMessage(`${finished.id} was stopped.`);
      }
      else if (!finished.success) {
        vscode.window.showErrorMessage(`${finished.id} failed${finished.message ? `: ${finished.message}` : "."}`);
      }
    }),
    sidecar.onDidExit(reason => show("error", `The Ledger sidecar ${reason}. Reload the window to restart it.`)),
    view.onRequest(({ method, params }) => {
      const id = (params as { id?: string } | undefined)?.id;
      if (method === "runAction" && id === aiReviewId) {
        runAiReview();
      }
      else if (method === "setAiModel") {
        ai.select((params as { name: string }).name);
      }
      else if (method === "openTest") {
        openTest((params as { id: string }).id);
      }
      else if (method === "setVerboseTests") {
        setVerboseTests((params as { value: boolean }).value);
      }
      else if (method === "setTestRequirements") {
        setTestRequirements((params as { text: string }).text);
      }
      else if (webviewRequests.includes(method as RequestName)) {
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
    publish(snapshot);
    ai.refresh();
  }
  catch (error) {
    show("error", `The Ledger sidecar could not start: ${error instanceof Error ? error.message : String(error)}`);
  }
}
