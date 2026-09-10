// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import { afterEach } from '@rstest/core';
import { cleanup } from '@testing-library/react/pure';

import { toast } from '@/lib/toast';

// The `/pure` entry registers no cleanup of its own, and a tree left mounted
// after a file's last test can still commit: floating-ui resolves `computePosition()`
// late and React flushes the passive effects through a real `setImmediate`, which lands
// after rstest 0.11.12 has torn jsdom down and so fails the run.
//
// sonner 2.0.8 replays active toasts into a newly mounted Toaster: its `subscribe`
// now calls `getActiveToasts().forEach(subscriber)`. `cleanup()` unmounts the Toaster
// but leaves the queue, so the next test would render the whole backlog.
afterEach(() => {
    cleanup();
    toast.dismiss(null);
});

// jsdom has no ResizeObserver (no layout engine). `useSize` only needs the
// constructor to exist, not to report sizes — a no-op stub is enough.

class NoopResizeObserver implements ResizeObserver {
    observe(_target: Element, _options?: ResizeObserverOptions): void {}
    unobserve(_target: Element): void {}
    disconnect(): void {}
}

globalThis.ResizeObserver ??= NoopResizeObserver;

// Nor a scrollIntoView, which Carbon's Dropdown calls on its highlighted option.
Element.prototype.scrollIntoView ??= function scrollIntoView(): void {};
