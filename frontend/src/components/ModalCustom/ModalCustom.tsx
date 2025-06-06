import { Component, createRef } from 'react';
import { createPortal } from 'react-dom';

import { ComposedModal, type ComposedModalProps, ModalHeader, ModalBody, ModalFooter } from '@carbon/react';
import { Loading } from '../Loading';

import cn from 'clsx';
import css from './ModalCustom.scss';

export interface CustomModalProps extends Omit<ComposedModalProps, 'title' | 'children' | 'open'> {
    title?: ReactNode;
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
        if (this.#ref.current?.handleClick) this.#ref.current.handleClick = this.#preventClose;
        this.#root.appendChild(this.#mount);
    }
    componentWillUnmount() {
        this.#root.removeChild(this.#mount);
    }
    componentDidUpdate(prevProps: CustomModalProps) {
        if (this.props.isInnerModal && prevProps.open) {
            document.body.classList.add('cds--body--with-modal-open');
        }
    }

    #ref = createRef<any>();
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
            children,
            footer,
            title,
            onSubmit,
            onClose,
            isLoading,
            open,
            hideHeader,
            id,
            portal,
            isInnerModal,
            bodyClassName,
            cancelBodyOverflowShadow,
            ...rest
        } = this.props;

        const m = (
            <ComposedModal
                {...rest}
                id={id}
                data-cy={id}
                ref={this.#ref}
                open={open}
                onSubmit={onSubmit}
                onClose={onClose ? this.#handleClose : undefined}
                onMouseDown={this.#handleMouseDown}
            >
                {!hideHeader && (title != null || typeof onClose === 'function') && (
                    <ModalHeader
                        className={cn(!onClose && css.hideModalCloseButton)}
                        {...this.#commonClickProps}
                        title={title}
                    />
                )}
                <ModalBody
                    {...this.#commonClickProps}
                    className={cn(css.body, cancelBodyOverflowShadow && css.cancelBodyOverflowShadow, bodyClassName)}
                    children={children}
                />

                {footer && <ModalFooter {...this.#commonClickProps} className={css.footer} children={footer} />}
                <Loading active={isLoading} cover />
            </ComposedModal>
        );

        return portal ? createPortal(m, this.#mount) : m;
    }
}
