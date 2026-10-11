// The encrypted Personal space's crypto (web/js/vault.js), run in Node's
// WebCrypto. Run: node test/vault.test.mjs
import assert from 'node:assert/strict';
import { createVault, openVault, rewrapVault, encryptNote, decryptNote, encryptBlob, decryptBlob, isEncryptedNote, sealBatches, unb64u } from '../web/js/vault.js';

const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
  }
}
const FAST = 100_000; // the real thing uses 600k; tests don't need to wait

await test('sealing earlier notes: batches fit one frame by size, not just count', async () => {
  const note = (i, size) => ({ id: `id-${i}`, body: `e1:${'A'.repeat(size)}` });
  // 25 long notes would have been one 2.5 MB frame (refused whole); now ~3 per batch.
  const long = Array.from({ length: 25 }, (_, i) => note(i, 100_000));
  const { batches, tooBig } = sealBatches(long);
  assert.equal(tooBig.length, 0);
  assert.equal(batches.flat().length, 25, 'every note is in a batch, once');
  for (const b of batches) assert.ok(JSON.stringify({ t: 'personal.seal', notes: b }).length < 384 * 1024, 'each batch fits a frame');
  // Many short notes: at most 50 a batch (the server's limit).
  const short = sealBatches(Array.from({ length: 120 }, (_, i) => note(i, 100)));
  assert.deepEqual(short.batches.map((b) => b.length), [50, 50, 20]);
  // One note bigger than any frame can't go.
  const huge = sealBatches([note(1, 10), note(2, 400_000), note(3, 10)]);
  assert.deepEqual(huge.tooBig.map((n) => n.id), ['id-2']);
  assert.deepEqual(huge.batches.flat().map((n) => n.id), ['id-1', 'id-3']);
});

await test('a note round-trips; the stored text gives nothing away', async () => {
  const { wrap, key } = await createVault('purple tiger river', FAST);
  const body = await encryptNote(key, { text: 'wifi at the cabin is on the fridge' });
  assert.ok(isEncryptedNote(body));
  assert.ok(!body.includes('cabin'));
  assert.deepEqual(await decryptNote(key, body), { text: 'wifi at the cabin is on the fridge' });
  // Another device: same passcode, same key.
  const again = await openVault(wrap, 'purple tiger river');
  assert.equal((await decryptNote(again, body)).text, 'wifi at the cabin is on the fridge');
});

await test('the wrap: right sizes, server-checkable shape', async () => {
  const { wrap } = await createVault('purple tiger river', FAST);
  assert.equal(wrap.v, 1);
  assert.equal(unb64u(wrap.salt).length, 16);
  assert.equal(unb64u(wrap.iv).length, 12);
  assert.equal(unb64u(wrap.ct).length, 48);
});

await test('wrong passcode is refused; tampering is caught', async () => {
  const { wrap, key } = await createVault('purple tiger river', FAST);
  await assert.rejects(openVault(wrap, 'purple tiger rivers'), { name: 'WrongPasscode' });
  const body = await encryptNote(key, { text: 'hi' });
  const flipped = body.slice(0, -2) + (body.at(-2) === 'A' ? 'B' : 'A') + body.at(-1);
  await assert.rejects(decryptNote(key, flipped));
  const other = (await createVault('another one entirely', FAST)).key;
  await assert.rejects(decryptNote(other, body), 'someone else’s key can’t open it');
});

await test('new passcode, same key: old notes still open', async () => {
  const { wrap, key } = await createVault('purple tiger river', FAST);
  const body = await encryptNote(key, { text: 'kept' });
  const next = await rewrapVault(wrap, 'purple tiger river', 'green ocean lamp');
  await assert.rejects(openVault(next, 'purple tiger river'));
  assert.equal((await decryptNote(await openVault(next, 'green ocean lamp'), body)).text, 'kept');
});

await test('pictures: bytes round-trip, and a note can carry a GIF and a picture reference', async () => {
  const { key } = await createVault('purple tiger river', FAST);
  const bytes = crypto.getRandomValues(new Uint8Array(5000));
  const sealed = await encryptBlob(key, bytes);
  assert.equal(sealed.length, 12 + 5000 + 16);
  assert.deepEqual(await decryptBlob(key, sealed), bytes);
  const payload = { text: 'look', image: { id: 'a'.repeat(32), width: 10, height: 20, mime: 'image/webp' }, gif: { provider: 'klipy', slug: 'x', title: 't', width: 1, height: 1 } };
  assert.deepEqual(await decryptNote(key, await encryptNote(key, payload)), payload);
});

for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.message ?? err}`);
}
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
