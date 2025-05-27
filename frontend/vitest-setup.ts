import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';
import { ReadableStream } from 'node:stream/web';
import '@testing-library/jest-dom/vitest';

Object.assign(global, {
    userId: 'DEADBEEF',
    ReadableStream,
    translate: (str: string) => str,
    translateL: (str: string) => ({ defaultMessage: str, id: str }),
    translateP: (_: string, str: string) => str,
    translatePL: (_: string, str: string) => ({ defaultMessage: str, id: str }),
});

afterEach(cleanup);
