import * as vscode from "vscode";
import type { LogNotification } from "./protocol";

const reset = "\x1b[0m";
const bold = "\x1b[1m";
const green = "\x1b[32m";
const red = "\x1b[31m";
const yellow = "\x1b[33m";
const dim = "\x1b[2m";

/** A terminal that only shows what it is given. */
class OutputTerminal implements vscode.Pseudoterminal {
  private readonly writeEmitter = new vscode.EventEmitter<string>();
  private opened = false;
  private buffer: string[] = [];

  readonly onDidWrite = this.writeEmitter.event;

  open(): void {
    this.opened = true;
    this.buffer.forEach(text => this.writeEmitter.fire(text));
    this.buffer = [];
  }

  close(): void {
    this.writeEmitter.dispose();
  }

  write(text: string): void {
    if (this.opened) {
      this.writeEmitter.fire(text);
    }
    else {
      this.buffer.push(text);
    }
  }
}

function title(source: string): string {
  return source === "image" ? "Ledger: image" : `Ledger: ${source}`;
}

/** One terminal for each source of output: the image, and the container of each device. */
export class LogTerminals implements vscode.Disposable {
  private readonly terminals = new Map<string, { terminal: vscode.Terminal; pty: OutputTerminal }>();
  private readonly closed: vscode.Disposable;

  constructor() {
    this.closed = vscode.window.onDidCloseTerminal((closed) => {
      for (const [source, entry] of this.terminals) {
        if (entry.terminal === closed) {
          this.terminals.delete(source);
        }
      }
    });
  }

  write(log: LogNotification): void {
    const entry = this.terminalFor(log.source);
    switch (log.kind) {
      case "started":
        entry.pty.write(`${bold}$ ${log.command}${reset}\r\n`);
        // A new run asks for attention, without taking the focus from what the user is typing.
        entry.terminal.show(true);
        break;
      case "line":
        entry.pty.write(`${log.stream === "stderr" ? dim : ""}${log.text}${reset}\r\n`);
        break;
      case "finished":
        entry.pty.write(log.success ? `${green}done${reset}\r\n` : `${red}failed${reset}\r\n`);
        break;
      case "note":
        entry.pty.write(`${yellow}${log.text}${reset}\r\n`);
        break;
    }
  }

  dispose(): void {
    this.closed.dispose();
    this.terminals.forEach(entry => entry.terminal.dispose());
    this.terminals.clear();
  }

  private terminalFor(source: string): { terminal: vscode.Terminal; pty: OutputTerminal } {
    const existing = this.terminals.get(source);
    if (existing) {
      return existing;
    }
    const pty = new OutputTerminal();
    const terminal = vscode.window.createTerminal({ name: title(source), pty });
    const entry = { terminal, pty };
    this.terminals.set(source, entry);
    return entry;
  }
}
