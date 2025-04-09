import type { ReactElement } from 'react';
import { renderToString } from 'react-dom/server';
import { cleanupHtml } from '@/components/format/Html';

export function toString(nodeOrString: ReactElement | string): string {
    const html = typeof nodeOrString === 'string' ? nodeOrString : renderToString(nodeOrString);

    const div = document.createElement('div');
    div.innerHTML = cleanupHtml(html);

    return div.innerText;
}
