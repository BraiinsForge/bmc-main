import type { ImgHTMLAttributes } from 'react';
import { assertUnreachable } from '@/lib/ts';
import type * as pb from '@/proto';

import { ClockScenePreview } from './clock';
import { TickerScenePreview } from './ticker';

export interface ScenePreviewProps extends ImgHTMLAttributes<HTMLImageElement> {
    kind: Maybe<pb.WidgetKind['value'] | 'combined'>;
}
export function ScenePreview(props: ScenePreviewProps) {
    const { kind, ...rest } = props;
    if (kind == null) return null;

    // Combined scene
    if (kind === 'combined') return <img {...rest} src={require('./preview-combined.png')} alt="Preview Combined" />;

    switch (kind.case) {
        case undefined:
            return null;

        case 'clock':
            return <ClockScenePreview {...rest} kind={kind.value.clockStyle} />;

        case 'tickerBtc':
            return <TickerScenePreview {...rest} />;

        case 'blockHeight':
            return <img {...rest} src={require('./preview-block-height.png')} alt="Preview Block Height" />;

        // case 'image':
        //     return (
        //         <img
        //             {...rest}
        //             src={require('./preview-image.png')}
        //             // biome-ignore lint/a11y/noRedundantAlt: Bullshit, this is talking about use picture
        //             alt="Preview Image"
        //         />
        //     );

        // case 'pool':
        //     return <img {...rest} src={require('./preview-pool.png')} alt="Preview Pool"/>;

        // case 'manager':
        //     return <img {...rest} src={require('./preview-manager.png')} alt="Preview Manager" />;

        default:
            assertUnreachable(kind, 'scene preview');
    }
}
