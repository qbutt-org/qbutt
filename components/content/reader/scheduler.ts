import type { ReaderDocument, ReaderEvent, ReaderPage, ReaderPosition } from "./types";

interface Schedule {
    observation: ReaderPosition;
    observedAt: number;
    secondsPerUnit: number;
    processingSeconds: number;
    images: boolean;
    chapters: string[];
    stop?: AbortController;
    job?: AbortController;
    target?: { chapterId: string; imageIndex?: number };
}

const schedules = new Map<string, Schedule>();

export function observeReading(documentId: string, observation: ReaderPosition): void {
    const schedule = schedules.get(documentId);
    if (!schedule) {
        return;
    }
    const now = performance.now();
    const elapsed = (now - schedule.observedAt) / 1000;
    const previous = schedule.observation;
    const chapter = schedule.chapters.indexOf(observation.chapterId);
    const oldChapter = schedule.chapters.indexOf(previous.chapterId);
    const distance = schedule.images
        ? chapter === oldChapter ? (observation.imageIndex ?? 0) - (previous.imageIndex ?? 0) : 0
        : chapter - oldChapter + (observation.offset ?? 0) - (previous.offset ?? 0);
    if (distance > 0 && distance <= (schedule.images ? 3 : 1.2) && elapsed >= 0.2 && elapsed <= 300) {
        const minimum = schedule.images ? 5 : 30;
        const measured = Math.min(schedule.images ? 300 : 3600, Math.max(minimum, elapsed / distance));
        schedule.secondsPerUnit = (schedule.secondsPerUnit * 0.6) + (measured * 0.4);
    }
    schedule.observation = observation;
    if (!schedule.images || chapter !== oldChapter || observation.imageIndex !== previous.imageIndex) {
        schedule.observedAt = now;
    }
    const target = schedule.target;
    if (!target) {
        return;
    }
    const targetChapter = schedule.chapters.indexOf(target.chapterId);
    const obsolete = targetChapter < chapter || targetChapter > chapter + 2
        || (schedule.images && targetChapter === chapter && (target.imageIndex ?? 0) < (observation.imageIndex ?? 0));
    if (obsolete) {
        schedule.job?.abort();
    }
}

export async function readAhead(document: ReaderDocument, observation: ReaderPosition, signal: AbortSignal,
    readChapter: (chapterId: string, signal: AbortSignal) => Promise<ReaderPage>,
    translate: (page: ReaderPage, imageIndex: number | undefined, signal: AbortSignal) => Promise<ReaderPage>,
    onEvent: ReaderEvent): Promise<{ processed: number; processingSeconds: number }> {
    const previous = schedules.get(document.id);
    observeReading(document.id, observation);
    previous?.stop?.abort();
    const stop = new AbortController();
    const lifetime = AbortSignal.any([signal, stop.signal]);
    const schedule: Schedule = {
        observation,
        observedAt: previous?.observedAt ?? performance.now(),
        secondsPerUnit: previous?.secondsPerUnit ?? 240,
        processingSeconds: previous?.processingSeconds ?? 15,
        images: previous?.images ?? false,
        chapters: document.chapters.map(chapter => chapter.id),
        stop,
    };
    schedules.delete(document.id);
    schedules.set(document.id, schedule);
    for (const [id, entry] of schedules) {
        if (schedules.size <= 32) {
            break;
        }
        if (!entry.stop) {
            schedules.delete(id);
        }
    }
    const completed = new Set<string>();
    let processed = 0;
    try {
        while (processed < (schedule.images ? 7 : 3)) {
            lifetime.throwIfAborted();
            const current = schedule.observation;
            const chapterIndex = schedule.chapters.indexOf(current.chapterId);
            if (chapterIndex < 0) {
                throw new Error("Глава отсутствует в этой книге.");
            }
            const currentPage = await readChapter(current.chapterId, lifetime);
            const images = currentPage.kind === "images";
            if (images !== schedule.images) {
                schedule.secondsPerUnit = images ? 30 : 240;
                schedule.processingSeconds = images ? 20 : 15;
                schedule.images = images;
            }
            const targets: { chapterId: string; imageIndex?: number }[] = [];
            if (images) {
                const first = Math.max(0, Math.min(currentPage.images!.length - 1, current.imageIndex ?? 0));
                const ahead = Math.min(6, Math.max(1, Math.ceil((schedule.processingSeconds * 2) / schedule.secondsPerUnit)));
                for (let index = first; index <= first + ahead && index < currentPage.images!.length; ++index) {
                    targets.push({ chapterId: current.chapterId, imageIndex: index });
                }
                if (first + ahead >= currentPage.images!.length && chapterIndex + 1 < schedule.chapters.length) {
                    targets.push({ chapterId: schedule.chapters[chapterIndex + 1]!, imageIndex: 0 });
                }
            }
            else {
                const remaining = (1 - (current.offset ?? 0)) * schedule.secondsPerUnit;
                const ahead = remaining <= schedule.processingSeconds * 1.5
                    ? Math.min(2, Math.max(1, Math.ceil((schedule.processingSeconds * 2) / schedule.secondsPerUnit))) : 0;
                for (let index = chapterIndex; index <= chapterIndex + ahead && index < schedule.chapters.length; ++index) {
                    targets.push({ chapterId: schedule.chapters[index]! });
                }
            }
            const target = targets.find(entry => !completed.has(`${entry.chapterId}:${entry.imageIndex ?? "text"}`));
            if (!target) {
                break;
            }
            schedule.target = target;
            schedule.job = new AbortController();
            const jobSignal = AbortSignal.any([lifetime, schedule.job.signal]);
            const started = performance.now();
            try {
                const page = target.chapterId === currentPage.id ? currentPage : await readChapter(target.chapterId, jobSignal);
                const translated = await translate(page, target.imageIndex, jobSignal);
                lifetime.throwIfAborted();
                if (!translated.cached) {
                    const seconds = Math.max(0.2, (performance.now() - started) / 1000);
                    schedule.processingSeconds = (schedule.processingSeconds * 0.5) + (seconds * 0.5);
                }
                completed.add(`${target.chapterId}:${target.imageIndex ?? "text"}`);
                ++processed;
                onEvent("page", translated);
            }
            catch (error) {
                if (!schedule.job.signal.aborted || lifetime.aborted) {
                    throw error;
                }
            }
            finally {
                schedule.target = undefined;
                schedule.job = undefined;
            }
        }
        return { processed, processingSeconds: schedule.processingSeconds };
    }
    finally {
        schedule.stop = undefined;
        schedule.target = undefined;
        schedule.job = undefined;
    }
}
