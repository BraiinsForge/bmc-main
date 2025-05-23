export * from './tz';

type GetTimestamp = (offset?: null | number, date?: Date | number) => number;
export const getTimestampMs: GetTimestamp = (offset, time): number => {
    const d = time ? new Date(time).getTime() : Date.now();
    const o = offset ?? 0;
    return Math.floor(d + o);
};
export const getTimestamp: GetTimestamp = (offset, time): number => {
    const o = (offset ?? 0) * 1e3; // Convert offset to milliseconds
    return Math.floor(getTimestampMs(o, time) / 1e3);
};
