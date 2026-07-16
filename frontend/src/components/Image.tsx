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
