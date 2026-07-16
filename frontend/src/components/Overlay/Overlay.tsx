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

import { Component } from 'react';
import { createPortal } from 'react-dom';

// Styles
import cn from 'clsx';
import css from './Overlay.scss';

export interface OverlayProps {
    isOpen: boolean;
    onChange?(isOpen: boolean): void;

    inPortal?: boolean;
    children?: ReactNode;

    className?: string;
    style?: CSSProperties;
}
export class Overlay extends Component<OverlayProps> {
    componentDidUpdate(prevProps: OverlayProps) {
        const { isOpen, onChange } = this.props;
        const wasOpen = prevProps.isOpen;

        if (isOpen !== wasOpen) {
            onChange?.(isOpen);
            document.body.style.overflow = isOpen ? 'hidden' : '';
        }
    }

    render() {
        const { isOpen, inPortal, children, className, style } = this.props;
        const content = (
            <div
                role="dialog"
                hidden={!isOpen}
                style={style}
                className={cn(css.root, inPortal && css.inPortal, isOpen && css.isVisible, className)}
            >
                <div className={css.contentOuter}>
                    <div className={css.contentInner} children={children} />
                </div>
            </div>
        );
        return inPortal ? createPortal(content, document.body) : content;
    }
}
