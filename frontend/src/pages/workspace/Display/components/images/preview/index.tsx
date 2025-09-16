import type { ImgHTMLAttributes } from 'react';
import { assertUnreachable } from '@/lib/ts';
import type * as pb from '@/proto';

// import { ClockScenePreview } from './clock';
// import { TickerScenePreview } from './ticker';
import * as Icons from '@/components/images/icons';

export interface ScenePreviewProps extends ImgHTMLAttributes<HTMLImageElement> {
    kind: Maybe<pb.WidgetKind['value'] | 'combined'>;
}
export function ScenePreview(props: ScenePreviewProps) {
    const { kind /* ...rest */ } = props;
    if (kind == null) return null;

    // Combined scene
    // if (kind === 'combined') return <img {...rest} src={require('./preview-combined.png')} alt="Preview Combined" />;
    if (kind === 'combined') return <Icons.WidgetCombined size={40} />;

    switch (kind.case) {
        case undefined:
            return null;

        case 'clock':
            // return <ClockScenePreview {...rest} kind={kind.value.clockStyle} />;
            return <Icons.WidgetClocks size={40} />;

        case 'tickerBtc':
            // return <TickerScenePreview {...rest} />;
            return <Icons.WidgetTicker size={40} />;

        case 'blockHeight':
            // return <img {...rest} src={require('./preview-block-height.png')} alt="Preview Block Height" />;
            return <Icons.WidgetBlockHeight size={40} />;

        case 'braiinsPool':
            // return <img {...rest} src={require('./preview-pool.png')} alt="Preview for Braiins Pool" />;
            return <Icons.WidgetPool size={40} />;

        // case 'image':
        //     return (
        //         <img
        //             {...rest}
        //             src={require('./preview-image.png')}
        //             // biome-ignore lint/a11y/noRedundantAlt: Bullshit, this is talking about use picture
        //             alt="Preview Image"
        //         />
        //     );

        // case 'manager':
        //     return <img {...rest} src={require('./preview-manager.png')} alt="Preview Manager" />;

        default:
            assertUnreachable(kind, 'scene preview');
    }
}
