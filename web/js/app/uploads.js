// Uploads from this connection: pictures, sealed pictures, clips and drawings.

import { settings, upload } from '../settings.js';
import { PIX_W, PIX_H } from '../draw.js';
import { firstTextChannel, state } from './state.js';
import { toast } from './ui.js';
import { queueSend } from './chat.js';

/** This connection's upload credentials (uploads need both: session ids are visible to everyone). */
const uploader = () => ({ sid: state.sid ?? '', ut: state.uploadToken ?? '' });

export async function postSound(blob, params) {
  return (await upload('/api/sound', blob, { ...uploader(), ...params }, 'audio/wav')).sound;
}

export function postBlob(bytes) {
  return upload('/api/blob', bytes, uploader());
}

export function postImage(path, blob) {
  return upload(path, blob, uploader());
}

/** Draw: a picture of a drawing (`page`) for the board; `pin` '0' leaves a taken-down one down. */
export async function uploadDrawing(blob, params = {}) {
  try {
    return (await upload('/api/drawing', blob, { ...uploader(), ...params })).drawing;
  } catch (err) {
    toast(`Couldn't pin it to the board: ${err.message}`);
    return null;
  }
}

/** Draw › Share: the drawing into the chat (just a picture: Save to board is what pins it). */
export async function shareDrawing(blob) {
  const channel = state.channels.get(settings.channel)?.kind === 'text' ? settings.channel : firstTextChannel()?.id;
  if (!channel) return;
  let stored;
  try {
    stored = await postImage('/api/image', blob);
  } catch (err) {
    return toast(`Couldn't share it: ${err.message}`);
  }
  queueSend({ t: 'send', channelId: channel, body: '', image: { id: stored.id, width: PIX_W, height: PIX_H } });
  toast(`Shared in ${state.channels.get(channel)?.name ?? 'the chat'}.`);
}
