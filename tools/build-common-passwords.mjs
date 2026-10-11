#!/usr/bin/env node
// Builds worker/common-passwords.ts — the most common passwords of 8 or more
// characters, which sign-up and Change password refuse (worker/auth.ts) — from
// SecLists' list of the 100,000 most common passwords (MIT; drawn from Mark
// Burnett's 10 million password corpus). Pinned to one commit and checked by
// its SHA-256, so a rebuild gives the same file:
//
//   node tools/build-common-passwords.mjs            downloads the pinned file
//   node tools/build-common-passwords.mjs <file>     or reads a copy you have
//
// Kept: lowercased, 8–128 characters, most common first, no repeats, until the
// module would pass ~100 KB (the Worker carries it in every cold start).
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, fail } from './lib.mjs';

const COMMIT = '04a690dde43143a3b2a6733317c0916e21d85cde';
const FILE = 'Passwords/Common-Credentials/xato-net-10-million-passwords-100000.txt';
const URL = `https://raw.githubusercontent.com/danielmiessler/SecLists/${COMMIT}/${FILE}`;
const SHA256 = '1472aafa2561df5e3293aee252aee3ca660c12b399a283cf808bb01b39be388b';
const MAX_BYTES = 100_000;
const OUT = path.join(ROOT, 'worker', 'common-passwords.ts');

const LICENSE = `MIT License

Copyright (c) 2018 Daniel Miessler

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;

const local = process.argv[2];
const bytes = local ? fs.readFileSync(local) : Buffer.from(await (await fetch(URL)).arrayBuffer());
const sha = crypto.createHash('sha256').update(bytes).digest('hex');
if (sha !== SHA256) fail(`That isn't the pinned list (SHA-256 ${sha}, expected ${SHA256}).`);

const header = (count) => `// The ${count.toLocaleString('en-US')} most common passwords of 8 or more characters, lowercased,
// most common first: sign-up and Change password refuse them (worker/auth.ts).
// Made by tools/build-common-passwords.mjs (don't edit by hand) from SecLists,
// ${FILE}
// at commit ${COMMIT}
// (SHA-256 ${SHA256}), which comes from Mark Burnett's 10 million password corpus.
// SecLists' licence:
//
${LICENSE.split('\n').map((line) => `// ${line}`.trimEnd()).join('\n')}

export default \``;
const footer = '`;\n';
// In a template literal only these three need escaping.
const escape = (s) => s.replace(/\\/g, '\\\\').replace(/`/g, '\\`').replace(/\$\{/g, '\\${');

const kept = [];
const seen = new Set();
let size = Buffer.byteLength(header(99_999) + footer);
for (const raw of bytes.toString('utf8').split(/\r?\n/)) {
  const pw = raw.toLowerCase();
  if (pw.length < 8 || pw.length > 128 || seen.has(pw) || /[^\x20-\x7e]/.test(pw)) continue;
  const line = Buffer.byteLength(`${escape(pw)}\n`);
  if (size + line > MAX_BYTES) break;
  seen.add(pw);
  kept.push(pw);
  size += line;
}
const body = kept.map(escape).join('\n');
fs.writeFileSync(OUT, `${header(kept.length)}${body}${footer}`);
console.log(`${path.relative(ROOT, OUT)}: ${kept.length} passwords, ${fs.statSync(OUT).size} bytes`);
