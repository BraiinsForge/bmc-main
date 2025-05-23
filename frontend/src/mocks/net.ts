import { type LengthRange, getLength } from './generics';
import { lorem, word } from './string';
import { number } from './number';
import { randomItem, arrayOf } from './collection';

export function ipv4(): IPv4 {
    return `${number(0, 255)}.${number(0, 255)}.${number(0, 255)}.${number(0, 255)}`;
}
export function ipv6(): IPv6 {
    const n = () => (Math.random() <= 0.5 ? 0 : number(0, 65535)).toString(16);
    const v: IPv6 = `${n()}:${n()}:${n()}:${n()}:${n()}:${n()}:${n()}:${n()}`;
    return (
        v
            // collapse zeros
            .replace(/:0*:/g, '::')
            // collapse repetitive colons
            .replace(/:{2,}/g, '::') as IPv6
    );
}
export function ip(v: 4 | 6 = randomItem([4, 6])) {
    return v === 4 ? ipv4() : ipv6();
}

export function mac() {
    return 'XX:XX:XX:XX:XX:XX'.replace(/X/g, () => '0123456789ABCDEF'.charAt(Math.floor(Math.random() * 16)));
}

interface UrlConfig {
    lengthHostname?: number;
    lengthTld?: number;
    static?: Partial<URL>;
    pathNameCount?: LengthRange;
    pathNameLength?: LengthRange;
}
export const url = (conf?: UrlConfig): URL => {
    const hostname = word(conf?.lengthHostname || 5);
    const tld = lorem.generateWords(1).slice(0, conf?.lengthTld || 3);
    const pathName: string = arrayOf(
        // Number of path segments
        conf?.pathNameCount ?? 3,
        // Length of each segment
        word.bind(null, conf?.pathNameLength ?? [6, 15]),
    ).join('/');

    const res = new URL(`https://${hostname}.${tld}/${pathName}`);
    if (conf?.static) Object.assign(res, conf.static);

    return res;
};
export function urlStratum(): URL {
    return url({
        lengthHostname: 20,
        static: {
            protocol: 'stratum+tcp',
            port: String(number(3333, 6666, false)),
            hostname: lorem.generateWords(2).split(' ').join('.'),
        },
    });
}

export const email = (): string => {
    const a = lorem.generateWords(1);
    const b = lorem.generateWords(1);
    const c = lorem.generateWords(1).slice(0, 3);
    return `${a}@${b}.${c}`;
};

export function file(content: string, name: string, type?: string): File {
    return new File([content], name, { type });
}

export function fileName(extensions?: string[]): string {
    const $exts = extensions ?? ['jpeg', 'png', 'gif', 'svg', 'pdf', 'txt', 'doc', 'docx', 'xls', 'xlsx'];
    return `${word(5)}.${randomItem($exts)}`;
}

export function hostname(wordsCount: LengthRange, domain: string = 'local'): string {
    const count = getLength(wordsCount);
    const name = lorem.generateWords(count).replace(/\s/g, '-').toLowerCase();
    return [name, domain].filter(Boolean).join('.');
}
