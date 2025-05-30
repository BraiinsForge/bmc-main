import { abort } from '@/lib/dom';
import { create, type Message } from '@bufbuild/protobuf';

import type { Timezone } from './pb';

export function renderTimezone(tz: Maybe<Timezone>): string {
    if (!tz) return 'N/A';
    return `UTC${tz.offset} (${tz.label})`;
}

// Utilities index
export * from './pb';
export * from './rpc';
export * from './forms';

export {
    abort,
    create,
    // Types
    type Message,
};
