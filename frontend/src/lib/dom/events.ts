export function listenDocumentEvent<K extends keyof DocumentEventMap>(config: {
    name: K;
    handler(e?: DocumentEventMap[K]): void;
    runImmediately?: boolean;
}) {
    document.addEventListener(config.name, config.handler);
    const unsubscribe = () => document.removeEventListener(config.name, config.handler);
    if (config.runImmediately) config.handler();

    return { unsubscribe };
}
