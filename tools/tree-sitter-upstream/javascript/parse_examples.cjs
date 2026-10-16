// Frontend parse diagnostics only: never load imports, type-check or execute input.
const readline = require('node:readline');
const ts = require(process.argv[2]);
if (ts.version !== '6.0.3') throw Error('TypeScript 6.0.3 required');
const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', line => {
  const row = JSON.parse(line);
  const kinds = { typescript: ts.ScriptKind.TS, tsx: ts.ScriptKind.TSX, jsx: ts.ScriptKind.JSX };
  if (!(row.language in kinds)) throw Error('unexpected dialect');
  const source = ts.createSourceFile(row.path, row.source, ts.ScriptTarget.Latest, false, kinds[row.language]);
  const diagnostics = source.parseDiagnostics.map(d => ({
    code: d.code, start: d.start, length: d.length,
    message: ts.flattenDiagnosticMessageText(d.messageText, '\n'),
  }));
  process.stdout.write(JSON.stringify({ id: row.id, diagnostics }) + '\n');
});
