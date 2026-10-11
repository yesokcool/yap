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

export async function uploadDrawing(blob) {
  try {
    return (await postImage('/api/drawing', blob)).drawing;
  } catch (err) {
    toast(`Couldn't save to the gallery: ${err.message}`);
    return null;
  }
}

/** Draw › Share: the drawing goes to the gallery and into the chat. */
export async function shareDrawing(blob) {
  const drawing = await uploadDrawing(blob);
  if (!drawing) return;
  const channel = state.channels.get(settings.channel)?.kind === 'text' ? settings.channel : firstTextChannel()?.id;
  if (!channel) return;
  queueSend({ t: 'send', channelId: channel, body: '', image: { id: drawing.media, width: PIX_W, height: PIX_H } });
  toast(`Shared in ${state.channels.get(channel)?.name ?? 'the chat'}.`);
}
