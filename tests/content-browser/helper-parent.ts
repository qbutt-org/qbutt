import type { Event } from "../../components/content/contracts";
import { NativeBrowserFixture } from "./native-parent";

export class HelperBrowserSession {
    private readonly parent: NativeBrowserFixture;

    constructor(discoveryPath: string, write: (message: string) => void) {
        this.parent = new NativeBrowserFixture({ discoveryPath });
        this.parent.onSessionReply = (_generation, response) => write(JSON.stringify({ browserReply: response }) + "\n");
    }

    receive(event: Event): boolean {
        if (event.id !== 0 || !["browserRequest", "browserCancel"].includes(event.event))
            return false;
        this.parent.forward(2, event.event, event.data);
        return true;
    }

    async close(): Promise<void> {
        this.parent.forward(2, "browserSessionClosed");
        await this.parent.close();
    }
}
