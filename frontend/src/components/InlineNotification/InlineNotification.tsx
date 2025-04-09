import css from './InlineNotification.scss';
import THEME from '@/styles/theme';
import cn from 'clsx';

import { ActionableNotification as BaseComponent, type ActionableNotificationProps as BaseProps } from '@carbon/react';

export type InlineNotificationProps = Omit<BaseProps, 'title' | 'actions'> & {
    stretch?: boolean;
    title?: ReactNode;
    children?: null | ReactNode;
    theme?: keyof typeof THEME;
    action?: {
        label: string;
        onClick(): void;
    };
};

export function InlineNotification(props: InlineNotificationProps) {
    const { stretch, className, kind, theme, action, children, title, ...rest } = props;

    if (!children && !title) return null;

    let kindClass: string = '';
    switch (kind) {
        case 'info':
            kindClass = css.info;
            break;

        case 'warning':
            kindClass = css.warning;
            break;

        case 'error':
            kindClass = css.error;
            break;

        case 'success':
            kindClass = css.success;
            break;

        default:
            kindClass = css.info;
    }

    const resProps = {
        ...rest,
        children,
        kind,
        lowContrast: true,
        title: title || '',
        className: cn(css.root, stretch && css.stretch, kindClass, theme && THEME[theme], className),
    } as BaseProps;

    if (action) {
        Object.assign(resProps, {
            actionButtonLabel: action.label,
            onActionButtonClick: action.onClick,
        });
    }

    return <BaseComponent {...resProps} />;
}
