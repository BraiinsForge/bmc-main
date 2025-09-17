import { useCallback, type Ref, type UIEvent, type UIEventHandler } from 'react';
import { useNavigate } from 'react-router';
import { blockEvent } from '@/lib/react';

import type { CarbonIconType } from '@carbon/react/icons';

import cn from 'clsx';
import css from './Link.scss';

export interface LinkProps {
    href?: null | string;
    target?: string | '_blank';
    external?: boolean;

    onClick?: null | UIEventHandler<HTMLElement>;
    replace?: boolean;
    disabled?: boolean;

    children?: ReactNode;
    icon?: CarbonIconType;
    iconPlacement?: 'start' | 'end';

    className?: string;
    style?: CSSProperties;
    ref?: Ref<null | HTMLAnchorElement>;
}

export function Link(props: LinkProps) {
    const {
        href,
        target,
        replace,
        external,
        disabled,
        onClick,
        // Icon
        icon: Icon,
        iconPlacement,
        // DOM & rest,
        className,
        children,
        ref,
        ...rest
    } = props;
    const navigate = useNavigate();

    const click = useCallback(
        (e: UIEvent<HTMLElement>) => {
            if (disabled) return;

            // prevent default link behav. as we'll handle it ourselves
            blockEvent(e);

            if (onClick) onClick(e);

            // Uses local history navigation when possible
            if (href) {
                const $target = target ?? (external ? '_blank' : undefined);
                if ($target) window.open(href, $target);
                else navigate(href, { replace: !!replace });
            }
        },
        [disabled, href, replace, navigate, external, onClick, target],
    );
    if (!href && !onClick) return null;

    let $icon: ReactNode;
    if (Icon) $icon = <Icon key="icon" size="1em" />;

    const $text = <span key="txt" children={children} />;
    const p = {
        ...rest,
        children: (
            <span children={iconPlacement === 'start' ? [$icon, $text] : [$text, $icon]} className={css.spaced} />
        ),
        href: href || '#',
        ref,
        onClick: click,
        className: cn('cds--link', css.root, className),
        role: !href && onClick ? 'button' : undefined,
    };
    if (external) Object.assign(p, { target: '_blank', rel: 'noopener noreferrer' });

    return <a {...p} />;
}
