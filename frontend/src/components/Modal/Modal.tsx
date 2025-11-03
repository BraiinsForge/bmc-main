import { Component } from 'react';
import { createPortal } from 'react-dom';

import cn from 'clsx';
import css from './Modal.scss';

import { Modal as BaseComponent, type ModalProps as BaseProps } from '@carbon/react';
import { PortalContext, type PortalContextValue } from '../PortalContext';

export interface ModalProps extends Omit<BaseProps, 'id'> {
    // Required because it's used for testing
    // and finding modal any other way is flaky
    id: string;
    portal?: boolean;
    cancelContentPadding?: boolean;
    cancelBodyOverflowShadow?: boolean;
}

export class Modal extends Component<ModalProps> {
    static contextType = PortalContext;
    declare context: PortalContextValue;

    #root: HTMLElement | ShadowRoot;
    #mount: HTMLElement;

    constructor(props: ModalProps) {
        super(props);

        // `carbon-components-react` don't use portal which causes z-index & overflow based issues.
        // Our mitigation is to inject portal API & allow disabling if such thing would be desired.
        const existing = document.body.querySelector<HTMLElement>('div[data-cy="modals-root"]');

        // If we already have a mount point, we'll just store the refference to it here
        if (existing) {
            this.#root = existing;
        }
        // Otherwise we'll create a new one and and append it to the detected root container
        else {
            this.#root = document.createElement('div');
            this.#root.setAttribute('data-cy', 'modals-root');
            document.body.appendChild(this.#root);
        }

        this.#mount = document.createElement('div');
        this.#mount.setAttribute('data-modal-mount', props.id);
    }
    componentDidMount() {
        this.#root.appendChild(this.#mount);
    }
    componentWillUnmount() {
        try {
            this.#root.removeChild(this.#mount);
        } catch (err: any) {
            console.error(err);
        }
    }

    render() {
        const { portal = true, cancelContentPadding, cancelBodyOverflowShadow, ...props } = this.props;
        const m = (
            <BaseComponent
                {...props}
                className={cn(
                    css.root,
                    cancelContentPadding && css.cancelContentPadding,
                    cancelBodyOverflowShadow && css.cancelBodyOverflowShadow,
                    props.className,
                    !props.onRequestClose && css.hideCloseButton,
                    this.context.disablePortal && css.inlineModal,
                )}
                data-cy={props.id}
            />
        );

        return portal && !this.context.disablePortal ? createPortal(m, this.#mount) : m;
    }
}
