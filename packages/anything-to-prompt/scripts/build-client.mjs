#!/usr/bin/env node
// Assemble packages/anything-to-prompt/lib/client/*.js into lib/client.js
// Every part file is valid standalone JS so linters/checkers validate each module independently.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const pkgRoot = path.resolve(__dirname, '..');
const partsDir = path.join(pkgRoot, 'lib', 'client');
const outFile = path.join(pkgRoot, 'lib', 'client.js');

const partFiles = fs
  .readdirSync(partsDir)
  .filter((name) => name.endsWith('.js'))
  .sort();

if (partFiles.length === 0) {
  throw new Error(`No client part files found in ${partsDir}`);
}

const chunks = [
  '// @local/anything-to-prompt/lib/client.js — Anything-to-Prompt Ambient Floating Bar + Session Picker + Tauri Bridge (v1.0.0)',
  '// GENERATED from packages/anything-to-prompt/lib/client/*.js — edit modular files in lib/client/ and run `node scripts/build-client.mjs`.',
  '// Only activates inside Anything-to-Prompt\'s own floating window (?dsh_bar_clone=1); never injects into DSH\'s normal WebUI.',
  '',
  'window.__ModuleLoader__.load({',
  "  id: '@local/anything-to-prompt',",
  '  factory(require) {',
];

for (const name of partFiles) {
  const fullPath = path.join(partsDir, name);
  execFileSync(process.execPath, ['--check', fullPath], { stdio: 'inherit' });
  const src = fs.readFileSync(fullPath, 'utf8').replace(/\r\n/g, '\n').trimEnd();
  chunks.push(`    // === module: lib/client/${name} ===`);
  chunks.push(src);
  chunks.push('');
}

chunks.push('    return createAnythingToPromptPlugin();');
chunks.push('  },');
chunks.push('});');
chunks.push('');

fs.writeFileSync(outFile, chunks.join('\n'), 'utf8');
execFileSync(process.execPath, ['--check', outFile], { stdio: 'inherit' });
console.log(`Built ${outFile} from ${partFiles.length} modules (${partFiles.join(', ')}).`);
