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
