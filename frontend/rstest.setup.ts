// jsdom has no ResizeObserver (no layout engine). `useSize` only needs the
// constructor to exist, not to report sizes — a no-op stub is enough.

class NoopResizeObserver implements ResizeObserver {
    observe(_target: Element, _options?: ResizeObserverOptions): void {}
    unobserve(_target: Element): void {}
    disconnect(): void {}
}

globalThis.ResizeObserver ??= NoopResizeObserver;
