// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import { forwardRef, type Ref, type AnchorHTMLAttributes, type ComponentType, type SyntheticEvent } from 'react';

import { Button as BaseComponent, InlineLoading } from '@carbon/react';
import type { ButtonBaseProps } from '@carbon/react/es/components/Button/Button';
import { ARIA } from '@/components/constants';
import { Tooltip, type TooltipProps } from '@/components/Tooltip';

// Styles
import cn from 'clsx';
import css from './Button.scss';

export type ButtonProps = Omit<ButtonBaseProps, 'href' | 'target' | 'rel' | 'kind' | 'onClick'> & {
    id: string;

    kind?: ButtonBaseProps['kind'];
    icon?: ComponentType;
    onClick?(e: SyntheticEvent): void;
    blurOnClick?: boolean;

    children?: ReactNode;
    title?: string;

    href?: AnchorHTMLAttributes<HTMLAnchorElement>['href'];
    target?: AnchorHTMLAttributes<HTMLAnchorElement>['target'];
    rel?: AnchorHTMLAttributes<HTMLAnchorElement>['rel'];

    loading?: null | boolean | string;
};
function ButtonComponent(props: ButtonProps & { innerRef: Ref<HTMLButtonElement> }) {
    const {
        icon,
        hasIconOnly,
        children: childrenRaw,
        title,
        onClick,
        blurOnClick = true,
        className,
        innerRef,
        loading,
        kind = 'primary',
        tooltipPosition,
        tooltipAlignment,
        // Pipe the rest through
        ...rest
    } = props;

    // Default children to link href if not provided
    let children = childrenRaw;
    if (childrenRaw == null && rest.href) children = rest.href;

    let renderIcon: undefined | ButtonBaseProps['renderIcon'] = props.renderIcon;
    if (icon) renderIcon = icon as ComponentType;

    const $hasIconOnly: boolean = hasIconOnly || Boolean(renderIcon && !children);
    const targetProps: ButtonBaseProps = {
        kind,
        title: undefined,

        // Icon
        renderIcon,

        // Pass-through the rest
        ...rest,
    };

    /**
     * CSD button has an idiotic props interface in that `tooltipPosition`
     * gets passed down to DOM when `hasIconOnly` is "present" (not just `true`)
     *
     * @see https://github.com/carbon-design-system/carbon/issues/16501
     */
    if ($hasIconOnly) {
        Object.assign(targetProps, {
            children: null,
            hasIconOnly: true,
            iconDescription: title,
            tooltipPosition: tooltipPosition ?? 'top',
            tooltipAlignment: tooltipAlignment ?? 'center',
        });
    }

    if (onClick) Object.assign(targetProps, ARIA.button(onClick, blurOnClick));
    if (!targetProps.iconDescription) targetProps.iconDescription = '';
    if (loading) {
        targetProps.disabled = true;
        targetProps['aria-busy'] = true;
    }

    // Except for classname, where we need to intercept
    targetProps.className = cn(
        className,
        css.root,
        props.disabled && css.disabled,
        props.size && (css as any)[props.size],
        props.kind && (css as any)[props.kind],
        $hasIconOnly ? css.iconOnly : null,
    );

    // Carbon renders the icon in button's big padding that is always there, but that is shit…
    // So we've re-done the spacing but need the extra `.buttonText` div.
    // Also it serves secondary pupose in vertically centering it's content
    // should someone want to render (for example) icon in the text instead of the standard way.
    return (
        <BaseComponent {...targetProps} ref={innerRef}>
            {!!loading && (
                <div key="loading" className={css.loading} role="status">
                    <InlineLoading
                        status="active"
                        iconDescription={typeof loading === 'string' ? loading : undefined}
                    />
                </div>
            )}
            {children != null && <div key="textWrapper" className={css.buttonText} children={children} />}
        </BaseComponent>
    );
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>((p, r) => {
    return <ButtonComponent {...p} innerRef={r} />;
});

export interface ButtonWithTooltipProps extends Omit<ButtonProps, 'tooltipPosition' | 'tooltipAlignment' | 'title'> {
    tooltipProps: Omit<TooltipProps, 'render'>;
}
export function ButtonWithTooltip(props: ButtonWithTooltipProps) {
    const { tooltipProps, ...rest } = props;
    return <Tooltip {...tooltipProps} render={ref => <Button {...rest} ref={ref} title="" />} />;
}
