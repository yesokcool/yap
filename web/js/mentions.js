// Who a message @-mentions. One rule for the server (pushes, away replies;
// worker/hub-messages.ts imports this file) and every page (the highlight on your
// messages, the mention badge and sound). It's the rule format.js ›
// renderBody highlights by: a member's name in any case, longest names first
// and each mention used once (so "@Sam Lee" is Sam Lee, never also Sam), not
// right after a letter, digit or @ (no e-mail addresses), and never inside
// code, links or custom emoji tokens (<:name:id>). Those are covered up
// first, and a covered-up span counts as a word: "`x`@ben" or
// "<:wave:…>@ben" mentions nobody, on every side. test/unit.test.mjs checks
// the two agree.

const CODE_BLOCK = /```[\s\S]*?```/g;
const CODE = /`[^`\n]+`/g;
const LINK = /\bhttps?:\/\/[^\s<]+/g;
const CUSTOM_EMOJI = /<:[A-Za-z0-9_]{2,32}:[a-f0-9]{32}>/g;
/** Covered up: same length, and a word character on both sides, like the highlighter's placeholders. */
const cover = (s) => 'x'.repeat(s.length);

/**
 * @param {string} body
 * @param {{ id: string, name: string }[]} users
 * @returns {{ uids: string[], everyone: boolean }} who is mentioned, and whether it says @everyone or @here
 */
export function findMentions(body, users) {
  let text = String(body ?? '').replace(CODE_BLOCK, cover).replace(CODE, cover).replace(CUSTOM_EMOJI, cover).replace(LINK, cover).toLowerCase();
  /** Where `needle` stands as a mention. */
  const at = (needle) => {
    const found = [];
    for (let i = text.indexOf(needle); i >= 0; i = text.indexOf(needle, i + 1)) {
      if (!/[\w@]/.test(text[i - 1] ?? ' ') && !/\w/.test(text[i + needle.length] ?? ' ')) found.push(i);
    }
    return found;
  };
  const everyone = at('@everyone').length > 0 || at('@here').length > 0;
  const uids = [];
  for (const user of [...users].filter((u) => u.name).sort((a, b) => b.name.length - a.name.length)) {
    const needle = `@${user.name.toLowerCase()}`;
    const found = at(needle);
    if (!found.length) continue;
    uids.push(user.id);
    // Taken: a shorter name can't match inside these any more.
    for (const i of found) text = text.slice(0, i) + cover(needle) + text.slice(i + needle.length);
  }
  return { uids, everyone };
}
