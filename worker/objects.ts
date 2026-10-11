// The Worker's way to its two Durable Objects: one instance of each, "main".
export const hub = (env: Env) => env.HUB.get(env.HUB.idFromName('main'));
export const media = (env: Env) => env.MEDIA.get(env.MEDIA.idFromName('main'));
