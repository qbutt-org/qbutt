import { dirname, join } from "node:path";

// Native children discover this isolated helper even when Edge omits inherited
// environment variables. The application profile is supplied by the driver.
process.env.QBUTT_BROWSER_BRIDGE_DISCOVERY = join(dirname(process.execPath), "live-browser-bridge.json");
await import("../../components/content/main.ts");
