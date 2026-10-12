// A tiny Yap client: signs in, shows a text channel live, and sends what you type.
// Node 22 or newer (it has WebSocket built in), no dependencies. It's a starting
// point for your own client or bot; PROTOCOL.md has every message.
//
//   node examples/tiny-client.mjs <server> <invite key> <name> <password> [--signup] [--channel <name>]
//
//   <server>      e.g. https://chat.example.com (or http://localhost:8787)
//   <invite key>  the key from your invite link (…/#key=THIS)
//   --signup      make the account first (on a new server, the first account is the owner;
//                 after that, new accounts wait until the owner or a mod lets them in)
//   --channel     which text channel to show (default: the first one)
import readline from 'node:readline';

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const [server, key, name, password] = args.filter((a, i) => !a.startsWith('--') && args[i - 1] !== '--channel');
if (!password) {
  console.error('Usage: node examples/tiny-client.mjs <server> <invite key> <name> <password> [--signup] [--channel <name>]');
  process.exit(1);
}

// The invite key travels as a WebSocket subprotocol ("key.<key>"), next to "yap".
const url = new URL('/ws', server);
url.protocol = url.protocol === 'http:' ? 'ws:' : 'wss:';
const ws = new WebSocket(url, ['yap', `key.${key}`]);
const send = (msg) => ws.send(JSON.stringify(msg));

const names = new Map(); // uid → name
let channel = null;

const print = (m) => {
  const time = new Date(m.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  console.log(`[${time}] ${names.get(m.uid) ?? 'someone'}: ${m.body || '(a picture or GIF)'}`);
};

ws.onopen = () => {
  // `hello` must come first: sign in (or sign up) with a name and password.
  const auth = flag('--signup') ? { signup: { name, pin: password, color: '#000080' } } : { login: { name, pin: password } };
  send({ t: 'hello', v: 1, client: 'tiny', ...auth });
  // Heartbeat: the bare text "ping" every ~15 s; the server answers "pong".
  setInterval(() => ws.send('ping'), 15_000).unref();
};

ws.onmessage = ({ data }) => {
  if (data === 'pong') return;
  const msg = JSON.parse(data);
  switch (msg.t) {
    case 'ready': {
      for (const u of msg.users) names.set(u.id, u.name);
      const texts = msg.channels.filter((c) => c.kind === 'text');
      channel = texts.find((c) => c.name === option('--channel')) ?? texts[0];
      console.log(`Signed in as ${name}. #${channel.name} (type a message and press Enter; Ctrl+C quits)`);
      send({ t: 'history', channelId: channel.id, limit: 20 });
      break;
    }
    case 'pending':
      console.log("You're in the waiting room: the owner or a mod has to let you in.");
      break;
    case 'history':
      if (msg.channelId === channel?.id) msg.messages.forEach(print);
      break;
    case 'message':
      if (msg.message.channelId === channel?.id) print(msg.message);
      break;
    case 'user':
      names.set(msg.user.id, msg.user.name);
      break;
    case 'error':
      console.error(`Yap says: ${msg.message} (${msg.code})`);
      break;
  }
};

ws.onclose = (e) => {
  console.log(`Disconnected${e.reason ? `: ${e.reason}` : ''}.`);
  process.exit(0);
};

// Every line you type is sent to the channel; Ctrl+D (end of input) signs off.
readline
  .createInterface({ input: process.stdin })
  .on('line', (line) => {
    if (channel && line.trim()) send({ t: 'send', channelId: channel.id, body: line });
  })
  .on('close', () => setTimeout(() => ws.close(), 500));
