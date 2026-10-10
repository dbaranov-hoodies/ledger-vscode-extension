"use strict";

import * as vscode from "vscode";
import { activateShell } from "./shell/activate";

console.log("Ledger: Loading extension");

export function activate(context: vscode.ExtensionContext) {
  console.log(`Ledger: activating extension`);
  // The panel is registered right away; what takes time is the first look at Docker.
  activateShell(context).catch(error => console.error("Ledger: activation failed", error));
}

export function deactivate() {
  console.log(`Ledger: deactivating extension`);
}
