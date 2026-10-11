import { runNativeMessaging } from "../../components/content/browser/native.ts";

try {
    if (!await runNativeMessaging()) process.exitCode = 2;
} catch {
    // Native stdout is exclusively the binary browser protocol.
    process.exitCode = 1;
}
