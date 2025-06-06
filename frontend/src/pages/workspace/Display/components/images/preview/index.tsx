import type { ImgHTMLAttributes } from 'react';
import { assertUnreachable } from '@/lib/ts';
import * as pb from '@/proto';

import { ClockScenePreview } from './clock';
import { TickerScenePreview } from './ticker';

export interface ScenePreviewProps extends ImgHTMLAttributes<HTMLImageElement> {
    kind: pb.SceneKind;
    variant?: pb.SceneVariant;
}
export function ScenePreview(props: ScenePreviewProps) {
    const { kind, variant, ...rest } = props;

    switch (kind) {
        case pb.SceneKind.clock:
            return <ClockScenePreview {...rest} kind={variant as pb.SceneVariantClock} />;

        case pb.SceneKind.ticker:
            return <TickerScenePreview {...rest} kind={variant as pb.SceneVariantTicker} />;

        case pb.SceneKind.combined:
            return <img {...rest} src={require('./preview-combined.png')} alt="Preview Combined" />;

        case pb.SceneKind.image:
            return (
                <img
                    {...rest}
                    src={require('./preview-image.png')}
                    // biome-ignore lint/a11y/noRedundantAlt: Bullshit, this is talking about use picture
                    alt="Preview Image"
                />
            );

        case pb.SceneKind.pool:
            return <img {...rest} src={require('./preview-pool.png')} alt="Preview Pool" />;

        case pb.SceneKind.manager:
            return <img {...rest} src={require('./preview-manager.png')} alt="Preview Manager" />;

        default:
            assertUnreachable(kind, 'scene preview');
    }
}
