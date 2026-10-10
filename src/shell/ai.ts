import * as vscode from "vscode";
import { runAIReview, setSelectedModel } from "../aiReviewer";

/** The AI review: it needs the language models of VS Code, so the extension runs it itself. */
export class AiReview implements vscode.Disposable {
  private models: string[] = [];
  private selected?: string;
  private readonly diagnostics = vscode.languages.createDiagnosticCollection("ledger-ai-review");
  private readonly changed: vscode.Disposable;

  /** `report` is called with the models and the selected one, whenever they change. */
  constructor(
    private readonly output: vscode.OutputChannel,
    private readonly report: (models: string[], selected: string | null) => void,
  ) {
    this.changed = vscode.lm.onDidChangeChatModels(() => this.refresh());
  }

  async refresh(): Promise<void> {
    const found = await vscode.lm.selectChatModels();
    this.models = found.map(model => model.name).filter((name, at, all) => all.indexOf(name) === at);
    if (!this.selected || !this.models.includes(this.selected)) {
      this.selected = this.models[0];
    }
    this.announce();
  }

  select(name: string): void {
    if (this.models.includes(name)) {
      this.selected = name;
      this.announce();
    }
  }

  /** Returns whether the review went through. */
  async run(appFolder: string): Promise<boolean> {
    try {
      await runAIReview(this.diagnostics, this.output, vscode.Uri.file(appFolder));
      return true;
    }
    catch (error) {
      this.output.appendLine(`AI review failed: ${error instanceof Error ? error.message : String(error)}`);
      return false;
    }
  }

  dispose(): void {
    this.changed.dispose();
    this.diagnostics.dispose();
  }

  private announce(): void {
    if (this.selected) {
      setSelectedModel(this.selected);
    }
    this.report(this.models, this.selected ?? null);
  }
}
