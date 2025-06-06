import { useState, useEffect, useMemo } from 'react';
import { assertUnreachable } from '@/lib/ts';

/**
 * Single attempt to ping an image resource.
 *
 * Temporary image node is created and appended into the DOM
 * for the attempt and removed when the result is obtained.
 *
 * @example
 * const url = new URL(document.baseURI);
 * url.pathname = `${pingUrl.pathname}/ping.png`.replaceAll('//', '/');
 * url.searchParams.set('t', Date.now().toString()); // Cache busting
 * const serverStatus = await ping(url.href);
 */
export function pingImage(imageURL: string): Promise<boolean> {
    return new Promise(resolve => {
        const d = document;

        // Create a new image
        const img = d.body.appendChild(d.createElement('img'));
        img.style.display = 'none';

        // Handle the result
        const handleDone = (result: boolean) => {
            return () => {
                try {
                    img.parentElement?.removeChild(img);
                } catch (_) {
                    // Noop
                }
                resolve(result);
            };
        };
        img.onload = handleDone(true);
        img.onerror = handleDone(false);

        // Start the request
        const url = new URL(imageURL, window.location.origin);
        url.searchParams.set('cacheBusting', String(Date.now()));

        img.src = url.href;
    });
}

export function pingXhr(targetUrl: string, timeout: number = 5e3): Promise<boolean> {
    return new Promise(resolve => {
        const xhr = new XMLHttpRequest();

        // Cache busting
        const url = new URL(targetUrl, window.location.origin);
        url.searchParams.set('cacheBusting', String(Date.now()));
        xhr.open('GET', url, true);

        xhr.timeout = timeout;
        xhr.onload = () => {
            console.log('xhr', xhr.status);
            resolve(xhr.status === 200);
        };
        xhr.onerror = () => resolve(false);
        xhr.ontimeout = () => resolve(false);

        xhr.send();
    });
}

export type PingStatus = null | boolean;
export type PingCallback = (isOnline: null | boolean, wasOnline: PingStatus) => void;
export interface PingOptions {
    url: string;
    method: 'img' | 'xhr';
    interval: number;
    timeout?: number;

    // Called on every response
    onPong?: PingCallback;
    // Called only when the status changes
    onChange?: PingCallback;
}

/**
 * Instance repeatedly pings the given image resource
 * and allows to stop / restart the process.
 *
 * Consumer is notified through a callback.
 *
 * Sentinel DOM image node is created for each attempt
 * and removed when the result is obtained.
 */
export class Ping {
    #options: PingOptions;
    constructor(options: PingOptions) {
        this.#options = options;
    }

    #lastStatus: PingStatus = null;
    #timerID: number = Number.NaN;

    #ping = async () => {
        const { url, onPong, onChange, method, timeout } = this.#options;

        let status: null | boolean = null;
        switch (method) {
            case 'img':
                status = await pingImage(url);
                break;

            case 'xhr':
                status = await pingXhr(url, timeout);
                break;

            default:
                assertUnreachable(method, 'Ping method');
        }

        onPong?.(status, this.#lastStatus);
        if (status !== this.#lastStatus) onChange?.(status, this.#lastStatus);

        this.#lastStatus = status;
        return status;
    };
    #go = () => {
        const { interval } = this.#options;

        // Ensure single timer is running
        this.stop();
        this.#isActive = true;

        this.#ping().then(() => {
            // There could be a race condition where we get stopped
            // between the process start and status resolution.
            if (!this.#isActive) return;
            this.#timerID = window.setTimeout(this.#go, interval);
        });
    };

    public start = (): Ping => {
        this.#go();
        return this;
    };
    public stop = (): Ping => {
        window.clearTimeout(this.#timerID);
        this.#isActive = false;
        return this;
    };

    #isActive: boolean = false;
    public get isActive(): boolean {
        return this.#isActive;
    }
}

interface UsePingOptions extends Omit<PingOptions, 'onChange'> {
    isActive?: boolean;
}
interface UsePingState {
    wasOnline: null | boolean;
    isOnline: null | boolean;
}

/**
 * React hook version of the Ping class.
 *
 * Possible states are:
 *  - null: no result yet
 *  - boolean: success / failure respectively
 */
export function usePing(opts: UsePingOptions): UsePingState {
    const { isActive, url, interval, onPong, timeout, method } = opts;
    const [state, setState] = useState<UsePingState>({ wasOnline: null, isOnline: null });
    const conf = useMemo(() => ({ url, interval, onPong }), [url, interval, onPong]);
    const CLS = useMemo(() => {
        return new Ping({
            ...conf,
            timeout,
            method,
            onChange: (isOnline, wasOnline) => setState({ isOnline, wasOnline }),
        });
    }, [conf, timeout, method]);

    useEffect(() => {
        if (isActive && !CLS.isActive) CLS.start();
        if (!isActive && CLS.isActive) CLS.stop();
        return () => {
            CLS.stop();
        };
    }, [isActive, CLS.isActive, CLS.start, CLS.stop]);

    return state;
}
