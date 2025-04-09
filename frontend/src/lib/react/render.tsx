import type { ReactElement } from 'react';
import { renderToString } from 'react-dom/server';
import { Html, cleanupHtml } from '@/components/format/Html';

export function toString(nodeOrString: ReactElement | string): string {
    const html = typeof nodeOrString === 'string' ? nodeOrString : renderToString(nodeOrString);

    const div = document.createElement('div');
    div.innerHTML = cleanupHtml(html);

    return div.innerText;
}

export function htmlify(message: string): ReactElement {
    return (
        <Html
            container="span"
            source={message
                .replace(/(https?:\/\/\S*)/g, match => {
                    let href = match;
                    let sufix = '';

                    if (match.endsWith('.')) {
                        href = match.slice(0, -1);
                        sufix = '.';
                    }

                    return `<a href="${href}" target="_blank" rel="noreferrer noopener">${href}</a>${sufix}`;
                })
                .replace('\n', '<br />')}
        />
    );
}
