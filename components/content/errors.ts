export class ContentError extends Error {
    constructor(readonly code: string, message: string, readonly url?: string) {
        super(message);
        this.name = "ContentError";
    }
}
