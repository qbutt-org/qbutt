import type { Source } from "./contracts";
import { protectSettings, unprotectSettings } from "./secrets";
import { LibraryStore } from "./store";

export class SourceSettings {
    private readonly decrypted = new Map<string, Record<string, string>>();
    private readonly writes = new Map<string, Promise<void>>();

    constructor(private readonly store: LibraryStore) {}

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

    async describe(source: Source): Promise<Source> {
        const values = await this.values(source.id);
        return {
            ...source,
            enabled: this.store.setting(`source.enabled:${source.id}`, source.enabled),
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
