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

import { type RefCallback, Fragment } from 'react';
import { createPortal } from 'react-dom';
import {
    usePopperTooltip,
    type Config as PopperConfig,
    type TriggerType as PopperTriggerType,
} from 'react-popper-tooltip';

import css from './Tooltip.scss';
import cn from 'clsx';

type Placement =
    | 'auto'
    | 'auto-start'
    | 'auto-end'
    | 'top'
    | 'top-start'
    | 'top-end'
    | 'bottom'
    | 'bottom-start'
    | 'bottom-end'
    | 'right'
    | 'right-start'
    | 'right-end'
    | 'left'
    | 'left-start'
    | 'left-end';

type Content = NonNullable<ReactNode>;
type Refs = {
    tooltipRef: null | HTMLElement;
    triggerRef: null | HTMLElement;
};

export interface BaseTooltipProps {
    trigger?: PopperTriggerType | PopperTriggerType[];
    interactive?: boolean;
    placement: Placement;
    // Shorthand for popper.js offset modifier, see https://popper.js.org/docs/v2/modifiers/offset/
    offset?: [skid: number, distance: number];
    hasArrow?: boolean;

    show?: boolean;
    // Delay in hiding the tooltip (ms)
    delayHide?: PopperConfig['delayHide'];
    // Delay in showing the tooltip (ms)
    delayShow?: PopperConfig['delayShow'];
    closeOnOutsideClick?: boolean;
    onVisibleChange?(state: boolean): void;

    content: Content | ((refs: Refs) => Content);
    render(setTriggerRef: RefCallback<HTMLElement>, refs: Refs): ReactNode;

    className?: string;
    style?: CSSProperties;
}
export function BareTooltip(props: BaseTooltipProps) {
    const {
        trigger,
        placement,
        offset = [0, 16],
        interactive,
        hasArrow,

        show,
        delayShow,
        delayHide,
        onVisibleChange,
        closeOnOutsideClick,

        content,
        render,

        className,
        style,
    } = props;

    const {
        getArrowProps,
        getTooltipProps,
        setTooltipRef,
        tooltipRef,
        setTriggerRef,
        triggerRef,
        visible,
        //
    } = usePopperTooltip({
        offset,
        trigger: trigger ?? 'hover',
        placement,
        interactive,
        delayHide: interactive ? (delayHide ?? 800) : undefined,
        delayShow: delayShow ?? 0,
        visible: show ?? undefined,
        onVisibleChange,
        closeOnOutsideClick: closeOnOutsideClick ?? true,
    });

    const refs = { tooltipRef, triggerRef };
    const $children = render(setTriggerRef, refs);

    let $tooltip: ReactNode = null;
    if (visible) {
        $tooltip = createPortal(
            <div ref={setTooltipRef} {...getTooltipProps({ className, style })}>
                {typeof content === 'function' ? content(refs) : content}
                {hasArrow !== false ? <div {...getArrowProps({ className: css.arrow })} /> : null}
            </div>,
            document.body,
        );
    }

    return (
        <Fragment>
            {$children}
            {$tooltip}
        </Fragment>
    );
}

export interface TooltipProps extends Omit<BaseTooltipProps, 'hasArrow'> {
    limitedWidth?: boolean | number;
    contentHasPadding?: boolean;
    // Ghostly tooltip does not have an arrow, background and shadow
    ghostly?: boolean;
}
export function Tooltip(props: TooltipProps) {
    const { limitedWidth = true, ghostly, contentHasPadding, ...rest } = props;
    const className: string = cn(
        css.root,
        ghostly && css.ghostly,
        limitedWidth === true && css.limitedWidth,
        contentHasPadding && css.contentHasPadding,
        props.className,
    );
    const style: CSSProperties = {
        maxWidth: typeof limitedWidth === 'number' ? limitedWidth : undefined,
    };

    return <BareTooltip {...rest} hasArrow={!ghostly} style={style} className={className} />;
}
