'use strict';

// Observe selected route behavior. Expectations live outside this program.
const fs = require('node:fs');
const http = require('node:http');
const { once } = require('node:events');
const express = require(process.argv[2]);
const cases = JSON.parse(fs.readFileSync(process.argv[3], 'utf8')).cases;
const sourceLines = fs.readFileSync(__filename, 'utf8').split('\n');
const registrationLine = sourceLines.findIndex(line => line.trimEnd().endsWith('// observation: registration')) + 1;
const registrationColumn = sourceLines[registrationLine - 1].indexOf('router.get') + 1;

function request(port, method, path) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: '127.0.0.1', port, method, path }, res => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', chunk => { body += chunk; });
      res.on('end', () => {
        let params = null;
        if (body && res.statusCode === 200) params = JSON.parse(body).params;
        resolve({ status: res.statusCode, params });
      });
    });
    req.setTimeout(3000, () => req.destroy(new Error('local request timeout')));
    req.on('error', reject);
    req.end();
  });
}

async function observe(scenario) {
  const app = express();
  const router = express.Router();
  const pattern = scenario.pattern_regex ? new RegExp(scenario.pattern_regex) : scenario.pattern;
  const location = { path: 'probe.cjs', line: registrationLine, column: registrationColumn,
    end_column: registrationColumn + 'router.get'.length, excerpt: 'router.get' };
  try {
    router.get(pattern, (req, res) => res.json({ params: req.params })); // observation: registration
    app.use(router);
  } catch (error) {
    return { id: scenario.id, location, actual: { registration: 'throws', responses: [] },
      exception: { name: error.name, message: error.message, stack: error.stack } };
  }
  const server = app.listen(0, '127.0.0.1');
  try {
    await once(server, 'listening');
    const responses = [];
    for (const input of scenario.requests) {
      responses.push(await request(server.address().port, input.method, input.path));
    }
    return { id: scenario.id, location, actual: { registration: 'accepted', responses } };
  } finally {
    await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
}

(async () => {
  const observations = [];
  for (const scenario of cases) observations.push(await observe(scenario));
  process.stdout.write(JSON.stringify({ observations }));
})().catch(error => { console.error(error); process.exitCode = 1; });
