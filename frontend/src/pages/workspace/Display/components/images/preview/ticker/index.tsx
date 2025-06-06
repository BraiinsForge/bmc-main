import type { ImgHTMLAttributes } from 'react';
import { assertUnreachable } from '@/lib/ts';
import * as pb from '@/proto';

export interface TickerScenePreviewProps extends ImgHTMLAttributes<HTMLImageElement> {
    kind: pb.SceneVariantTicker;
}

export function TickerScenePreview(props: TickerScenePreviewProps) {
    const { kind, ...rest } = props;

    switch (kind) {
        case pb.SceneVariantTicker.line:
            return <img {...rest} src={require('./ticker-line.png')} alt="Line ticker" />;

        case pb.SceneVariantTicker.list:
            return <img {...rest} src={require('./ticker-list.png')} alt="List ticket" />;

        case pb.SceneVariantTicker.candle:
            return <img {...rest} src={require('./ticker-candle.png')} alt="Candlestick ticker" />;

        default:
            assertUnreachable(kind, 'ticker scene preview');
    }
}
