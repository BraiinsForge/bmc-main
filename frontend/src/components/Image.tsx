import { useState, useCallback } from 'react';
import type { ImgHTMLAttributes, Ref } from 'react';

type HtmlImageProps = ImgHTMLAttributes<HTMLImageElement>;
type RenderImage = (ref?: Ref<null | HTMLImageElement>, props?: HtmlImageProps) => ReactElement;

interface OwnProps {
    src: null | string;
    fallback?: string;
    alt: string;

    render?(img: RenderImage, failed: boolean): ReactNode;
}
interface State {
    src: null | string;
    effectiveSource: Maybe<string>;
    hasFailed: boolean;
}

export interface ImageProps extends Omit<HtmlImageProps, keyof OwnProps>, OwnProps {}
export function Image(props: ImageProps) {
    const { src, fallback, render, alt, ...rest } = props;
    const [state, setState] = useState<State>({ src, effectiveSource: src, hasFailed: false });

    // Reset the derived state when the src prop changes — otherwise a prior load
    // failure stays pinned to the fallback and a new src never gets a chance.
    if (src !== state.src) setState({ src, effectiveSource: src, hasFailed: false });
    const { effectiveSource, hasFailed } = state;

    const handleFail = useCallback(
        () => setState({ src, effectiveSource: fallback, hasFailed: true }),
        [src, fallback],
    );
    const renderImage: RenderImage = (ref, extraProps) => (
        <img
            {...rest}
            {...extraProps}
            ref={ref}
            alt={alt}
            src={effectiveSource ?? undefined}
            onError={hasFailed ? undefined : handleFail}
        />
    );

    return typeof render === 'function' ? render(renderImage, hasFailed || !src) : renderImage();
}
