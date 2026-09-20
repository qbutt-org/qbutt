import assert from "node:assert/strict";

export type Value = Buffer | number | Value[] | { [key: string]: Value };
export function encode(value: Value): Buffer {
    if (Buffer.isBuffer(value)) return Buffer.concat([Buffer.from(`${value.length}:`), value]);
    if (typeof value === "number") return Buffer.from(`i${value}e`);
    if (Array.isArray(value)) return Buffer.concat([Buffer.from("l"), ...value.map(encode), Buffer.from("e")]);
    return Buffer.concat([Buffer.from("d"), ...Object.keys(value).sort().flatMap(key =>
        [encode(Buffer.from(key)), encode(value[key]!)]), Buffer.from("e")]);
}
export function decode(packet: Buffer): { [key: string]: Value } {
    assert(packet.length <= 4096, "DHT fixture frame limit");
    let cursor = 0, values = 0;
    function read(depth = 0): Value {
        assert(depth <= 8 && ++values <= 128 && cursor < packet.length, "DHT fixture nesting limit");
        const type = packet[cursor]!;
        if (type === 100 || type === 108) {
            cursor++;
            const entries: { [key: string]: Value } = {}, items: Value[] = [];
            while (packet[cursor] !== 101) {
                if (type === 108) items.push(read(depth + 1));
                else {
                    const key = read(depth + 1); assert(Buffer.isBuffer(key));
                    entries[key.toString()] = read(depth + 1);
                }
            }
            cursor++; return type === 108 ? items : entries;
        }
        const end = packet.indexOf(type === 105 ? 101 : 58, cursor + (type === 105 ? 1 : 0));
        assert(end >= cursor, "Malformed DHT fixture field");
        const text = packet.subarray(cursor + (type === 105 ? 1 : 0), end).toString();
        assert(/^-?\d+$/.test(text));
        const number = Number(text); assert(Number.isSafeInteger(number)); cursor = end + 1;
        if (type === 105) return number;
        assert(number >= 0 && cursor + number <= packet.length);
        const result = packet.subarray(cursor, cursor + number); cursor += number; return result;
    }
    const value = read(); assert(cursor === packet.length && !Array.isArray(value)
        && !Buffer.isBuffer(value) && typeof value === "object"); return value;
}
export function compact(host: string, port: number): Buffer {
    const endpoint = Buffer.from([...host.split(".").map(Number), 0, 0]);
    endpoint.writeUInt16BE(port, 4); return endpoint;
}
