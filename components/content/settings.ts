import type { SearchRequest, Source, Variant } from "./contracts";
import { ContentError } from "./errors";
import { safeError } from "./net";
import { persistentVariant, publicReference } from "./references";
import { protectSettings, unprotectSettings } from "./secrets";
import { LibraryStore } from "./store";

export type SourceCheck = { method: "search"; request: SearchRequest }
    | { method: "inspect" | "resolve"; variant: Variant };
type SourceFailure = NonNullable<Source["health"]> & { check?: SourceCheck };

export class SourceSettings {
    private readonly decrypted = new Map<string, Record<string, string>>();
    private readonly writes = new Map<string, Promise<void>>();

    constructor(private readonly store: LibraryStore) {}

    async hasConfiguredAuthorization(source: Source): Promise<boolean> {
        return !source.authorizationSetting || !!(await this.values(source.id))[source.authorizationSetting]?.trim();
    }

    async values(id: string): Promise<Record<string, string>> {
        const cached = this.decrypted.get(id);
        if (cached)
            return { ...cached };

        const encrypted = this.store.setting<string>(`source.credentials:${id}`, "");
        const values = encrypted ? await unprotectSettings(encrypted) : {};
        this.decrypted.set(id, values);
        return { ...values };
    }

    configure(id: string, enabled?: boolean, changes?: Record<string, string>): Promise<void> {
        const write = (this.writes.get(id) || Promise.resolve()).catch(() => {}).then(async () => {
            if (changes) {
                const values = { ...await this.values(id), ...changes };
                this.store.setSetting(`source.credentials:${id}`, await protectSettings(values));
                this.decrypted.set(id, values);
            }
            if (typeof enabled === "boolean")
                this.store.setSetting(`source.enabled:${id}`, enabled);
        }).finally(() => {
            if (this.writes.get(id) === write)
                this.writes.delete(id);
        });
        this.writes.set(id, write);
        return write;
    }

    recordHealth(id: string, error?: unknown, check?: SourceCheck): void {
        const message = error === undefined ? undefined : safeError(typeof error === "string" ? new Error(error) : error);
        if (check?.method === "inspect") check = { ...check, variant: persistentVariant(check.variant) };
        if (check?.method === "resolve") {
            const stable = persistentVariant(check.variant);
            const identity = ["id", "sourceId", "sourceName", "title", "url", "transport", "magnet", "infoHash",
                "version", "edition", "cooperative", "publicationKind", "episode", "season", "translation", "player"] as const;
            check = { ...check, variant: Object.fromEntries(identity.map(key => {
                const value = stable[key];
                return [key, typeof value === "string" || key === "cooperative" && typeof value === "boolean" ? value : undefined];
            })) as unknown as Variant };
        }
        const failed = this.failedCheck(id);
        if (!message && failed && JSON.stringify(failed) !== JSON.stringify(check)) return;
        this.store.setSetting(`source.health:${id}`, message
            ? { status: "error", message, checkedAt: new Date().toISOString(), check,
                ...(error instanceof ContentError ? { code: error.code, url: publicReference(error.url) } : {}) } : null);
    }

    health(id: string): Source["health"] {
        const failure = this.store.setting<SourceFailure | null>(`source.health:${id}`, null);
        return failure ? { status: failure.status, message: failure.message, checkedAt: failure.checkedAt,
            code: failure.code, url: failure.url } : undefined;
    }

    failedCheck(id: string): SourceCheck | undefined {
        return this.store.setting<SourceFailure | null>(`source.health:${id}`, null)?.check;
    }

    async describe(source: Source): Promise<Source> {
        const values = await this.values(source.id);
        return {
            ...source,
            enabled: this.store.setting(`source.enabled:${source.id}`, source.enabled),
            health: this.health(source.id),
            settings: source.settings?.map(setting => {
                let value = Object.hasOwn(values, setting.name) ? values[setting.name] : setting.value;
                if (setting.type === "password")
                    value = "";
                else if (setting.type === "boolean")
                    value = value === true || value === "true";
                return { ...setting, configured: !!values[setting.name], value };
            }),
        };
    }
}
