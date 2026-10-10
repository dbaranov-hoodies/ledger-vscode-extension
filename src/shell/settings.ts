import * as vscode from "vscode";
import type { SettingsInput } from "../generated/SettingsInput";

/** The settings of the extension that the sidecar depends on. */
export function readSettings(context: vscode.ExtensionContext): SettingsInput {
  const conf = vscode.workspace.getConfiguration("ledgerDevTools");
  const useHostKey = conf.get<boolean>("userScpPrivateKey") ?? false;
  const hostKey = process.env.SCP_PRIVKEY;
  if (useHostKey && !hostKey) {
    vscode.window.showWarningMessage(
      "The setting to use the SCP_PRIVKEY of the host is on, but this variable is not set.",
    );
  }
  return {
    dockerImage: conf.get<string>("dockerImage") ?? "",
    dockerRunArgs: conf.get<string>("dockerRunArgs") ?? "",
    autoUpdate: conf.get<boolean>("dockerAutoUpdate") ?? true,
    maxRetries: conf.get<number>("containerUpdateRetries") ?? 5,
    openAsRoot: conf.get<boolean>("openContainerAsRoot") ?? false,
    onboardingPin: conf.get<string>("onboardingPin") ?? "",
    onboardingSeed: conf.get<string>("onboardingSeed") ?? "",
    scpPrivateKey: useHostKey && hostKey ? hostKey : null,
    allowNanoX: conf.get<boolean>("enableDeviceOpsForNanoX") ?? false,
    defaultDevice: conf.get<string>("defaultDevice") ?? null,
    verboseTests: context.globalState.get<boolean>("verboseTests", false),
    testRequirements: conf.get<Record<string, string>>("additionalReqsPerApp") ?? {},
  };
}
