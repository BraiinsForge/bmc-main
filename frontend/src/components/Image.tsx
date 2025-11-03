import { type ImgHTMLAttributes, type Ref, useState, useCallback } from 'react';

type HtmlImageProps = ImgHTMLAttributes<HTMLImageElement>;
type RenderImage = (ref?: Ref<null | HTMLImageElement>, props?: HtmlImageProps) => ReactElement;

interface OwnProps {
    src: null | string;
    fallback?: string;
    alt: string;

    render?(img: RenderImage, failed: boolean): ReactNode;
}
interface State {
    effectiveSource: Maybe<string>;
    hasFailed: boolean;
}

export interface ImageProps extends Omit<HtmlImageProps, keyof OwnProps>, OwnProps {}
export function Image(props: ImageProps) {
    const { src, fallback, render, alt, ...rest } = props;
    const [{ effectiveSource, hasFailed }, setState] = useState<State>({ effectiveSource: src, hasFailed: false });

    const handleFail = useCallback(() => setState({ effectiveSource: fallback, hasFailed: true }), [fallback]);
    const renderImage: RenderImage = useCallback(
        (ref, extraProps) => {
            return (
                <img
                    {...rest}
                    {...extraProps}
                    ref={ref}
                    alt={alt}
                    src={effectiveSource ?? undefined}
                    onError={hasFailed ? undefined : handleFail}
                />
            );
        },
        // biome-ignore lint/correctness/useExhaustiveDependencies: Couldn't find a better way around the "rest" prop
        [effectiveSource, rest, alt, hasFailed, handleFail],
    );

    return typeof render === 'function' ? render(renderImage, hasFailed || !src) : renderImage();
}
