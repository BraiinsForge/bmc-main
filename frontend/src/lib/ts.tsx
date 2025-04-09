import { isEqual } from 'es-toolkit';

/**
 * Used for exhaustive switch checking
 * @see https://stackoverflow.com/questions/39419170/how-do-i-check-that-a-switch-block-is-exhaustive-in-typescript
 */
export function assertUnreachable(x: never, label?: string): never {
    const message = label ? `Unexected ${label} value has been reached!` : 'Unexected value has been reached!';
    throw new Error(`${message} - ${x}`);
}
export function assertUnreachableEnumUnspecified(x: 0, label?: string): never {
    const message = label ? `Unexected ${label} value has been reached!` : 'Unexected value has been reached!';
    throw new Error(`${message} - ${x}`);
}
export function assertUnreachableNil(x: undefined | null, label?: string): never {
    const message = label ? `Unexected ${label} value has been reached!` : 'Unexected value has been reached!';
    throw new Error(`${message} - ${x}`);
}

interface StorageWrapperConfig<V> {
    key: string;
    defaultValue: V;

    parse(value: string): V;
    stringify(value: V): string;
    validate?(value: V | unknown): boolean;
}

export class StorageWrapper<V> {
    #storage: Storage;
    #config: StorageWrapperConfig<V>;
    constructor(storage: Storage, config: StorageWrapperConfig<V>) {
        this.#storage = storage;
        this.#config = config;
    }

    #catch<T>(fn: () => T, fallback: T): T {
        try {
            return fn();
        } catch (e: unknown) {
            // Exception is thrown when:
            //  - user disabled the storage (some browsers do this in incognito mode)
            //  - storage quota is exceeded
            return fallback;
        }
    }

    public save = (value: V): void => {
        this.#catch(() => {
            const { key, stringify } = this.#config;
            this.#storage.setItem(key, stringify(value));
        }, null);
    };
    public clear = (): void => {
        this.#catch(() => {
            this.#storage.removeItem(this.#config.key);
        }, null);
    };

    public load = (): V => {
        const { key, parse, defaultValue, validate } = this.#config;
        return this.#catch(() => {
            const raw = this.#storage.getItem(key);
            const parsed = raw ? parse(raw) : defaultValue;
            return validate && validate(parsed) !== true ? defaultValue : parsed;
        }, defaultValue);
    };
    public listen = (callback: (value: V) => void): { dispose(): void } => {
        const fn = (e: StorageEvent) => {
            // Skip if the event is not related to the storage area we manage
            if (e.key !== this.#config.key || e.storageArea !== this.#storage) return;

            // Skip if the specific managed value has not changed
            if (isEqual(e.oldValue, e.newValue)) return;

            callback(this.load());
        };
        window.addEventListener('storage', fn);

        return { dispose: () => window.removeEventListener('storage', fn) };
    };
}

// Convert numbers to strings
export type StringifyNumbers<Obj extends Record<string, unknown>> = {
    [K in keyof Required<Obj>]: NonNullable<Obj[K]> extends number | bigint
        ? // Numbers are converted to strings
          null | string
        : // Other types are left as is
          NonNullable<Obj[K]>;
};
