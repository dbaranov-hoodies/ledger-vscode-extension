import * as vscode from "vscode";
import { formatLog } from "./logFormat";
import type { LogNotification } from "./protocol";

/** A terminal that only shows what it is given. */
class OutputTerminal implements vscode.Pseudoterminal {
  private readonly writeEmitter = new vscode.EventEmitter<string>();
  private opened = false;
  private buffer: string[] = [];

  readonly onDidWrite = this.writeEmitter.event;

  constructor(private readonly onInterrupt: () => void) {}

  open(): void {
    this.opened = true;
    this.buffer.forEach(text => this.writeEmitter.fire(text));
    this.buffer = [];
  }

  close(): void {
    this.writeEmitter.dispose();
  }

  /** Ctrl-C stops what runs, as in any terminal. */
  handleInput(data: string): void {
    if (data.includes("\x03")) {
      this.write("^C\r\n");
      this.onInterrupt();
    }
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

/** One terminal for everything that runs: each line says which container it comes from. */
export class LogTerminal implements vscode.Disposable {
  private current?: { terminal: vscode.Terminal; pty: OutputTerminal };
  private readonly closed: vscode.Disposable;

  /** `onInterrupt` is called when the user presses Ctrl-C in the terminal. */
  constructor(private readonly onInterrupt: () => void) {
    this.closed = vscode.window.onDidCloseTerminal((closed) => {
      if (this.current?.terminal === closed) {
        this.current = undefined;
      }
    });
  }

  write(log: LogNotification): void {
    const { terminal, pty } = this.terminal();
    formatLog(log).forEach(line => pty.write(`${line}\r\n`));
    if (log.kind === "started") {
      // A new run asks for attention, without taking the focus from what the user is typing.
      terminal.show(true);
    }
  }

  dispose(): void {
    this.closed.dispose();
    this.current?.terminal.dispose();
    this.current = undefined;
  }

  private terminal(): { terminal: vscode.Terminal; pty: OutputTerminal } {
    if (!this.current) {
      const pty = new OutputTerminal(this.onInterrupt);
      this.current = { terminal: vscode.window.createTerminal({ name: "Ledger", pty }), pty };
    }
    return this.current;
  }
}
