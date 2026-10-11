import { Database } from "bun:sqlite";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";

import type { Variant, Work } from "./contracts";
import { persistentVariant, persistentWork } from "./references";
import { mergeWorkMetadata, sameWork } from "./results";

export interface LibraryEntry {
    work: Work;
    variant?: Variant;
    favorite: boolean;
    lastOpened?: string;
    progress?: Record<string, string | number>;
    download?: { status: string; path?: string };
}

export class LibraryStore {
    private constructor(private readonly db: Database) {}

    static async open(dataDir: string): Promise<LibraryStore> {
        await mkdir(dataDir, { recursive: true });

        const db = new Database(join(dataDir, "library.sqlite"), { create: true, strict: true });
        db.exec(`
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS library (
                id TEXT PRIMARY KEY,
                entry TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                id TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        `);

        return new LibraryStore(db);
    }

    entries(mode = "all"): LibraryEntry[] {
        const rows = this.db.query("SELECT entry FROM library ORDER BY updated_at DESC LIMIT 500")
            .all() as { entry: string }[];

        return rows.map(row => JSON.parse(row.entry) as LibraryEntry)
            .filter(entry => mode === "favorites" ? entry.favorite : mode !== "history" || entry.lastOpened);
    }

    remember(work: Work, update: Partial<Omit<LibraryEntry, "work">>): LibraryEntry {
        if (!work.id || !work.title || !Array.isArray(work.variants))
            throw new Error("The item is incomplete.");

        const stableWork = persistentWork(work);
        const previous = this.find(stableWork);
        // Library rows are stable bookmarks. Resolve fresh acquisition URLs
        // when opening them; keep large episode inventories in the source.
        const selected = update.variant || previous?.variant;
        const variants = [selected, ...work.variants].filter((variant): variant is Variant => !!variant);
        const compact = [...new Map(variants.map(variant => [variant.id, persistentVariant(variant)])).values()].slice(0, 32);
        const entry: LibraryEntry = {
            favorite: false,
            ...previous,
            ...update,
            work: {
                ...mergeWorkMetadata(previous?.work, stableWork),
                id: previous?.work.id || stableWork.id,
                variants: compact,
            },
            ...(selected ? { variant: persistentVariant(selected) } : {}),
        };

        this.db.query("INSERT OR REPLACE INTO library (id, entry, updated_at) VALUES (?, ?, ?)")
            .run(entry.work.id, JSON.stringify(entry), Date.now());

        return entry;
    }

    get(id: string): LibraryEntry | undefined {
        const row = this.db.query("SELECT entry FROM library WHERE id = ?").get(id) as { entry: string } | null;
        return row ? JSON.parse(row.entry) : undefined;
    }

    find(work: Work): LibraryEntry | undefined {
        return this.get(work.id) || this.entries().find(entry => sameWork(entry.work, work));
    }

    update(id: string, changes: Partial<Omit<LibraryEntry, "work">>): void {
        const entry = this.get(id);
        if (entry)
            this.remember(entry.work, changes);
    }

    remove(id: string): void {
        this.db.query("DELETE FROM library WHERE id = ?").run(id);
    }

    setting<T>(id: string, initial: T): T {
        const row = this.db.query("SELECT value FROM settings WHERE id = ?").get(id) as { value: string } | null;
        return row ? JSON.parse(row.value) : initial;
    }

    setSetting(id: string, value: unknown): void {
        this.db.query("INSERT OR REPLACE INTO settings (id, value) VALUES (?, ?)")
            .run(id, JSON.stringify(value));
    }

    close(): void {
        this.db.close();
    }
}
