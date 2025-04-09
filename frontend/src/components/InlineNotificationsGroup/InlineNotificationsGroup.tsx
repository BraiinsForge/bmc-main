import type { ReactElement } from 'react';

import cn from 'clsx';
import THEME from '@/styles/theme';
import css from './InlineNotificationsGroup.scss';

import { InlineNotification, type InlineNotificationProps } from '../InlineNotification';

export type ErrorItem = undefined | null | false | string | InlineNotificationProps;
export type InlineNotificationsGroupProps = {
    items?: Maybe<Array<ErrorItem>>;
    renderedItems?: Array<ReactElement<any, typeof InlineNotification>>;

    stretch?: boolean;
    theme?: keyof typeof THEME;
    kind?: InlineNotificationProps['kind'];

    className?: string;
    style?: CSSProperties;
};

export function InlineNotificationsGroup(props: InlineNotificationsGroupProps) {
    const { items, renderedItems, className, kind, theme, stretch, ...rest } = props;
    const children: ReactNode[] = [];

    items?.forEach((d, i) => {
        if (d == null || d === false) return;

        const itemProps: InlineNotificationProps = {
            stretch,
            kind: kind || 'info',
            hideCloseButton: true,
        };
        if (typeof d === 'string') itemProps.children = d;
        else Object.assign(itemProps, d);

        children.push(<InlineNotification key={i} {...itemProps} />);
    });
    if (renderedItems?.length) children.push(...renderedItems);

    if (!children.length) return null;

    return (
        <div
            {...rest}
            role="alert"
            className={cn(css.root, theme && THEME[theme], stretch && css.stretch, className)}
            children={children}
        />
    );
}
