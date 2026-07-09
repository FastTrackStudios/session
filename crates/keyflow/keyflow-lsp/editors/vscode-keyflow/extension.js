// Minimal VS Code extension that boots keyflow-lsp.
const { workspace, ExtensionContext } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const cfg = workspace.getConfiguration("keyflow");
  const serverPath = cfg.get("serverPath", "keyflow-lsp");

  const serverOptions = {
    command: serverPath,
    transport: TransportKind.stdio,
  };

  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "keyflow" }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher("**/*.{kf,keyflow}"),
    },
  };

  client = new LanguageClient(
    "keyflow-lsp",
    "Keyflow Language Server",
    serverOptions,
    clientOptions
  );
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
