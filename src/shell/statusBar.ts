import * as vscode from "vscode";
import type { Snapshot } from "./protocol";

export const updateContainerCommand = "ledgerDevTools.updateContainer";
export const stopRunningCommand = "ledgerDevTools.stopRunning";

const imageToolTip = "Click to update image and respawn container.";
const outdatedToolTip = "Container is running but image is outdated. Click to update image and respawn container.";

/** The actions of the selected app that run now. */
export function runningActions(snapshot: Snapshot) {
  return (snapshot.app?.groups ?? [])
    .flatMap(group => [group.main, ...group.options])
    .filter(action => action?.running)
    .map(action => action!);
}

function icon(status: Snapshot["docker"]["status"]): string {
  switch (status) {
    case "running": return "sync";
    case "syncing": return "sync~spin";
    case "stopped": return "notebook-stop";
  }
}

/** The status bar items of the extension: the app and its containers, the device, the build mode. */
export class ShellStatusBar implements vscode.Disposable {
  private readonly containers = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
  private readonly device = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
  private readonly buildUseCase = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
  private readonly stop = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);

  constructor() {
    this.stop.command = stopRunningCommand;
    this.stop.tooltip = "Click to stop what is running.";
    this.stop.backgroundColor = new vscode.ThemeColor("statusBarItem.warningBackground");
    this.containers.command = updateContainerCommand;
    this.device.command = "selectTarget";
    this.device.tooltip = "Click to select another device.";
    this.buildUseCase.command = "buildUseCase";
    this.buildUseCase.tooltip = "Click to select a build use case.";
    for (const item of [this.device, this.buildUseCase]) {
      item.backgroundColor = new vscode.ThemeColor("statusBarItem.prominentBackground");
    }
  }

  update(snapshot: Snapshot): void {
    const app = snapshot.app;
    if (!app) {
      [this.containers, this.device, this.buildUseCase, this.stop].forEach(item => item.hide());
      return;
    }
    const running = runningActions(snapshot).map(action => action.label);
    if (running.length > 0) {
      this.stop.text = `$(debug-stop) L : stop ${running.join(", ")}`;
      this.stop.show();
    }
    else {
      this.stop.hide();
    }
    const { status, imageOutdated } = snapshot.docker;
    this.containers.text = `$(${icon(status)}) L : ${app.name}`;
    this.containers.backgroundColor = new vscode.ThemeColor(
      imageOutdated || status === "syncing"
        ? "statusBarItem.warningBackground"
        : status === "running" ? "statusBarItem.prominentBackground" : "statusBarItem.errorBackground",
    );
    this.containers.tooltip = imageOutdated
      ? `[outdated] ${outdatedToolTip}`
      : `[${status}] ${imageToolTip}`;
    this.containers.show();

    this.device.text = `$(target) L : ${app.devices.label}`;
    this.device.show();

    const useCase = app.buildUseCases.selected;
    if (useCase) {
      this.buildUseCase.text = `$(tools) L : ${useCase}`;
      this.buildUseCase.show();
    }
    else {
      this.buildUseCase.hide();
    }
  }

  dispose(): void {
    [this.containers, this.device, this.buildUseCase, this.stop].forEach(item => item.dispose());
  }
}
