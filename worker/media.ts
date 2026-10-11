import { DurableObject } from 'cloudflare:workers';
import { runMigrations } from './schema';

/**
 * Every stored file — custom emojis, pictures, avatars, drawings, doodles,
 * soundboard clips, encrypted pictures — kept out of the Hub so the chat
 * database's 1 GB free-plan budget stays for messages. Blobs are keyed by a
 * hash of their bytes, so the bytes behind an id never change; a deleted
 * post's picture is erased, though (uploads.ts › serveBlob caches for that).
 */

const MIGRATIONS: ((sql: SqlStorage) => void)[] = [
  (sql) => {
    sql.exec(`
      CREATE TABLE blobs (
        id TEXT PRIMARY KEY,
        mime TEXT NOT NULL,
        data BLOB NOT NULL,
        size INTEGER NOT NULL,
        created_at INTEGER NOT NULL
      );
    `);
  },
];

export class Media extends DurableObject<Env> {
  private sql: SqlStorage;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    ctx.blockConcurrencyWhile(async () => {
      runMigrations(this.sql, MIGRATIONS);
    });
  }

  async put(id: string, mime: string, data: ArrayBuffer): Promise<void> {
    this.sql.exec(
      'INSERT OR IGNORE INTO blobs (id, mime, data, size, created_at) VALUES (?, ?, ?, ?, ?)',
      id, mime, data, data.byteLength, Date.now(),
    );
  }

  /** Erase one blob (a deleted post's picture nothing else uses). */
  async remove(id: string): Promise<void> {
    this.sql.exec('DELETE FROM blobs WHERE id = ?', id);
  }

  async get(id: string): Promise<{ mime: string; data: ArrayBuffer } | null> {
    const row = this.sql.exec<{ mime: string; data: ArrayBuffer }>('SELECT mime, data FROM blobs WHERE id = ?', id).toArray()[0];
    return row ? { mime: row.mime, data: row.data } : null;
  }

  async list(): Promise<{ id: string; mime: string; size: number }[]> {
    return this.sql.exec<{ id: string; mime: string; size: number }>('SELECT id, mime, size FROM blobs ORDER BY created_at').toArray();
  }

  async stats(): Promise<{ blobs: number; bytes: number; databaseBytes: number }> {
    const { n, bytes } = this.sql.exec<{ n: number; bytes: number | null }>('SELECT COUNT(*) AS n, SUM(size) AS bytes FROM blobs').one();
    return { blobs: n, bytes: bytes ?? 0, databaseBytes: this.ctx.storage.sql.databaseSize };
  }
}
