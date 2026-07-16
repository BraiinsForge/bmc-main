// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
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

import { Component, createRef } from 'react';
import { createPortal } from 'react-dom';

import { ComposedModal, type ComposedModalProps, ModalHeader, ModalBody, ModalFooter } from '@carbon/react';
import { PortalContext, type PortalContextValue } from '../PortalContext';
import { Loading } from '../Loading';

import cn from 'clsx';
import css from './ModalCustom.scss';

export interface CustomModalProps extends Omit<ComposedModalProps, 'title' | 'children' | 'open'> {
    // Header labeling
    title?: ReactNode;
    label?: string;

    // Content
    children: ReactNode;
    footer?: ReactNode;

    open: boolean;
    isLoading?: boolean;
    hideHeader?: boolean;
    // The `isInnerModal` prop should be set
    // if the modal opens inside another modal.
    // It prevents page scrolling after inner modal is closed
    // and the "root" modal is still open
    isInnerModal?: boolean;

    onClose?(): void;
    onSubmit?(): void;

    // Required because it's used for testing
    // and finding modal any other way is flaky
    id: string;
    portal?: boolean;
    cancelBodyOverflowShadow?: boolean;

    bodyClassName?: string;
    className?: string;
    style?: CSSProperties;
}
type State = {
    clickedInside: boolean;
    clickedOutside: boolean;
};
const getInitialState = (): State => ({ clickedInside: false, clickedOutside: false });

export class ModalCustom extends Component<CustomModalProps> {
    static defaultProps = { portal: true };
    readonly state = getInitialState();

    static contextType = PortalContext;
    declare context: PortalContextValue;

    #root: HTMLElement;
    #mount: HTMLElement;

    constructor(props: CustomModalProps) {
        super(props);

        const envBaseElement = document.body;

        // `carbon-components-react` don't use portal which causes z-index & overflow based issues.
        // Our mitigation is to inject portal API & allow disabling if such thing would be desired.
        const existing = envBaseElement.querySelector<HTMLElement>('div[data-cy="modals-root"]');

        // If we already have a mount point, we'll just store the refference to it here
        if (existing) this.#root = existing;
        // Otherwise we'll create a new one and and append it to the detected root container
        else {
            this.#root = document.createElement('div');
            this.#root.setAttribute('data-cy', 'modals-root');
            envBaseElement.appendChild(this.#root);
        }

        this.#mount = document.createElement('div');
        this.#mount.setAttribute('data-modal-mount', props.id);
    }
    componentDidMount() {
        // Replace the click handler so that we can create more sensible closing UX
        if (this.#refRoot.current?.handleClick) this.#refRoot.current.handleClick = this.#preventClose;
        this.#root.appendChild(this.#mount);

        setTimeout(this.#maybeMoveFocusInside, 30);
    }
    componentWillUnmount() {
        this.#root.removeChild(this.#mount);
    }
    componentDidUpdate(prevProps: CustomModalProps) {
        if (this.props.isInnerModal && prevProps.open) document.body.classList.add('cds--body--with-modal-open');
        if (this.props.open && !prevProps.open) setTimeout(this.#maybeMoveFocusInside, 30);
    }

    #maybeMoveFocusInside = () => {
        const { open, selectorPrimaryFocus } = this.props;
        const body = this.#refBody.current;

        // Nothing to do if modal is not open
        // or we don't have a body ref (yet)
        if (!open || !body) return;

        // Nothing to do if modal has no primary focus
        const focusedElement = document.activeElement;

        // Focus is already inside the modal body
        if (body.contains(focusedElement)) return;

        const focusable = body.querySelector<HTMLElement>(selectorPrimaryFocus ?? 'input, button, textarea, select');
        try {
            focusable?.focus();
        } catch (error) {
            console.groupCollapsed(
                '%c<ModalCustom /> %cFailed to move focus into the modal body',
                'color: goldenrod;',
                'color: unset;',
            );
            console.log({ focusable, error });
            console.groupEnd();
        }
    };

    #refRoot = createRef<any>();
    #refBody = createRef<null | HTMLDivElement>();
    #preventClose = (): void => {
        const { onClose } = this.props;
        const { clickedInside, clickedOutside } = this.state;

        if (!clickedInside && clickedOutside && onClose) onClose();
        this.setState(getInitialState);
    };

    #commonClickProps = {
        onMouseDownCapture: () => this.setState({ clickedInside: true }),
        onMouseUpCapture: () => this.setState({ clickedInside: true }),
    };
    #handleClose = () => {
        this.props.onClose?.();
        return false;
    };
    #handleMouseDown = () => this.setState({ clickedOutside: true });

    render() {
        const {
            id,

            // Header labeling
            title,
            label,

            // Content
            children,
            footer,

            // Handlers
            onSubmit,
            onClose,

            // State
            isLoading,
            open,

            // Behaviour
            portal,
            hideHeader,

            // Styling
            isInnerModal,
            bodyClassName,
            cancelBodyOverflowShadow,
            selectorPrimaryFocus,
            ...rest
        } = this.props;

        const m = (
            <ComposedModal
                {...rest}
                id={id}
                data-cy={id}
                ref={this.#refRoot}
                open={open}
                onSubmit={onSubmit}
                onClose={onClose ? this.#handleClose : undefined}
                onMouseDown={this.#handleMouseDown}
                selectorPrimaryFocus={selectorPrimaryFocus}
                className={cn(rest.className, this.context.disablePortal && css.inlineModal)}
            >
                {!hideHeader && (title != null || typeof onClose === 'function') && (
                    <ModalHeader
                        className={cn(!onClose && css.hideModalCloseButton)}
                        {...this.#commonClickProps}
                        title={title}
                        label={label}
                    />
                )}
                <ModalBody
                    {...this.#commonClickProps}
                    className={cn(css.body, cancelBodyOverflowShadow && css.cancelBodyOverflowShadow, bodyClassName)}
                    children={children}
                    ref={this.#refBody}
                />

                {footer && <ModalFooter {...this.#commonClickProps} className={css.footer} children={footer} />}
                <Loading active={isLoading} cover />
            </ComposedModal>
        );

        return portal && !this.context.disablePortal ? createPortal(m, this.#mount) : m;
    }
}
