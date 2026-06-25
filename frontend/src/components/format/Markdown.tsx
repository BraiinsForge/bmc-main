import MarkdownIt from 'markdown-it';
import { Html, type HtmlProps } from './Html';

// Shared instance — Markdown rendering is stateless, no need to rebuild per call.
// `html: false` drops raw HTML in the source; `Html` then sanitizes the output.
const md = MarkdownIt('default', {
    html: false,
    breaks: false,
    linkify: true,
    typographer: false,
});

export type MarkdownProps = Omit<HtmlProps, 'source' | 'children'> & {
    /** Markdown source; rendered to HTML, then sanitized by `Html`. */
    source?: null | string;
};

export function Markdown({ source, container = 'div', ...rest }: MarkdownProps) {
    return <Html container={container} source={source == null ? '' : md.render(source)} {...rest} />;
}
