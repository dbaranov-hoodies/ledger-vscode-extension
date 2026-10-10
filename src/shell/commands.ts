import * as vscode from "vscode";
import type { RequestName, Requests, Snapshot } from "./protocol";
import { updateContainerCommand } from "./statusBar";

/** What the commands need of the extension. */
export interface CommandContext {
  snapshot(): Snapshot | undefined;
  ask<M extends RequestName>(method: M, params: Requests[M]["params"]): Promise<void>;
}

interface Pick<T> extends vscode.QuickPickItem {
  value: T;
}

async function pick<T>(placeHolder: string, items: Pick<T>[]): Promise<T | undefined> {
  const chosen = await vscode.window.showQuickPick(items, { placeHolder });
  return chosen?.value;
}

/** The commands of the palette and the status bar, which choose with a quick pick. */
export function registerCommands(context: vscode.ExtensionContext, shell: CommandContext): void {
  const register = (command: string, run: (app: NonNullable<Snapshot["app"]>) => Promise<void>) =>
    context.subscriptions.push(
      vscode.commands.registerCommand(command, async () => {
        const app = shell.snapshot()?.app;
        if (app) {
          await run(app);
        }
      }),
    );

  register("selectTarget", async (app) => {
    const devices = app.devices.available.map(device => ({ label: device.name, value: [device.id] }));
    const all = app.devices.available.length > 1
      ? [{ label: "All", value: app.devices.available.map(device => device.id) }]
      : [];
    const chosen = await pick("Please select a target", [...all, ...devices]);
    if (chosen) {
      await shell.ask("setDevices", { devices: chosen });
    }
  });

  register("selectVariant", async (app) => {
    const items = (app.variants?.items ?? []).map(value => ({ label: value, value }));
    const chosen = await pick("Please select a variant", items);
    if (chosen) {
      await shell.ask("setVariant", { value: chosen });
    }
  });

  register("buildUseCase", async (app) => {
    const items = app.buildUseCases.items.map(value => ({ label: value, value }));
    const chosen = await pick("Please select a use case", items);
    if (chosen) {
      await shell.ask("setBuildUseCase", { name: chosen });
    }
  });

  register("selectCheck", async (app) => {
    const items = app.enforcerChecks.items.map(value => ({ label: value, value }));
    const chosen = await pick("Please select a check to run", items);
    if (chosen) {
      await shell.ask("setEnforcerCheck", { name: chosen });
    }
  });

  context.subscriptions.push(
    vscode.commands.registerCommand("showAppList", async () => {
      const apps = shell.snapshot()?.apps ?? [];
      const items = apps.map(app => ({ label: app.name, description: app.folder, value: app.folder }));
      const chosen = await pick("Please select an app", items);
      if (chosen) {
        await shell.ask("selectApp", { folder: chosen });
      }
    }),
    vscode.commands.registerCommand(updateContainerCommand, () =>
      shell.ask("runAction", { id: "Update Container" })),
  );
}
