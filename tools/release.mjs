#!/usr/bin/env node
// Cut a release: pick the next version from the commits since the last tag
// (feat → minor, anything else → patch; pre-1.0), add a changelog entry,
// bump package.json, commit and tag. Then `npm run deploy`.
//
//   node tools/release.mjs ["Release title"] [--dry-run] [--trailer "Key: value"]…
//
// Changelog text: lines in commit bodies starting with "Changelog: " are the
// friendly, user-facing bullets. Commits without any fall back to their
// subject line (minus the "feat:" prefix). chore/docs/test and merge commits
// are skipped.
// Trailers for the release commit (e.g. Co-Authored-By): each --trailer, and
// the lines of RELEASE_TRAILERS. None by default.
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, changelogLines, fail, git, usage } from './lib.mjs';

const args = process.argv.slice(2);
const dryRun = args.includes('--dry-run');
const positional = [];
const trailers = (process.env.RELEASE_TRAILERS ?? '').split('\n');
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--trailer') trailers.push(args[++i] ?? '');
  else if (!args[i].startsWith('--')) positional.push(args[i]);
}
const title = positional[0] ?? '';
const trailerLines = trailers.map((t) => t.trim()).filter(Boolean);
const notTrailer = trailerLines.find((t) => !/^[A-Za-z][\w-]*: \S/.test(t));
if (notTrailer) usage(`node tools/release.mjs ["Title"] [--dry-run] [--trailer "Key: value"]… ("${notTrailer}" isn't "Key: value")`);

// Only from a clean tree: the release commit and tag must describe what's committed.
const dirty = git('status', '--porcelain', '--untracked-files=no');
if (dirty && !dryRun) fail(`Release from a clean working tree. Commit or stash these first:\n${dirty}`);

let lastTag = '';
try {
  lastTag = git('describe', '--tags', '--abbrev=0', '--match', 'v*');
} catch {}
const range = lastTag ? `${lastTag}..HEAD` : 'HEAD';
const raw = git('log', range, '--format=%s%x1f%b%x1e');
const commits = raw.split('\x1e').map((c) => c.trim()).filter(Boolean).map((c) => {
  const [subject, body = ''] = c.split('\x1f');
  return { subject, body };
}).filter((c) => !/^chore\(release\)/.test(c.subject));
if (!commits.length) {
  console.log(`Nothing new since ${lastTag || 'the beginning'}.`);
  process.exit(0);
}

const items = [];
let minor = false;
for (const c of commits.reverse()) {
  const type = c.subject.match(/^(\w+)(?:\([^)]*\))?!?:/)?.[1] ?? '';
  if (type === 'feat') minor = true;
  const friendly = changelogLines(c.body);
  if (friendly.length) items.push(...friendly);
  else if (!/^Merge /.test(c.subject) && !['chore', 'docs', 'test', 'ci', 'build', 'style', 'refactor'].includes(type)) {
    const text = c.subject.replace(/^\w+(?:\([^)]*\))?!?:\s*/, '');
    items.push(text.charAt(0).toUpperCase() + text.slice(1));
  }
}

if (!items.length) {
  console.log(`Nothing user-facing since ${lastTag || 'the beginning'} (only chore/docs/test commits) — no release.`);
  process.exit(0);
}

const log = JSON.parse(fs.readFileSync(path.join(ROOT, 'web/changelog.json'), 'utf8'));
const [maj, min, pat] = log.current.split('.').map(Number);
const version = minor ? `${maj}.${min + 1}.0` : `${maj}.${min}.${pat + 1}`;
const entry = { version, date: new Date().toISOString().slice(0, 10), title: title || (minor ? 'New stuff' : 'Fixes'), items };
console.log(JSON.stringify(entry, null, 2));
if (dryRun) process.exit(0);

log.current = version;
log.releases.unshift(entry);
fs.writeFileSync(path.join(ROOT, 'web/changelog.json'), `${JSON.stringify(log, null, 2)}\n`);
const pkg = JSON.parse(fs.readFileSync(path.join(ROOT, 'package.json'), 'utf8'));
pkg.version = version;
fs.writeFileSync(path.join(ROOT, 'package.json'), `${JSON.stringify(pkg, null, 2)}\n`);
// --only: commit just these files, whatever else happens to be staged. (The
// build stamp isn't committed: `npm run deploy` makes it from what it ships.)
git('commit', '-q', '--only', '-m', `chore(release): v${version}${trailerLines.length ? `\n\n${trailerLines.join('\n')}` : ''}`, '--', 'web/changelog.json', 'package.json');
git('tag', '-a', `v${version}`, '-m', `v${version}${entry.title ? ` — ${entry.title}` : ''}`);
console.log(`Tagged v${version}. Deploy with: npm run deploy`);
