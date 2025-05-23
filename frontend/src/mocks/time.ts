import type { Timestamp } from '@bufbuild/protobuf/wkt';
import { type LengthRange, getLength } from './generics';
import { getTimestamp } from '@/lib/time';

const SECONDS = {
    second: 1,
    minute: 60,
    hour: 3_600,
    day: 86_400,
    year: 31_556_926,
};
const ts = (offset: LengthRange = 0): number => getTimestamp(getLength(offset));
export const timestamp = Object.assign(ts, {
    minutes: (offset: LengthRange) => timestamp(getLength(offset) * SECONDS.minute),
    hours: (offset: LengthRange) => timestamp(getLength(offset) * SECONDS.hour),
    days: (offset: LengthRange) => timestamp(getLength(offset) * SECONDS.day),
    months: (offset: LengthRange) => timestamp(getLength(offset) * SECONDS.day * 31),
});

function protoTs(offset: LengthRange = 0): Timestamp {
    const seconds = BigInt(getTimestamp(getLength(offset)));
    return { $typeName: 'google.protobuf.Timestamp', seconds, nanos: 0 };
}
export const protoTimestamp = Object.assign(protoTs, {
    minutes: (offset: LengthRange) => protoTimestamp(getLength(offset) * SECONDS.minute),
    hours: (offset: LengthRange) => protoTimestamp(getLength(offset) * SECONDS.hour),
    days: (offset: LengthRange) => protoTimestamp(getLength(offset) * SECONDS.day),
    months: (offset: LengthRange) => protoTimestamp(getLength(offset) * SECONDS.day * 31),
});
