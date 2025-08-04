import * as pb from '@/proto';
import { timestamp } from './time';
import { randomItem } from './collection';
import type { LengthRange } from './generics';

export const proto = {
    randomEnumItem<K extends string, V extends number>(enm: Record<K, V>): V {
        const input = Object.values(enm).filter(x => typeof x === 'number' && x > 0) as V[];
        return randomItem<V>(input);
    },
    timestamp(offset: LengthRange = 0): pb.Timestamp {
        return pb.create(pb.TimestampSchema, { seconds: BigInt(timestamp(offset)) });
    },
};
