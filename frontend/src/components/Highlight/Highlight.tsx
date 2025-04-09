import { Prism } from '@/lib/prism';
import { useIntl } from 'react-intl';
import { CopyButton } from '@/components/CopyButton';

import cn from 'clsx';
import css from './Highlight.scss';

export type HighlightProps = {
    src: string;
    lang?: string;

    diff?: boolean;
    copy?: boolean;

    className?: string;
};

export function Highlight({ src, lang, copy, diff, className }: HighlightProps) {
    const intl = useIntl();
    const language = lang ?? 'plain';

    return (
        <div className={cn(css.root, className)}>
            {copy === true && (
                <div className={css.copyButtonWrapper}>
                    <CopyButton
                        align="left"
                        kind="transparent"
                        title={intl.formatMessage({ defaultMessage: 'Copy to clipboard' })}
                        feedback={intl.formatMessage({ defaultMessage: 'Copied!' })}
                        value={src}
                    />
                </div>
            )}
            <pre
                className={cn(
                    'prism-code',
                    diff ? `language-diff-${language}` : `language-${language}`,
                    diff && 'diff-highlight',
                )}
            >
                <code
                    // biome-ignore lint/security/noDangerouslySetInnerHtml: Prism sanitizes the markup
                    dangerouslySetInnerHTML={{
                        __html: Prism.highlight(
                            src,
                            Prism.languages[diff ? 'diff' : language],
                            diff ? `diff-${language}` : language,
                        ),
                    }}
                />
            </pre>
        </div>
    );
}
