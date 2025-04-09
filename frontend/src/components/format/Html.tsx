import { createElement, type Ref, useRef, useEffect } from 'react';
import DOMPurify, { type Config } from 'dompurify';
import { mergeRefs } from '@/lib/react/props';

export type HtmlProps<T extends HTMLElement = HTMLElement> = {
    source?: null | string;

    // Both are equivalents
    container?: keyof HTMLElementTagNameMap;
    children?: string;
    fwdRef?: Ref<T extends keyof HTMLElementTagNameMap ? HTMLElementTagNameMap[T] : T>;

    process?(node: HTMLElement): void;

    additionalTags?: Array<keyof HTMLElementTagNameMap>;
    additionalAttrs?: Array<string>;

    className?: string;
    style?: CSSProperties;
};

const defaultConfig = {
    ADD_TAGS: [
        'a',
        'b',
        'code',
        'div',
        'em',
        'h1',
        'h2',
        'h3',
        'h4',
        'h5',
        'h6',
        'i',
        'img',
        'input',
        'li',
        'ol',
        'p',
        'picture',
        'pre',
        'source',
        'span',
        'strong',
        'table',
        'tbody',
        'td',
        'th',
        'thead',
        'tr',
        'ul',
    ],
    ADD_ATTR: ['class', 'height', 'id', 'media', 'name', 'rel', 'srcset', 'style', 'value', 'width'],
} satisfies Config;
function getConfig(extra: Pick<HtmlProps, 'additionalTags' | 'additionalAttrs'>): Config {
    const ADD_TAGS: Set<string> = new Set(defaultConfig.ADD_TAGS);
    if (extra.additionalTags) extra.additionalTags.forEach(x => ADD_TAGS.add(x));

    const ADD_ATTR: Set<string> = new Set(defaultConfig.ADD_ATTR);
    if (extra.additionalAttrs) extra.additionalAttrs.forEach(x => ADD_TAGS.add(x));

    return {
        ADD_TAGS: Array.from(ADD_TAGS),
        ADD_ATTR: Array.from(ADD_ATTR),
    };
}

export function cleanupHtml(input: Maybe<string>, config?: Config): string {
    return input == null ? '' : DOMPurify.sanitize(input, config);
}

export function Html(props: HtmlProps) {
    const { source, children, container, additionalTags, additionalAttrs, className, style, fwdRef, process } = props;

    const config = getConfig({ additionalTags, additionalAttrs });
    const clean = cleanupHtml(source || children, config);

    const ref = useRef<HTMLElement>(null);
    useEffect(() => {
        const el = ref.current;
        if (process && el) process(el);
    }, [process]);

    return createElement(container || 'span', {
        // biome-ignore lint/security/noDangerouslySetInnerHtml: 'Insane' sanitize the markup
        dangerouslySetInnerHTML: { __html: clean },
        ref: mergeRefs(ref, fwdRef),
        className,
        style,
    });
}
