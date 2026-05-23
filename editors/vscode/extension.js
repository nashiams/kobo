const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');

let client;

function activate(context) {
  const command = process.env.KOBO_LSP || 'kobo-lsp';
  client = new LanguageClient(
    'kobo',
    'Kobo',
    { command, args: ['--stdio'] },
    {
      documentSelector: [{ scheme: 'file', language: 'kobo' }],
      synchronize: {
        fileEvents: vscode.workspace.createFileSystemWatcher('**/.kobo/witnesses/*.kwit'),
      },
    },
  );

  context.subscriptions.push(client.start());
  context.subscriptions.push(vscode.commands.registerCommand('kobo.runScenario', runScenario));
  context.subscriptions.push(vscode.commands.registerCommand('kobo.replayWitness', replayWitness));
  context.subscriptions.push(
    vscode.commands.registerCommand('kobo.explainDiagnostic', explainDiagnostic),
  );
}

async function runScenario() {
  const document = vscode.window.activeTextEditor?.document;
  const file = document?.uri.fsPath;
  if (!file) {
    return;
  }
  await runInTerminal(`kobo test --sim quick --witness-dir .kobo/witnesses "${file}"`);
}

async function replayWitness(uri) {
  const witness = uri?.fsPath || (await prompt('Witness path'));
  if (!witness) {
    return;
  }
  await runInTerminal(`kobo replay "${witness}"`);
}

async function explainDiagnostic(code) {
  const diagnosticCode = typeof code === 'string' ? code : await prompt('Diagnostic code');
  if (!diagnosticCode) {
    return;
  }
  await runInTerminal(`kobo explain ${diagnosticCode}`);
}

async function prompt(placeHolder) {
  return vscode.window.showInputBox({ placeHolder });
}

async function runInTerminal(command) {
  const terminal = vscode.window.createTerminal('Kobo');
  terminal.show();
  terminal.sendText(command);
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
