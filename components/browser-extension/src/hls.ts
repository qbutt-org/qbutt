export type HlsVariant = {
  audioGroup?: string;
  bandwidth: number;
  codecs?: string;
  frameRate?: number;
  height?: number;
  url: string;
  width?: number;
};

export type HlsRendition = {
  default: boolean;
  forced: boolean;
  groupId: string;
  language?: string;
  name: string;
  type: "audio" | "subtitles";
  url?: string;
};

export type HlsMaster = {
  renditions: HlsRendition[];
  variants: HlsVariant[];
};

function parseAttributes(line: string): Map<string, string> {
  const attributes = new Map<string, string>();
  const body = line.slice(line.indexOf(":") + 1);
  const matcher = /([A-Z0-9-]+)=((?:"[^"]*")|[^,]*)/gi;
  for (const match of body.matchAll(matcher)) {
    const key = match[1];
    const rawValue = match[2];
    if (!key || rawValue === undefined) continue;
    attributes.set(key.toUpperCase(), rawValue.replace(/^"|"$/g, ""));
  }
  return attributes;
}

function yes(value: string | undefined): boolean {
  return value?.toUpperCase() === "YES";
}

function absoluteUrl(value: string, baseUrl: string): string {
  return new URL(value, baseUrl).href;
}

export function parseHlsMaster(body: string, baseUrl: string): HlsMaster {
  const lines = body.split(/\r?\n/).map((line) => line.trim());
  const variants: HlsVariant[] = [];
  const renditions: HlsRendition[] = [];

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (!line) continue;

    if (line.startsWith("#EXT-X-MEDIA:")) {
      const attributes = parseAttributes(line);
      const type = attributes.get("TYPE")?.toLowerCase();
      if (type !== "audio" && type !== "subtitles") continue;
      const groupId = attributes.get("GROUP-ID");
      const name = attributes.get("NAME");
      if (!groupId || !name) continue;
      const url = attributes.get("URI");
      const language = attributes.get("LANGUAGE");
      renditions.push({
        default: yes(attributes.get("DEFAULT")),
        forced: yes(attributes.get("FORCED")),
        groupId,
        name,
        type,
        ...(language ? { language } : {}),
        ...(url ? { url: absoluteUrl(url, baseUrl) } : {}),
      });
      continue;
    }

    if (!line.startsWith("#EXT-X-STREAM-INF:")) continue;
    const attributes = parseAttributes(line);
    const relativeUrl = lines.slice(index + 1).find((candidate) => candidate && !candidate.startsWith("#"));
    if (!relativeUrl) continue;
    const resolution = attributes.get("RESOLUTION")?.match(/^(\d+)x(\d+)$/i);
    const audioGroup = attributes.get("AUDIO");
    const codecs = attributes.get("CODECS");
    const frameRate = attributes.get("FRAME-RATE");
    variants.push({
      bandwidth: Number(attributes.get("AVERAGE-BANDWIDTH") ?? attributes.get("BANDWIDTH") ?? 0),
      url: absoluteUrl(relativeUrl, baseUrl),
      ...(audioGroup ? { audioGroup } : {}),
      ...(codecs ? { codecs } : {}),
      ...(frameRate ? { frameRate: Number(frameRate) } : {}),
      ...(resolution ? { width: Number(resolution[1]), height: Number(resolution[2]) } : {}),
    });
  }

  return { renditions, variants };
}

export function bestHlsVariant(variants: readonly HlsVariant[]): HlsVariant | undefined {
  return [...variants].sort((left, right) => {
    const leftPixels = (left.width ?? 0) * (left.height ?? 0);
    const rightPixels = (right.width ?? 0) * (right.height ?? 0);
    return rightPixels - leftPixels || right.bandwidth - left.bandwidth;
  })[0];
}

export function hlsIsEncrypted(body: string): boolean {
  return /#EXT-X-KEY:[^\n]*METHOD=(?!NONE(?:,|$))/i.test(body);
}
