import * as vscode from "vscode";
import type { ActionFinishedNotification } from "../generated/ActionFinishedNotification";
import type { Snapshot } from "./protocol";

export interface WebviewRequest {
  method: string;
  params: unknown;
}

/** How the user likes the panel, which is not about the apps: it is kept by the extension. */
interface UiState {
  pinned: string[];
  expanded: string[];
}

/** The panel of the extension. It shows the last snapshot, and sends back what the user does. */
export class ShellWebview implements vscode.WebviewViewProvider {
  private view?: vscode.WebviewView;
  private snapshot?: Snapshot;
  private readonly requestEmitter = new vscode.EventEmitter<WebviewRequest>();
  private readonly visibleEmitter = new vscode.EventEmitter<void>();

  readonly onRequest = this.requestEmitter.event;
  readonly onDidBecomeVisible = this.visibleEmitter.event;

  constructor(private readonly context: vscode.ExtensionContext) {}

  setSnapshot(snapshot: Snapshot): void {
    this.snapshot = snapshot;
    this.view?.webview.postMessage({ type: "snapshot", snapshot });
  }

  actionFinished(finished: ActionFinishedNotification): void {
    this.view?.webview.postMessage({ type: "actionFinished", id: finished.id, success: finished.success });
  }

  resolveWebviewView(view: vscode.WebviewView): void {
    this.view = view;
    view.webview.options = { enableScripts: true, localResourceRoots: [this.context.extensionUri] };
    view.webview.html = this.html(view.webview);
    view.onDidChangeVisibility(() => {
      if (view.visible) {
        this.visibleEmitter.fire();
      }
    });
    view.webview.onDidReceiveMessage((message) => {
      switch (message.type) {
        case "ready":
          this.sendUiState();
          if (this.snapshot) {
            view.webview.postMessage({ type: "snapshot", snapshot: this.snapshot });
          }
          break;
        case "request":
          this.requestEmitter.fire({ method: message.method, params: message.params });
          break;
        case "saveUiState":
          this.saveUiState(message);
          break;
        case "command":
          vscode.commands.executeCommand(message.command, ...(message.args ?? []));
          break;
      }
    });
  }

  private sendUiState(): void {
    const state = this.context.globalState;
    const uiState: UiState = {
      pinned: state.get<string[]>("pinnedActions", []),
      expanded: state.get<string[]>("expandedGroups", []),
    };
    this.view?.webview.postMessage({ type: "uiState", ...uiState });
  }

  private saveUiState(uiState: UiState): void {
    const state = this.context.globalState;
    state.update("pinnedActions", uiState.pinned ?? []);
    state.update("expandedGroups", uiState.expanded ?? []);
  }

  private html(webview: vscode.Webview): string {
    const asUri = (...parts: string[]) =>
      webview.asWebviewUri(vscode.Uri.joinPath(this.context.extensionUri, ...parts));
    return /* html */ `<!DOCTYPE html>
      <html lang="en">
      <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <link href="${asUri("dist", "codicon.css")}" rel="stylesheet" id="vscode-codicon-stylesheet" />
        <title>Ledger Dev Tools</title>
      </head>
      <body>
        <script>
          window.resourceUris = { wordmark: "${asUri("resources", "ledger-wordmark-transparent.png")}" };
        </script>
        <script src="${asUri("dist", "webview.js")}"></script>
      </body>
      </html>`;
  }
}
